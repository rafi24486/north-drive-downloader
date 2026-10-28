#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod guest;
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
const SCOPE: &str = "openid email profile https://www.googleapis.com/auth/drive.readonly";
const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const KEYCHAIN_SERVICE: &str = "com.northdrive.downloader.oauth";
// The publisher's Desktop OAuth client ID is public by design; never embed a client secret.
const BUNDLED_GOOGLE_CLIENT_ID: Option<&str> = option_env!("NORTH_GOOGLE_CLIENT_ID");
fn oauth_client_id() -> &'static str {
    BUNDLED_GOOGLE_CLIENT_ID.filter(|s| !s.trim().is_empty()).unwrap_or("")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    save_directory: String,
    max_parallel: usize,
}
impl Default for Preferences {
    fn default() -> Self {
        Self { save_directory: String::new(), max_parallel: 4 }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GoogleProfile {
    // Google sub is immutable for this account + app. Never use name/picture as an identity key.
    sub: String,
    name: Option<String>,
    email: Option<String>,
    picture: Option<String>,
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
fn keychain_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, "refresh-token").map_err(|e| e.to_string())
}
fn has_login() -> bool { keychain_entry().and_then(|e| e.get_password().map_err(|x| x.to_string())).is_ok() }

#[tauri::command]
fn get_preferences(state: State<'_, Arc<AppState>>) -> Preferences { state.prefs.lock().unwrap().clone() }
#[tauri::command]
fn is_connected() -> bool { has_login() }
#[tauri::command]
fn get_auth_config() -> serde_json::Value {
    serde_json::json!({
        "managedOAuthConfigured": BUNDLED_GOOGLE_CLIENT_ID.is_some_and(|id| !id.trim().is_empty())
    })
}
#[tauri::command]
fn set_preferences(app: AppHandle, state: State<'_, Arc<AppState>>, preferences: Preferences) -> Result<(), String> {
    if preferences.max_parallel == 0 || preferences.max_parallel > 8 { return Err("Parallel downloads must be between 1 and 8".into()); }
    let preferences = Preferences { save_directory: preferences.save_directory.trim().to_string(), max_parallel: preferences.max_parallel };
    persist_prefs(&app, &preferences)?;
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
async fn connect_google(state: State<'_, Arc<AppState>>) -> Result<GoogleProfile, String> {
    let client_id = oauth_client_id();
    if client_id.is_empty() { return Err("Google sign-in is not configured for this build. Contact the North Drive publisher.".into()); }
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
    }).await.map_err(|_| "Google sign-in did not complete. If Google showed Access blocked, check your OAuth Test users and Desktop client setup.".to_string())??;
    let response = state.client.post("https://oauth2.googleapis.com/token").form(&[
        ("code", code.as_str()), ("client_id", client_id), ("code_verifier", verifier.as_str()),
        ("redirect_uri", redirect_uri.as_str()), ("grant_type", "authorization_code")
    ]).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let detail = response.text().await.unwrap_or_default();
        return Err(oauth_failure_text(&detail));
    }
    let token: OAuthToken = response.json().await.map_err(|e| e.to_string())?;
    let refresh = token.refresh_token.ok_or("Google did not return a refresh token; revoke access and retry")?;
    let profile = fetch_google_profile(&state.client, &token.access_token).await?;
    ensure_google_drive_enabled(&state.client, &token.access_token).await?;
    keychain_entry()?.set_password(&refresh).map_err(|e| e.to_string())?;
    Ok(profile)
}

async fn refresh_access_token(client: &Client, _prefs: &Preferences) -> Result<String, String> {
    let client_id = oauth_client_id();
    if client_id.is_empty() { return Err("Google sign-in is not configured in this build. Contact the publisher".into()); }
    let refresh = keychain_entry()?.get_password().map_err(|_| "Google account not connected. Go to Accounts and sign in".to_string())?;
    let response = client.post("https://oauth2.googleapis.com/token").form(&[
        ("client_id", client_id), ("refresh_token", refresh.as_str()), ("grant_type", "refresh_token")
    ]).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let detail = response.text().await.unwrap_or_default();
        let code = serde_json::from_str::<serde_json::Value>(&detail).ok()
            .and_then(|v| v.get("error").and_then(|s| s.as_str()).map(str::to_owned))
            .unwrap_or_else(|| "unknown".into());
        return Err(format!("Google session is invalid ({code}). Reconnect your Google account."));
    }
    let token: OAuthToken = response.json().await.map_err(|e| e.to_string())?;
    Ok(token.access_token)
}

async fn ensure_google_drive_enabled(client: &Client, token: &str) -> Result<(), String> {
    // A tiny live API probe: validates that the publisher's Google Cloud project
    // has enabled Drive API and that Drive read scope was actually granted.
    let response = client.get(DRIVE_BASE)
        .bearer_auth(token)
        .query(&[("pageSize", "1"), ("fields", "files(id)")])
        .timeout(Duration::from_secs(20))
        .send().await.map_err(|e| format!("Could not reach Google Drive API: {e}"))?;
    if response.status().is_success() { return Ok(()); }
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let text = body.to_lowercase();
    if text.contains("has not been used") || text.contains("api is disabled")
        || text.contains("accessnotconfigured") || text.contains("service_disabled") {
        return Err("Google Drive API is disabled in the publisher's Google Cloud project. Enable it in APIs & Services, then try again.".into());
    }
    if status == 401 { return Err("Google Drive rejected the sign-in token. Disconnect and sign in again.".into()); }
    if status == 403 && (text.contains("insufficient") || text.contains("scope")) {
        return Err("Google Drive read permission was not granted. Sign in again and approve Drive access.".into());
    }
    let message = serde_json::from_str::<serde_json::Value>(&body).ok()
        .and_then(|v| v.pointer("/error/message").and_then(|s| s.as_str()).map(str::to_owned))
        .unwrap_or_else(|| format!("HTTP {status}"));
    Err(format!("Google Drive check failed: {}", message.chars().take(190).collect::<String>()))
}

fn oauth_failure_text(body: &str) -> String {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    let code = value.as_ref().and_then(|v| v.get("error")).and_then(|v| v.as_str()).unwrap_or("unknown");
    match code {
        "invalid_client" => "Google rejected North Drive's OAuth client. Confirm the client ID is a Desktop app OAuth client in the correct Google Cloud project.".into(),
        "unauthorized_client" | "redirect_uri_mismatch" => "Google rejected this desktop sign-in redirect. Check the Google OAuth client's type is Desktop app.".into(),
        "access_denied" => "Google denied access. If the app is in Testing, add your Gmail under Google Auth Platform > Audience > Test users.".into(),
        "invalid_grant" => "The Google sign-in code expired or was already used. Try connecting again.".into(),
        _ => format!("Google OAuth failed ({code}). Check Google Cloud authentication settings.")
    }
}

async fn fetch_google_profile(client: &Client, access_token: &str) -> Result<GoogleProfile, String> {
    let response = client.get(GOOGLE_USERINFO_URL)
        .bearer_auth(access_token)
        .send().await.map_err(|e| format!("Could not load Google profile: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Google profile request failed (HTTP {}). Please reconnect.", response.status()));
    }
    let profile: GoogleProfile = response.json().await.map_err(|e| format!("Invalid Google profile: {e}"))?;
    if profile.sub.trim().is_empty() { return Err("Google did not provide a valid account identifier".into()); }
    Ok(profile)
}

#[tauri::command]
async fn get_google_profile(state: State<'_, Arc<AppState>>) -> Result<GoogleProfile, String> {
    let prefs = state.prefs.lock().unwrap().clone();
    let token = refresh_access_token(&state.client, &prefs).await?;
    let profile = fetch_google_profile(&state.client, &token).await?;
    ensure_google_drive_enabled(&state.client, &token).await?;
    Ok(profile)
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

async fn json_request<T: for<'de> Deserialize<'de>>(client: &Client, access: &Access, url: &str, params: &[(&str, String)], keys: &HashMap<String, String>) -> Result<T, String> {
    let mut req = access.request(client, url).query(params);
    if !keys.is_empty() {
        let header_value = keys.iter().map(|(k,v)| format!("{k}/{v}")).collect::<Vec<_>>().join(",");
        req = req.header("X-Goog-Drive-Resource-Keys", header_value);
    }
    let response = req.send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let code = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(format!("Google Drive HTTP {}: {}", code.as_u16(), detail.chars().take(250).collect::<String>()));
    }
    response.json::<T>().await.map_err(|e| format!("Unexpected Google Drive response: {e}"))
}
async fn metadata(client: &Client, access: &Access, id: &str, keys: &HashMap<String,String>) -> Result<DriveFile, String> {
    json_request(client, access, &format!("{DRIVE_BASE}/{id}"), &[
        ("fields", "id,name,mimeType,size,md5Checksum,resourceKey,capabilities(canDownload)".into()),
        ("supportsAllDrives", "true".into())
    ], keys).await
}
async fn resolve_access(client: &Client, prefs: &Preferences, id: &str, rk: &Option<String>) -> Result<(Access, DriveFile), String> {
    if !has_login() { return Err("Connect your Google account first to open Drive folder links".into()); }
    let mut keys = HashMap::new();
    if let Some(k) = rk { keys.insert(id.to_string(), k.clone()); }
    let oauth = Access::OAuth(refresh_access_token(client, prefs).await?);
    let meta = metadata(client, &oauth, id, &keys).await
        .map_err(|e| format!("Unable to access this folder with your connected Google account: {e}"))?;
    Ok((oauth, meta))
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
async fn scan_folder(link: String) -> Result<FolderPlan, String> {
    guest::scan_folder(&link).await
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
    let mut f = async_fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut md5 = md5::Context::new(); let mut buf = [0u8; 1024 * 128];
    loop { let n = f.read(&mut buf).await.map_err(|e| e.to_string())?; if n == 0 { break; } md5.consume(&buf[..n]); }
    Ok(format!("{:x}", md5.compute()).eq_ignore_ascii_case(expected))
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
async fn download_one(app: &AppHandle, client: &Client, prefs: &Preferences, auth_cache: &AuthCache, file: &DownloadItem, root: &Path, control: &RunControl) -> Result<(), String> {
    let relative = safe_relative(&file.relative_path)?;
    let target = root.join(relative);
    if let Some(parent) = target.parent() { async_fs::create_dir_all(parent).await.map_err(|e| e.to_string())?; }
    // Skip only previously verified matching files. Never overwrite other files silently.
    if target.exists() {
        if file.md5_checksum.as_ref().is_some_and(|sum| file.size == fs::metadata(&target).ok().map(|m| m.len()) && sum.len() == 32) {
            if verify_md5(&target, file.md5_checksum.as_ref().unwrap()).await? {
                progress(app, file, file.size.unwrap_or(0), "completed", 0.0, Some("Already verified".into()));
                return Ok(());
            }
        }
        return Err(format!("Existing destination file differs: {}. Move or rename it before downloading", target.display()));
    }
    let (part, meta) = part_paths(&target);
    let fingerprint = serde_json::json!({"id":file.id,"size":file.size,"md5":file.md5_checksum,"mime":file.mime_type,"export":file.export_mime});
    let expected = serde_json::to_vec(&fingerprint).map_err(|e| e.to_string())?;
    if part.exists() {
        if fs::read(&meta).ok().as_deref() != Some(expected.as_slice()) {
            return Err(format!("Partial file metadata does not match source: {}. Remove the .northpart files to restart", target.display()));
        }
    } else { async_fs::write(&meta, &expected).await.map_err(|e| e.to_string())?; }
    let mut attempt = 0;
    loop {
        wait_for_run(control).await?;
        attempt += 1;
        let result = download_attempt(app, client, prefs, auth_cache, file, &part, control).await;
        match result {
            Ok(()) => break,
            Err(e) => {
                if control.cancelled.load(Ordering::SeqCst) { return Err("Cancelled".into()); }
                if attempt >= 5 { return Err(format!("{} (after {attempt} attempts)", e)); }
                progress(app, file, fs::metadata(&part).map(|m|m.len()).unwrap_or(0), "retrying", 0.0, Some(format!("Retry {attempt}/4: {e}")));
                sleep(Duration::from_secs((1u64 << attempt).min(30))).await;
            }
        }
    }
    let length = fs::metadata(&part).map_err(|e| e.to_string())?.len();
    if let Some(size) = file.size { if length != size { return Err(format!("Downloaded size mismatch: expected {size}, got {length}")); } }
    if let Some(expected) = &file.md5_checksum {
        progress(app, file, length, "verifying", 0.0, None);
        if !verify_md5(&part, expected).await? { return Err("MD5 checksum mismatch. Partial file kept for inspection".into()); }
    }
    // Check for a conflicting destination again before finalizing.
    if target.exists() { return Err(format!("Destination appeared during transfer: {}", target.display())); }
    // Same-directory rename is atomic on typical macOS volumes.
    async_fs::rename(&part, &target).await.map_err(|e| e.to_string())?;
    let _ = async_fs::remove_file(meta).await;
    progress(app, file, length, "completed", 0.0, None);
    Ok(())
}
async fn download_attempt(app: &AppHandle, client: &Client, prefs: &Preferences, auth_cache: &AuthCache, file: &DownloadItem, path: &Path, control: &RunControl) -> Result<(), String> {
    let mut offset = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    // Google Workspace exports do not support byte ranges, so restart any partial export.
    if file.export_mime.is_some() && offset > 0 {
        async_fs::remove_file(path).await.map_err(|e| e.to_string())?;
        offset = 0;
    }
    if file.size == Some(offset) && offset > 0 { return Ok(()); }
    let endpoint = if let Some(mime) = &file.export_mime {
        let _ = mime;
        format!("{DRIVE_BASE}/{}/export", file.id)
    } else { format!("{DRIVE_BASE}/{}", file.id) };
    // Share a cached OAuth token across concurrent file transfers; refresh before expiry.
    let auth = current_access(auth_cache, client, prefs).await?;
    let mut req = auth.request(client, &endpoint);
    if let Some(mime) = &file.export_mime { req = req.query(&[("mimeType",mime.as_str())]); }
    else { req = req.query(&[("alt","media"), ("supportsAllDrives", "true")]); }
    if let Some(key) = &file.resource_key { req = req.header("X-Goog-Drive-Resource-Keys",format!("{}/{}",file.id,key)); }
    if offset > 0 { req = req.header(header::RANGE, format!("bytes={offset}-")); }
    let mut response = req.send().await.map_err(|e| e.to_string())?;
    if response.status() == StatusCode::UNAUTHORIZED { expire_access(auth_cache).await; return Err("Google session expired; retrying with fresh token".into()); }
    if offset > 0 {
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(format!("Server did not honor resume range (HTTP {})", response.status()));
        }
        let range = response.headers().get(header::CONTENT_RANGE).and_then(|v|v.to_str().ok()).unwrap_or("");
        if !range.starts_with(&format!("bytes {offset}-")) { return Err("Server returned unexpected content range".into()); }
    } else if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Google Drive HTTP {status}: {}",text.chars().take(180).collect::<String>()));
    }
    let mut f = async_fs::OpenOptions::new().create(true).append(true).open(path).await.map_err(|e| e.to_string())?;
    let started = Instant::now(); let mut bytes = offset; let mut last_update = Instant::now();
    progress(app, file, bytes, "downloading", 0.0, None);
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        wait_for_run(control).await?;
        f.write_all(&chunk).await.map_err(|e| e.to_string())?;
        bytes += chunk.len() as u64;
        if last_update.elapsed() >= Duration::from_millis(280) {
            let rate = (bytes-offset) as f64 / started.elapsed().as_secs_f64().max(0.001);
            progress(app, file, bytes, "downloading", rate, None);
            last_update = Instant::now();
        }
    }
    f.flush().await.map_err(|e| e.to_string())?;
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
        *current = Some(run.clone());
    }
    let client = state.client.clone(); let app_handle = app.clone();
    let state_handle = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let result = guest::download_folder(&app_handle, &plan, &destination, &run).await;
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
    let access = match refresh_access_token(client, prefs).await {
        Ok(token) => Access::OAuth(token),
        Err(e) => return RunSummary { completed:0, failed:plan.files.len(), cancelled:false, errors:vec![e] }
    };
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
            let client = Client::builder().connect_timeout(Duration::from_secs(25)).build()?;
            app.manage(Arc::new(AppState { prefs: Mutex::new(prefs), run: Mutex::new(None), client }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_preferences, set_preferences, is_connected, get_auth_config, connect_google, get_google_profile, disconnect, scan_folder, start_download, pause_download, cancel_download])
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
    #[test] fn oauth_error_classification() {
        assert!(oauth_failure_text(r#"{"error":"invalid_client"}"#).contains("Desktop"));
        assert!(oauth_failure_text(r#"{"error":"access_denied"}"#).contains("Test users"));
    }
    #[test] fn rejects_wrong_host() {assert!(parse_drive_link("https://evil.com/drive/folders/12345678").is_err());}
    #[test] fn rejects_path_traversal() { assert!(safe_relative("../footage.mov").is_err()); assert!(safe_relative("A/B.mov").is_ok()); }
    #[test] fn collision_names() {let mut seen=HashSet::new();assert_eq!(unique_name("A.mov",&mut seen),"A.mov");assert_eq!(unique_name("a.MOV",&mut seen),"a (2).MOV");}
}
