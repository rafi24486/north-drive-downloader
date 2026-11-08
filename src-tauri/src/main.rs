#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet, VecDeque}, fs, io, path::{Component, Path, PathBuf}, process::Command, sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex}, time::{Duration, Instant}};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{fs as async_fs, io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener, time::sleep};

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const SHORTCUT_MIME: &str = "application/vnd.google-apps.shortcut";
const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3/files";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
const KEYCHAIN_SERVICE: &str = "com.northdrive.downloader.oauth";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    api_key: String,
    client_id: String,
    save_directory: String,
    max_parallel: usize,
}
impl Default for Preferences {
    fn default() -> Self {
        Self { api_key: String::new(), client_id: String::new(), save_directory: String::new(), max_parallel: 4 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadItem {
    id: String,
    name: String,
    relative_path: String,
    size: Option<u64>,
    mime_type: String,
    md5_checksum: Option<String>,
    resource_key: Option<String>,
    export_mime: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FolderPlan {
    folder_id: String,
    folder_name: String,
    files: Vec<DownloadItem>,
    empty_folders: Vec<String>,
    total_bytes: u64,
    warnings: Vec<String>,
    access_method: String,
    folder_resource_key: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FileProgress {
    id: String,
    relative_path: String,
    bytes: u64,
    total: Option<u64>,
    status: &'static str,
    speed: f64,
    message: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RunSummary { completed: usize, failed: usize, cancelled: bool, errors: Vec<String> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadSession {
    plan: FolderPlan,
    directory: String,
    #[serde(default)]
    auto_resume: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryView {
    plan: FolderPlan,
    directory: String,
    partial_bytes: HashMap<String, u64>,
    auto_resume: bool,
}

#[derive(Default)]
struct RunControl { paused: AtomicBool, cancelled: AtomicBool }
struct AppState { prefs: Mutex<Preferences>, run: Mutex<Option<Arc<RunControl>>>, client: Client }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFile {
    id: String, name: String,
    #[serde(default)] mime_type: String,
    size: Option<String>,
    md5_checksum: Option<String>,
    resource_key: Option<String>,
    capabilities: Option<Capabilities>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities { can_download: Option<bool> }
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilePage { #[serde(default)] files: Vec<DriveFile>, next_page_token: Option<String> }
#[derive(Deserialize)]
struct OAuthToken { access_token: String, refresh_token: Option<String> }

#[derive(Clone)]
enum Access { Public(String), OAuth(String) }
impl Access {
    fn method(&self) -> &'static str { match self { Access::Public(_) => "public", Access::OAuth(_) => "oauth" } }
    fn request(&self, client: &Client, url: &str) -> reqwest::RequestBuilder {
        match self {
            Access::Public(key) => client.get(url).query(&[("key", key.as_str())]),
            Access::OAuth(token) => client.get(url).bearer_auth(token),
        }
    }
}

type AuthCache = Arc<tokio::sync::Mutex<(Instant, Access)>>;

async fn current_access(cache: &AuthCache, client: &Client, prefs: &Preferences) -> Result<Access, String> {
    let mut stored = cache.lock().await;
    if matches!(stored.1, Access::OAuth(_)) && stored.0.elapsed() > Duration::from_secs(45 * 60) {
        stored.1 = Access::OAuth(refresh_access_token(client, prefs).await?);
        stored.0 = Instant::now();
    }
    Ok(stored.1.clone())
}

async fn expire_access(cache: &AuthCache) {
    let mut stored = cache.lock().await;
    if matches!(stored.1, Access::OAuth(_)) {
        stored.0 = Instant::now() - Duration::from_secs(3600);
    }
}

fn config_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("preferences.json"))
}
fn persist_prefs(app: &AppHandle, prefs: &Preferences) -> Result<(), String> {
    let path = config_file(app)?;
    let data = serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}
fn read_prefs(app: &AppHandle) -> Preferences {
    config_file(app).ok().and_then(|p| fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
fn session_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_file(app)?.with_file_name("last-download.json"))
}
fn write_session(app: &AppHandle, session: &DownloadSession) -> Result<(), String> {
    let path = session_file(app)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(session).map_err(|e|e.to_string())?)
        .map_err(|e|e.to_string())?;
    fs::rename(temporary, path).map_err(|e|e.to_string())
}
#[tauri::command]
fn recover_session(app: AppHandle) -> Result<Option<RecoveryView>, String> {
    let path = session_file(&app)?;
    if !path.exists() { return Ok(None); }
    let session: DownloadSession = serde_json::from_slice(&fs::read(path).map_err(|e|e.to_string())?)
        .map_err(|e|format!("Cannot restore previous download: {e}"))?;
    if !Path::new(&session.directory).is_absolute() { return Err("Invalid saved destination".into()); }
    let root = Path::new(&session.directory).join(sanitized_name(&session.plan.folder_name));
    let mut partial_bytes = HashMap::new();
    for item in &session.plan.files {
        if let Ok(relative) = safe_relative(&item.relative_path) {
            let destination = root.join(relative);
            let part = part_paths(&destination).0;
            let bytes = fs::metadata(&destination).or_else(|_| fs::metadata(&part))
                .map(|m|m.len()).unwrap_or(0);
            if bytes > 0 { partial_bytes.insert(item.id.clone(), bytes); }
        }
    }
    Ok(Some(RecoveryView {plan:session.plan, directory:session.directory, partial_bytes, auto_resume:session.auto_resume}))
}
fn keychain_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, "refresh-token").map_err(|e| e.to_string())
}
fn has_login() -> bool { keychain_entry().and_then(|e| e.get_password().map_err(|x| x.to_string())).is_ok() }

#[tauri::command]
fn get_preferences(state: State<'_, Arc<AppState>>) -> Preferences { state.prefs.lock().unwrap().clone() }
#[tauri::command]
fn is_connected(state: State<'_, Arc<AppState>>) -> bool { has_login() && !state.prefs.lock().unwrap().client_id.is_empty() }
#[tauri::command]
fn set_preferences(app: AppHandle, state: State<'_, Arc<AppState>>, preferences: Preferences) -> Result<(), String> {
    if preferences.max_parallel == 0 || preferences.max_parallel > 8 { return Err("Parallel downloads must be between 1 and 8".into()); }
    let preferences = Preferences { api_key: preferences.api_key.trim().to_string(), client_id: preferences.client_id.trim().to_string(), save_directory: preferences.save_directory.trim().to_string(), max_parallel: preferences.max_parallel };
    persist_prefs(&app, &preferences)?;
    let previous_client = state.prefs.lock().unwrap().client_id.clone();
    if previous_client != preferences.client_id {
        // Authorization belongs to a particular user-provided OAuth client ID.
        if let Ok(entry) = keychain_entry() { let _ = entry.delete_credential(); }
    }
    *state.prefs.lock().unwrap() = preferences;
    Ok(())
}
#[tauri::command]
fn disconnect() -> Result<(), String> {
    match keychain_entry()?.delete_credential() { Ok(_) => Ok(()), Err(e) if e.to_string().contains("No matching") => Ok(()), Err(e) => Err(e.to_string()) }
}

fn new_verifier() -> (String, String, String) {
    let mut raw = [0u8; 32]; rand::rngs::OsRng.fill_bytes(&mut raw);
    let verifier = URL_SAFE_NO_PAD.encode(raw);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    rand::rngs::OsRng.fill_bytes(&mut raw);
    (verifier, challenge, URL_SAFE_NO_PAD.encode(raw))
}

#[tauri::command]
async fn connect_google(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let client_id = state.prefs.lock().unwrap().client_id.clone();
    if client_id.is_empty() { return Err("Add your Google OAuth Desktop Client ID in Settings first".into()); }
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let (verifier, challenge, state_code) = new_verifier();
    let mut authorize = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
    authorize.query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state_code);
    Command::new("open").arg(authorize.as_str()).spawn().map_err(|e| format!("Cannot open browser: {e}"))?;
    let code = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let (mut stream, addr) = listener.accept().await.map_err(|e| e.to_string())?;
            if !addr.ip().is_loopback() { continue; }
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
            let req = String::from_utf8_lossy(&buf[..n]);
            let target = req.lines().next().and_then(|s| s.split_whitespace().nth(1)).ok_or("Invalid callback")?;
            let cb = url::Url::parse(&format!("http://127.0.0.1:{port}{target}")).map_err(|e| e.to_string())?;
            if cb.path() != "/callback" { continue; }
            let params: HashMap<String,String> = cb.query_pairs().into_owned().collect();
            let valid = params.get("state") == Some(&state_code);
            let success = valid && params.contains_key("code");
            let message = if success { "Google connected. You can close this tab and return to North Drive Downloader." } else { "Authorization failed. Return to North Drive Downloader." };
            let body = format!("<html><body style='font-family:-apple-system;background:#0c0912;color:#fff;text-align:center;padding:100px'><h2>{message}</h2></body></html>");
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = stream.write_all(response.as_bytes()).await;
            if !valid { return Err("OAuth state mismatch".into()); }
            if let Some(error) = params.get("error") { return Err(format!("Google authorization denied: {error}")); }
            return params.get("code").cloned().ok_or_else(|| "Missing authorization code".to_string());
        }
    }).await.map_err(|_| "Google sign-in timed out".to_string())??;
    let response = state.client.post("https://oauth2.googleapis.com/token").form(&[
        ("code", code.as_str()), ("client_id", client_id.as_str()), ("code_verifier", verifier.as_str()),
        ("redirect_uri", redirect_uri.as_str()), ("grant_type", "authorization_code")
    ]).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() { return Err(format!("Google sign-in failed: {}", response.text().await.unwrap_or_default())); }
    let token: OAuthToken = response.json().await.map_err(|e| e.to_string())?;
    let refresh = token.refresh_token.ok_or("Google did not return a refresh token; revoke access and retry")?;
    keychain_entry()?.set_password(&refresh).map_err(|e| e.to_string())?;
    Ok("Google account connected".into())
}

async fn refresh_access_token(client: &Client, prefs: &Preferences) -> Result<String, String> {
    if prefs.client_id.is_empty() { return Err("OAuth Client ID not configured".into()); }
    let refresh = keychain_entry()?.get_password().map_err(|_| "Private folder needs authorization in Settings".to_string())?;
    let response = client.post("https://oauth2.googleapis.com/token").form(&[
        ("client_id", prefs.client_id.as_str()), ("refresh_token", refresh.as_str()), ("grant_type", "refresh_token")
    ]).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() { return Err("Google session expired or was revoked. Reconnect your account".into()); }
    let token: OAuthToken = response.json().await.map_err(|e| e.to_string())?;
    Ok(token.access_token)
}

fn parse_drive_link(link: &str) -> Result<(String, Option<String>), String> {
    let url = url::Url::parse(link.trim()).map_err(|_| "Paste a complete Google Drive folder URL".to_string())?;
    let domain = url.host_str().unwrap_or("");
    if domain != "drive.google.com" && domain != "docs.google.com" { return Err("Only Google Drive URLs are supported".into()); }
    let pieces: Vec<_> = url.path_segments().map(|s| s.collect()).unwrap_or_else(Vec::new);
    let id: Option<String> = if let Some(i) = pieces.iter().position(|p| *p == "folders") {
        pieces.get(i + 1).map(|value| (*value).to_string())
    } else {
        url.query_pairs().find(|(k, _)| k == "id").map(|(_, v)| v.into_owned())
    };
    let id = id.ok_or("This is not a Drive folder link. Use Share → Copy link on the folder")?;
    if !valid_id(&id) { return Err("Invalid Drive folder identifier".into()); }
    let rk = url.query_pairs().find(|(k,_)| k == "resourcekey").map(|(_,v)| v.into_owned());
    Ok((id, rk))
}
fn valid_id(s: &str) -> bool { s.len() >= 8 && s.len() <= 200 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) }
fn sanitized_name(raw: &str) -> String {
    let clean: String = raw.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if clean.is_empty() || clean == "." || clean == ".." { "Unnamed".into() } else { clean.chars().take(220).collect() }
}
fn unique_name(name: &str, used: &mut HashSet<String>) -> String {
    let key = name.to_lowercase();
    if used.insert(key) { return name.into(); }
    let (stem, ext) = match name.rsplit_once('.') { Some((s,e)) if !s.is_empty() => (s, format!(".{e}")), _ => (name, String::new()) };
    for n in 2..100000 {
        let candidate = format!("{stem} ({n}){ext}");
        if used.insert(candidate.to_lowercase()) { return candidate; }
    }
    format!("{}-duplicate", name)
}


fn response_issue(status: StatusCode, raw: &str) -> String {
    let body: serde_json::Value = serde_json::from_str(raw).unwrap_or_default();
    let reason = body.pointer("/error/errors/0/reason").and_then(|v|v.as_str()).unwrap_or("");
    let detail = body.pointer("/error/message").and_then(|v|v.as_str())
        .unwrap_or(if raw.trim_start().starts_with('<') { "Google returned an HTML error page; possible download quota or access restriction" } else { "Request rejected by Google Drive" });
    let message = format!("Google Drive HTTP {}{}: {}", status.as_u16(),
        if reason.is_empty() { String::new() } else { format!(" ({reason})") }, detail.chars().take(240).collect::<String>());
    let permanent_reason = matches!(reason, "insufficientFilePermissions" | "fileNotDownloadable"
        | "domainPolicy" | "appNotAuthorizedToFile" | "keyInvalid" | "accessNotConfigured"
        | "forbidden" | "notFound" | "cannotDownloadFile" | "storageQuotaExceeded");
    if status == StatusCode::FORBIDDEN && !permanent_reason { format!("QUOTA: {message}") }
    else if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() { format!("RETRY: {message}") }
    else { format!("PERMANENT: {message}") }
}

fn visible_error(message: &str) -> &str {
    message.strip_prefix("QUOTA: ").or_else(|| message.strip_prefix("RETRY: "))
        .or_else(|| message.strip_prefix("PERMANENT: ")).unwrap_or(message)
}

async fn json_request<T: for<'de> Deserialize<'de>>(client: &Client, access: &Access, url: &str, params: &[(&str, String)], keys: &HashMap<String, String>) -> Result<T, String> {
    for attempt in 0u32..8 {
        let mut req = access.request(client, url).query(params);
        if !keys.is_empty() {
            let header_value = keys.iter().map(|(k,v)| format!("{k}/{v}")).collect::<Vec<_>>().join(",");
            req = req.header("X-Goog-Drive-Resource-Keys", header_value);
        }
        let response = match tokio::time::timeout(Duration::from_secs(90), req.send()).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) | Err(_) if attempt < 7 => {
                sleep(Duration::from_secs((1u64 << attempt.min(5)).min(32))).await;
                continue;
            },
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => return Err("Google Drive request timed out".into())
        };
        if response.status().is_success() {
            return response.json::<T>().await.map_err(|e| format!("Unexpected Google Drive response: {e}"));
        }
        let status = response.status();
        let issue = response_issue(status, &response.text().await.unwrap_or_default());
        if attempt < 7 && !issue.starts_with("PERMANENT: ") {
            sleep(Duration::from_secs((1u64 << attempt.min(5)).min(32) + u64::from(rand::random::<u8>() % 3))).await;
            continue;
        }
        return Err(visible_error(&issue).to_string());
    }
    Err("Drive request retries exhausted".into())
}

async fn metadata(client: &Client, access: &Access, id: &str, keys: &HashMap<String,String>) -> Result<DriveFile, String> {
    json_request(client, access, &format!("{DRIVE_BASE}/{id}"), &[
        ("fields", "id,name,mimeType,size,md5Checksum,resourceKey,capabilities(canDownload)".into()),
        ("supportsAllDrives", "true".into())
    ], keys).await
}
async fn resolve_access(client: &Client, prefs: &Preferences, id: &str, rk: &Option<String>) -> Result<(Access, DriveFile), String> {
    let mut keys = HashMap::new(); if let Some(k) = rk { keys.insert(id.to_string(), k.clone()); }
    let mut public_err = String::new();
    if !prefs.api_key.is_empty() {
        let public = Access::Public(prefs.api_key.clone());
        match metadata(client, &public, id, &keys).await {
            Ok(meta) => return Ok((public,meta)), Err(err) => public_err = err
        }
    }
    if has_login() {
        let oauth = Access::OAuth(refresh_access_token(client, prefs).await?);
        let meta = metadata(client, &oauth, id, &keys).await?;
        return Ok((oauth, meta));
    }
    Err(if prefs.api_key.is_empty() {
        "Add your Google Drive API key in Settings; private folders also need an authorized OAuth Desktop Client ID".into()
    } else { format!("Public access failed: {public_err}. If this is restricted, connect an authorized Google account") })
}
fn workspace_export(mime: &str) -> Option<(&'static str, &'static str)> {
    match mime {
        "application/vnd.google-apps.document" => Some(("application/vnd.openxmlformats-officedocument.wordprocessingml.document", ".docx")),
        "application/vnd.google-apps.spreadsheet" => Some(("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", ".xlsx")),
        "application/vnd.google-apps.presentation" => Some(("application/vnd.openxmlformats-officedocument.presentationml.presentation", ".pptx")),
        "application/vnd.google-apps.drawing" => Some(("application/pdf", ".pdf")),
        _ => None
    }
}

#[tauri::command]
async fn scan_folder(link: String, state: State<'_, Arc<AppState>>) -> Result<FolderPlan, String> {
    let prefs = state.prefs.lock().unwrap().clone();
    let (id, rk) = parse_drive_link(&link)?;
    let client = &state.client;
    let (access, root) = resolve_access(client, &prefs, &id, &rk).await?;
    if root.mime_type != FOLDER_MIME { return Err("The shared link is not a folder".into()); }
    let mut queue: VecDeque<(String, String)> = VecDeque::from([(id.clone(), String::new())]);
    let mut keys = HashMap::new();
    if let Some(k) = rk.clone().or(root.resource_key.clone()) { keys.insert(id.clone(), k); }
    let mut seen_folders: HashSet<String> = std::iter::once(id.clone()).collect();
    let mut files = Vec::new(); let mut empty_folders = Vec::new(); let mut warnings = Vec::new();
    let mut total_bytes = 0u64;
    while let Some((folder_id, parent_path)) = queue.pop_front() {
        let mut found_children = false;
        let mut page: Option<String> = None;
        let mut occupied = HashSet::new();
        loop {
            let mut params = vec![
                ("q", format!("'{}' in parents and trashed = false", folder_id)),
                ("pageSize", "1000".into()),
                ("fields", "nextPageToken,files(id,name,mimeType,size,md5Checksum,resourceKey,capabilities(canDownload))".into()),
                ("supportsAllDrives", "true".into()),
                ("includeItemsFromAllDrives", "true".into()),
            ];
            if let Some(ref token) = page { params.push(("pageToken", token.clone())); }
            let mut request_keys = HashMap::new();
            if let Some(k) = keys.get(&id) { request_keys.insert(id.clone(), k.clone()); }
            if let Some(k) = keys.get(&folder_id) { request_keys.insert(folder_id.clone(), k.clone()); }
            let result: FilePage = json_request(client, &access, DRIVE_BASE, &params, &request_keys).await?;
            for entry in result.files {
                found_children = true;
                if let Some(k) = &entry.resource_key { keys.insert(entry.id.clone(), k.clone()); }
                let export = workspace_export(&entry.mime_type);
                let base_name = sanitized_name(&entry.name);
                let filename = if let Some((_,suffix)) = export { format!("{base_name}{suffix}") } else { base_name };
                let final_name = unique_name(&filename, &mut occupied);
                let relative_path = if parent_path.is_empty() { final_name.clone() } else { format!("{parent_path}/{final_name}") };
                if entry.mime_type == FOLDER_MIME {
                    if seen_folders.insert(entry.id.clone()) { queue.push_back((entry.id, relative_path)); }
                    continue;
                }
                if entry.mime_type == SHORTCUT_MIME {
                    warnings.push(format!("Shortcut not copied: {relative_path}"));
                    continue;
                }
                if entry.capabilities.as_ref().and_then(|x| x.can_download) == Some(false) {
                    warnings.push(format!("Download disabled by owner: {relative_path}"));
                    continue;
                }
                if entry.mime_type.starts_with("application/vnd.google-apps.") && export.is_none() {
                    warnings.push(format!("Unsupported Google Workspace file: {relative_path}"));
                    continue;
                }
                let file_path = relative_path;
                let size = entry.size.and_then(|v| v.parse::<u64>().ok());
                total_bytes = total_bytes.saturating_add(size.unwrap_or_default());
                files.push(DownloadItem { id: entry.id, name: final_name, relative_path: file_path, size, mime_type: entry.mime_type, md5_checksum: entry.md5_checksum, resource_key: entry.resource_key, export_mime: export.map(|(m,_)| m.into()) });
            }
            page = result.next_page_token;
            if page.is_none() { break; }
        }
        if !found_children && !parent_path.is_empty() { empty_folders.push(parent_path); }
    }
    Ok(FolderPlan { folder_id: id, folder_name: sanitized_name(&root.name), files, empty_folders, total_bytes, warnings, access_method: access.method().into(), folder_resource_key: rk.or(root.resource_key) })
}

fn safe_relative(raw: &str) -> Result<PathBuf, String> {
    let input = Path::new(raw);
    if input.is_absolute() { return Err("Absolute file path rejected".into()); }
    let mut result = PathBuf::new();
    for part in input.components() {
        match part {
            Component::Normal(s) => {
                let component = s.to_string_lossy();
                if component.is_empty() || component == "." || component == ".." { return Err("Invalid path component".into()); }
                result.push(s);
            },
            _ => return Err("Unsafe relative path rejected".into())
        }
    }
    if result.as_os_str().is_empty() { return Err("Empty file path".into()); }
    Ok(result)
}
fn progress(app: &AppHandle, file: &DownloadItem, bytes: u64, status: &'static str, speed: f64, message: Option<String>) {
    let _ = app.emit("north:progress", FileProgress { id: file.id.clone(), relative_path: file.relative_path.clone(), bytes, total: file.size, status, speed, message });
}

async fn verify_md5(path: &Path, expected: &str) -> Result<bool, String> {
    let path = path.to_owned();
    let expected = expected.to_owned();
    tokio::task::spawn_blocking(move || -> Result<bool, String> {
        use std::io::Read;
        let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut md5 = md5::Context::new();
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 { break; }
            md5.consume(&buf[..n]);
        }
        Ok(format!("{:x}", md5.compute()).eq_ignore_ascii_case(&expected))
    }).await.map_err(|e| format!("Checksum worker stopped: {e}"))?
}

fn part_paths(destination: &Path) -> (PathBuf, PathBuf) {
    let name = destination.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "unnamed".to_string());
    let part = destination.with_file_name(format!("{name}.northpart"));
    let meta = destination.with_file_name(format!("{name}.northpart.json"));
    (part, meta)
}
async fn wait_for_run(control: &RunControl) -> Result<(), String> {
    loop {
        if control.cancelled.load(Ordering::SeqCst) { return Err("Cancelled".into()); }
        if !control.paused.load(Ordering::SeqCst) { return Ok(()); }
        sleep(Duration::from_millis(200)).await;
    }
}

async fn retry_wait(control: &RunControl, seconds: u64) -> Result<(), String> {
    for _ in 0..seconds {
        wait_for_run(control).await?;
        sleep(Duration::from_secs(1)).await;
    }
    Ok(())
}

async fn download_one(app: &AppHandle, client: &Client, prefs: &Preferences, auth_cache: &AuthCache, file: &DownloadItem, root: &Path, control: &RunControl) -> Result<(), String> {
    let relative = safe_relative(&file.relative_path)?;
    let target = root.join(relative);
    if let Some(parent) = target.parent() { async_fs::create_dir_all(parent).await.map_err(|e| e.to_string())?; }
    // Never replace a completed file without checking its source checksum.
    if target.exists() {
        if let (Some(checksum), Some(size)) = (&file.md5_checksum, file.size) {
            if fs::metadata(&target).ok().map(|m| m.len()) == Some(size) && checksum.len() == 32
                && verify_md5(&target, checksum).await? {
                progress(app, file, size, "completed", 0.0, Some("Previously verified".into()));
                return Ok(());
            }
        }
        return Err(format!("Existing destination file differs or cannot be verified: {}", target.display()));
    }
    let (part, meta) = part_paths(&target);
    let fingerprint = serde_json::json!({"id":file.id,"size":file.size,"md5":file.md5_checksum,"mime":file.mime_type,"export":file.export_mime});
    let expected = serde_json::to_vec(&fingerprint).map_err(|e| e.to_string())?;
    // A stale or missing sidecar is never appended to a new file. Reset it safely, without user intervention.
    if part.exists() && fs::read(&meta).ok().as_deref() != Some(expected.as_slice()) {
        async_fs::remove_file(&part).await.map_err(|e| format!("Cannot reset stale partial: {e}"))?;
    }
    if !part.exists() || fs::read(&meta).ok().as_deref() != Some(expected.as_slice()) {
        async_fs::write(&meta, &expected).await.map_err(|e| e.to_string())?;
    }
    if file.size.is_some_and(|size| fs::metadata(&part).ok().is_some_and(|m|m.len() > size)) {
        async_fs::remove_file(&part).await.map_err(|e| e.to_string())?;
    }

    let mut attempts = 0u32;
    let mut checksum_reset = false;
    loop {
        wait_for_run(control).await?;
        let result = download_attempt(app, client, prefs, auth_cache, file, &part, control).await;
        match result {
            Ok(()) => {
                let len = fs::metadata(&part).map_err(|e| e.to_string())?.len();
                if let Some(size) = file.size {
                    if len != size {
                        if len > size {
                            async_fs::remove_file(&part).await.map_err(|e| e.to_string())?;
                        }
                        attempts += 1;
                        if attempts >= 288 { return Err(format!("Size mismatch after retrying: {len} / {size} bytes")); }
                        progress(app, file, len.min(size), "retrying", 0.0, Some("Recovering incomplete transfer".into()));
                        retry_wait(control, 3).await?;
                        continue;
                    }
                }
                if let Some(checksum) = &file.md5_checksum {
                    progress(app, file, len, "verifying", 0.0, None);
                    if !verify_md5(&part, checksum).await? {
                        if checksum_reset { return Err("MD5 checksum mismatch after a clean retry".into()); }
                        checksum_reset = true;
                        async_fs::remove_file(&part).await.map_err(|e| e.to_string())?;
                        progress(app, file, 0, "retrying", 0.0, Some("Checksum mismatch; restarting this file safely".into()));
                        continue;
                    }
                }
                break;
            }
            Err(issue) => {
                if control.cancelled.load(Ordering::SeqCst) { return Err("Cancelled".into()); }
                if let Some(reason) = issue.strip_prefix("PERMANENT: ") { return Err(reason.to_string()); }
                if issue.starts_with("RESET_PARTIAL: ") {
                    async_fs::remove_file(&part).await.map_err(|e| e.to_string())?;
                    progress(app, file, 0, "retrying", 0.0, Some("Rebuilding a stale partial file".into()));
                    continue;
                }
                attempts += 1;
                // Allow long quota windows to recover; preserve every byte across transient errors.
                if attempts >= 288 { return Err(format!("{} (retry limit reached)", visible_error(&issue))); }
                let is_quota = issue.starts_with("QUOTA: ");
                let exponential = 1u64 << attempts.min(7);
                let seconds = if is_quota { (exponential * 5).clamp(30, 300) } else { exponential.clamp(2, 90) }
                    + u64::from(rand::random::<u8>() % 5);
                let bytes = fs::metadata(&part).map(|m|m.len()).unwrap_or(0);
                progress(app, file, bytes, "retrying", 0.0, Some(format!("{} — retrying in {}s (attempt {})", visible_error(&issue), seconds, attempts)));
                retry_wait(control, seconds).await?;
            }
        }
    }
    let length = fs::metadata(&part).map_err(|e| e.to_string())?.len();
    if target.exists() { return Err(format!("Destination appeared during transfer: {}", target.display())); }
    async_fs::rename(&part, &target).await.map_err(|e| e.to_string())?;
    let _ = async_fs::remove_file(meta).await;
    progress(app, file, length, "completed", 0.0, None);
    Ok(())
}

async fn download_attempt(app: &AppHandle, client: &Client, prefs: &Preferences, auth_cache: &AuthCache, file: &DownloadItem, path: &Path, control: &RunControl) -> Result<(), String> {
    let mut offset = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    // Workspace exports cannot resume a range: restart the export without touching other files.
    if file.export_mime.is_some() && offset > 0 {
        async_fs::remove_file(path).await.map_err(|e| e.to_string())?;
        offset = 0;
    }
    if file.size == Some(offset) && offset > 0 { return Ok(()); }
    let endpoint = if file.export_mime.is_some() {
        format!("{DRIVE_BASE}/{}/export", file.id)
    } else { format!("{DRIVE_BASE}/{}", file.id) };
    let auth = current_access(auth_cache, client, prefs).await?;
    let mut req = auth.request(client, &endpoint);
    if let Some(mime) = &file.export_mime { req = req.query(&[("mimeType",mime.as_str())]); }
    else { req = req.query(&[("alt","media"), ("supportsAllDrives", "true")]); }
    if let Some(key) = &file.resource_key { req = req.header("X-Goog-Drive-Resource-Keys",format!("{}/{}",file.id,key)); }
    if offset > 0 { req = req.header(header::RANGE, format!("bytes={offset}-")); }
    let mut response = tokio::time::timeout(Duration::from_secs(90),req.send()).await
        .map_err(|_| "RETRY: Timed out waiting for Google Drive response".to_string())?
        .map_err(|e|format!("RETRY: Network error: {e}"))?;
    if response.status() == StatusCode::UNAUTHORIZED && matches!(auth, Access::OAuth(_)) {
        expire_access(auth_cache).await;
        return Err("RETRY: Google OAuth token expired; refreshing".into());
    }
    if offset > 0 {
        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
            return Err("RESET_PARTIAL: Existing range no longer matches source".into());
        }
        if response.status() != StatusCode::PARTIAL_CONTENT {
            if response.status().is_success() {
                return Err("RESET_PARTIAL: Server ignored the range; safely restarting file".into());
            }
            let status = response.status();
            return Err(response_issue(status, &response.text().await.unwrap_or_default()));
        }
        let range = response.headers().get(header::CONTENT_RANGE).and_then(|v|v.to_str().ok()).unwrap_or("");
        if !range.starts_with(&format!("bytes {offset}-")) {
            return Err("RESET_PARTIAL: Server returned an unexpected content range".into());
        }
    } else if !response.status().is_success() {
        let status = response.status();
        return Err(response_issue(status, &response.text().await.unwrap_or_default()));
    }
    if response.headers().get(header::CONTENT_TYPE).and_then(|v|v.to_str().ok())
        .is_some_and(|v|v.to_ascii_lowercase().contains("text/html")) {
        return Err("RETRY: Google sent an HTML page instead of file data".into());
    }
    let mut f = async_fs::OpenOptions::new().create(true).append(true).open(path).await.map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut bytes = offset;
    let mut last_update = Instant::now();
    progress(app, file, bytes, "downloading", 0.0, None);
    loop {
        wait_for_run(control).await?;
        let chunk = match tokio::time::timeout(Duration::from_secs(120), response.chunk()).await {
            Ok(Ok(Some(data))) => data,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => return Err(format!("RETRY: Interrupted stream: {e}")),
            Err(_) => return Err("RETRY: No data received for 120 seconds".into())
        };
        if chunk.is_empty() { continue; }
        f.write_all(&chunk).await.map_err(|e| format!("PERMANENT: Cannot write to destination: {e}"))?;
        bytes += chunk.len() as u64;
        if let Some(size) = file.size {
            if bytes > size { return Err("RESET_PARTIAL: Received more data than expected".into()); }
        }
        if last_update.elapsed() >= Duration::from_millis(800) {
            let rate = (bytes-offset) as f64 / started.elapsed().as_secs_f64().max(0.001);
            progress(app, file, bytes, "downloading", rate, None);
            last_update = Instant::now();
        }
    }
    f.flush().await.map_err(|e| format!("PERMANENT: Cannot flush data: {e}"))?;
    progress(app, file, bytes, "downloading", 0.0, None);
    Ok(())
}

#[tauri::command]
async fn start_download(app: AppHandle, state: State<'_, Arc<AppState>>, plan: FolderPlan, directory: String) -> Result<(), String> {
    let prefs = state.prefs.lock().unwrap().clone();
    let destination = PathBuf::from(directory.trim());
    if !destination.is_absolute() || !destination.is_dir() { return Err("Choose an existing absolute destination folder".into()); }
    let mut seen = HashSet::new();
    for file in &plan.files { let rel = safe_relative(&file.relative_path)?; if !valid_id(&file.id) { return Err("Invalid file ID".into()); } if !seen.insert(rel.to_string_lossy().to_lowercase()) { return Err("Duplicate file destination path".into()); } }
    for path in &plan.empty_folders { safe_relative(path)?; }
    let run = Arc::new(RunControl::default());
    {
        let mut current = state.run.lock().unwrap();
        if current.is_some() { return Err("A download is already running. Finish cancelling before starting another".into()); }
        // No credentials are stored in the resume record.
        write_session(&app, &DownloadSession { plan: plan.clone(), directory: directory.trim().to_string(), auto_resume: true })?;
        *current = Some(run.clone());
    }
    let client = state.client.clone(); let app_handle = app.clone();
    let state_handle = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let result = download_folder(&app_handle, &client, &prefs, &plan, &destination, &run).await;
        if !result.cancelled && result.failed == 0 {
            if let Ok(path) = session_file(&app_handle) { let _ = fs::remove_file(path); }
        } else if let Ok(path) = session_file(&app_handle) {
            if let Ok(raw) = fs::read(&path) {
                if let Ok(mut session) = serde_json::from_slice::<DownloadSession>(&raw) {
                    session.auto_resume = false;
                    let _ = write_session(&app_handle, &session);
                }
            }
        }
        let _ = app_handle.emit("north:summary", result);
        let mut active = state_handle.run.lock().unwrap();
        if active.as_ref().is_some_and(|existing| Arc::ptr_eq(existing, &run)) { *active = None; }
    });
    Ok(())
}
async fn download_folder(app: &AppHandle, client: &Client, prefs: &Preferences, plan: &FolderPlan, destination: &Path, run: &Arc<RunControl>) -> RunSummary {
    let root = destination.join(sanitized_name(&plan.folder_name));
    let mut errors = vec![];
    if let Err(e) = async_fs::create_dir_all(&root).await { return RunSummary { completed:0, failed:plan.files.len(), cancelled:false, errors:vec![e.to_string()] }; }
    for name in &plan.empty_folders {
        match safe_relative(name) {
            Ok(p) => { if let Err(e)=async_fs::create_dir_all(root.join(p)).await { errors.push(e.to_string()); } }
            Err(e) => errors.push(e)
        }
    }
    let access = if plan.access_method == "public" && !prefs.api_key.is_empty() { Access::Public(prefs.api_key.clone()) }
        else { match refresh_access_token(client, prefs).await { Ok(t) => Access::OAuth(t), Err(e) => return RunSummary{completed:0,failed:plan.files.len(),cancelled:false,errors:vec![e]} } };
    let auth_cache: AuthCache = Arc::new(tokio::sync::Mutex::new((Instant::now(), access)));
    let mut tasks = tokio::task::JoinSet::new();
    let mut completed = 0; let mut failed = 0;
    let max_parallel = prefs.max_parallel.clamp(1,8);
    // Bound both active transfers AND completed-task bookkeeping.
    for file in plan.files.clone() {
        if run.cancelled.load(Ordering::SeqCst) { break; }
        while tasks.len() >= max_parallel {
            if let Some(joined) = tasks.join_next().await {
                match joined { Ok(Ok(()))=>completed+=1, Ok(Err(e))=>{failed+=1;errors.push(e)}, Err(e)=>{failed+=1;errors.push(e.to_string())} }
            }
        }
        let app = app.clone(); let root = root.clone(); let client = client.clone(); let prefs = prefs.clone(); let auth_cache = auth_cache.clone();
        let control = run.clone();
        tasks.spawn(async move {
            let result = download_one(&app, &client, &prefs, &auth_cache, &file, &root, &control).await;
            if let Err(ref e) = result {
                let partial_bytes = safe_relative(&file.relative_path).ok()
                    .map(|p| part_paths(&root.join(p)).0)
                    .and_then(|p| fs::metadata(p).ok())
                    .map(|m| m.len()).unwrap_or(0);
                progress(&app, &file, partial_bytes, "failed", 0.0, Some(e.clone()));
            }
            result.map_err(|e| format!("{}: {}", file.relative_path, e))
        });
    }
    while let Some(joined) = tasks.join_next().await {
        match joined { Ok(Ok(())) => completed+=1, Ok(Err(e)) => {failed+=1;errors.push(e)}, Err(e)=>{failed+=1;errors.push(e.to_string())} }
    }
    RunSummary {completed,failed,cancelled:run.cancelled.load(Ordering::SeqCst),errors}
}
#[tauri::command]
fn forget_session(app: AppHandle) -> Result<(), String> {
    let path = session_file(&app)?;
    if path.exists() { fs::remove_file(path).map_err(|e| e.to_string())?; }
    Ok(())
}

#[tauri::command]
fn open_developer_profile() -> Result<(), String> {
    Command::new("open").arg("https://www.instagram.com/muhaimenun.rafi/")
        .spawn().map_err(|e|format!("Cannot open Instagram: {e}"))?;
    Ok(())
}

#[tauri::command]
fn pause_download(state: State<'_, Arc<AppState>>, paused: bool) -> Result<(), String> {
    let guard=state.run.lock().unwrap(); let run=guard.as_ref().ok_or("No active transfer")?;
    run.paused.store(paused, Ordering::SeqCst); Ok(())
}
#[tauri::command]
fn cancel_download(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let guard=state.run.lock().unwrap(); let run=guard.as_ref().ok_or("No active transfer")?;
    run.cancelled.store(true, Ordering::SeqCst); Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let prefs = read_prefs(app.handle());
            let client = Client::builder().connect_timeout(Duration::from_secs(25)).pool_idle_timeout(Duration::from_secs(90)).build()?;
            app.manage(Arc::new(AppState { prefs: Mutex::new(prefs), run: Mutex::new(None), client }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_preferences, set_preferences, is_connected, connect_google, disconnect, scan_folder, start_download, recover_session, forget_session, open_developer_profile, pause_download, cancel_download])
        .run(tauri::generate_context!())
        .expect("Failed to start North Drive Downloader");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parse_folder_and_key() {
        let (id,k)=parse_drive_link("https://drive.google.com/drive/u/0/folders/1aBcD_efGh1234?resourcekey=0-secret").unwrap();
        assert_eq!(id,"1aBcD_efGh1234"); assert_eq!(k.as_deref(),Some("0-secret"));
    }
    #[test] fn rejects_wrong_host() {assert!(parse_drive_link("https://evil.com/drive/folders/12345678").is_err());}
    #[test] fn rejects_path_traversal() { assert!(safe_relative("../footage.mov").is_err()); assert!(safe_relative("A/B.mov").is_ok()); }
    #[test] fn collision_names() {let mut seen=HashSet::new();assert_eq!(unique_name("A.mov",&mut seen),"A.mov");assert_eq!(unique_name("a.MOV",&mut seen),"a (2).MOV");}
    #[test] fn drive_quota_is_retriable() {
        let raw = r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}],"message":"Rate Limit Exceeded"}}"#;
        assert!(response_issue(StatusCode::FORBIDDEN,raw).starts_with("QUOTA: "));
        assert!(response_issue(StatusCode::TOO_MANY_REQUESTS,raw).starts_with("RETRY: "));
    }
    #[test] fn drive_permissions_are_not_retried_forever() {
        let raw = r#"{"error":{"errors":[{"reason":"insufficientFilePermissions"}],"message":"Forbidden"}}"#;
        assert!(response_issue(StatusCode::FORBIDDEN,raw).starts_with("PERMANENT: "));
    }
    #[test] fn html_403_has_readable_message() {
        let e = response_issue(StatusCode::FORBIDDEN,"<html><body>error</body></html>");
        assert!(e.starts_with("QUOTA: "));
        assert!(e.contains("HTML error page"));
        assert!(!e.contains("<html>"));
    }
    #[test] fn partial_files_have_matching_sidecars() {
        let path = Path::new("/tmp/Footage.MP4");
        let (partial, sidecar) = part_paths(path);
        assert_eq!(partial.file_name().unwrap().to_str().unwrap(), "Footage.MP4.northpart");
        assert_eq!(sidecar.file_name().unwrap().to_str().unwrap(), "Footage.MP4.northpart.json");
    }
}
