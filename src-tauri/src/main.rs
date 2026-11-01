#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet, VecDeque}, fs, path::{Component, Path, PathBuf}, sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex}, time::{Duration, Instant}};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{fs as async_fs, io::{AsyncReadExt, AsyncWriteExt}, time::sleep};

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const SHORTCUT_MIME: &str = "application/vnd.google-apps.shortcut";
const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3/files";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    api_key: String,
    save_directory: String,
    max_parallel: usize,
}
impl Default for Preferences {
    fn default() -> Self {
        Self { api_key: String::new(), save_directory: String::new(), max_parallel: 2 }
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
    modified_time: Option<String>,
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
    modified_time: Option<String>,
    capabilities: Option<Capabilities>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities { can_download: Option<bool> }
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilePage { #[serde(default)] files: Vec<DriveFile>, next_page_token: Option<String> }
// This release never stores or sends Google account credentials.
#[derive(Clone)]
struct Access { api_key: String }
impl Access {
    fn request(&self, client: &Client, url: &str) -> reqwest::RequestBuilder {
        client.get(url).query(&[("key", self.api_key.as_str())])
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
#[tauri::command]
fn get_preferences(state: State<'_, Arc<AppState>>) -> Preferences { state.prefs.lock().unwrap().clone() }
#[tauri::command]
fn set_preferences(app: AppHandle, state: State<'_, Arc<AppState>>, preferences: Preferences) -> Result<(), String> {
    if preferences.max_parallel == 0 || preferences.max_parallel > 8 { return Err("Parallel downloads must be between 1 and 8".into()); }
    let preferences = Preferences { api_key: preferences.api_key.trim().to_string(), save_directory: preferences.save_directory.trim().to_string(), max_parallel: preferences.max_parallel };
    persist_prefs(&app, &preferences)?;
    *state.prefs.lock().unwrap() = preferences;
    Ok(())
}
fn parse_drive_link(link: &str) -> Result<(String, Option<String>), String> {
    let url = url::Url::parse(link.trim()).map_err(|_| "Paste a complete Google Drive folder URL".to_string())?;
    let domain = url.host_str().unwrap_or("");
    if url.scheme() != "https" || (domain != "drive.google.com" && domain != "docs.google.com") { return Err("Only secure Google Drive URLs are supported".into()); }
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
    let key_header = if keys.is_empty() { None } else {
        Some(keys.iter().map(|(k,v)| format!("{k}/{v}")).collect::<Vec<_>>().join(","))
    };
    for attempt in 1..=6u32 {
        let mut request = access.request(client, url).query(params);
        if let Some(header_value) = &key_header {
            request = request.header("X-Goog-Drive-Resource-Keys", header_value);
        }
        let result = request.send().await;
        match result {
            Ok(response) if response.status().is_success() => {
                return response.json::<T>().await.map_err(|e| format!("Unexpected Google Drive response: {e}"));
            }
            Ok(response) => {
                let status = response.status();
                let retry_after = response.headers().get(header::RETRY_AFTER)
                    .and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<u64>().ok());
                let detail = response.text().await.unwrap_or_default();
                let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
                    || (status == StatusCode::FORBIDDEN && (detail.contains("rateLimitExceeded") || detail.contains("userRateLimitExceeded")));
                if !retryable || attempt == 6 {
                    return Err(format!("Google Drive HTTP {}: {}", status.as_u16(), detail.chars().take(300).collect::<String>()));
                }
                let delay = retry_after.unwrap_or((1u64 << attempt).min(30)).clamp(1, 60);
                sleep(Duration::from_secs(delay)).await;
            }
            Err(e) => {
                if attempt == 6 { return Err(format!("Google Drive network error after {attempt} attempts: {e}")); }
                sleep(Duration::from_secs((1u64 << attempt).min(30))).await;
            }
        }
    }
    Err("Drive request exhausted retries".into())
}

async fn metadata(client: &Client, access: &Access, id: &str, keys: &HashMap<String,String>) -> Result<DriveFile, String> {
    json_request(client, access, &format!("{DRIVE_BASE}/{id}"), &[
        ("fields", "id,name,mimeType,size,md5Checksum,resourceKey,modifiedTime,capabilities(canDownload)".into()),
        ("supportsAllDrives", "true".into())
    ], keys).await
}
async fn resolve_access(client: &Client, prefs: &Preferences, id: &str, rk: &Option<String>) -> Result<(Access, DriveFile), String> {
    let api_key = prefs.api_key.trim();
    if api_key.is_empty() {
        return Err("To scan public Google Drive folders without signing in, add a Google Drive API key in Settings. No Google account login is required.".into());
    }
    let mut keys = HashMap::new();
    if let Some(k) = rk { keys.insert(id.to_string(), k.clone()); }
    let access = Access { api_key: api_key.to_string() };
    let meta = metadata(client, &access, id, &keys).await.map_err(|error| {
        if error.contains("HTTP 401") || error.contains("HTTP 403") || error.contains("HTTP 404") {
            format!("This folder cannot be accessed publicly. Set Google Drive sharing to 'Anyone with the link' and verify your API key. Details: {error}")
        } else { error }
    })?;
    Ok((access, meta))
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
                ("fields", "nextPageToken,files(id,name,mimeType,size,md5Checksum,resourceKey,modifiedTime,capabilities(canDownload))".into()),
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
                files.push(DownloadItem { id: entry.id, name: final_name, relative_path: file_path, size, mime_type: entry.mime_type, md5_checksum: entry.md5_checksum, resource_key: entry.resource_key, export_mime: export.map(|(m,_)| m.into()), modified_time: entry.modified_time });
            }
            page = result.next_page_token;
            if page.is_none() { break; }
        }
        if !found_children && !parent_path.is_empty() { empty_folders.push(parent_path); }
    }
    Ok(FolderPlan { folder_id: id, folder_name: sanitized_name(&root.name), files, empty_folders, total_bytes, warnings, access_method: "public".into(), folder_resource_key: rk.or(root.resource_key) })
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

fn retryable_transfer_error(error: &str) -> bool {
    // Do not waste minutes retrying permanent permissions, invalid requests, or unavailable files.
    let permanent = ["HTTP 400", "HTTP 401", "HTTP 404", "HTTP 410", "HTTP 416"];
    if permanent.iter().any(|code| error.contains(code)) { return false; }
    if error.contains("HTTP 403") {
        return ["rateLimitExceeded", "userRateLimitExceeded", "quotaExceeded", "Rate Limit", "rate limit"]
            .iter().any(|reason| error.contains(reason));
    }
    if error.contains("MD5 checksum mismatch after clean retry")
        || error.contains("Remote file changed during resume") { return false; }
    true
}

fn validate_resume_response(
    offset: u64, status: StatusCode, content_range: Option<&str>, expected_size: Option<u64>,
) -> Result<bool, String> {
    if offset == 0 {
        if status != StatusCode::OK { return Err(format!("Unexpected download response HTTP {status}")); }
        return Ok(false);
    }
    // A proxy may ignore Range. 200 means replace the partial; NEVER append a whole file to it.
    if status == StatusCode::OK { return Ok(true); }
    if status != StatusCode::PARTIAL_CONTENT {
        return Err(format!("Cannot resume at byte {offset}: server returned HTTP {status}"));
    }
    let value = content_range.ok_or("Missing Content-Range on resumed download")?;
    let (bounds, size) = value.strip_prefix("bytes ").and_then(|v| v.split_once('/'))
        .ok_or("Invalid Content-Range on resumed download")?;
    let (start, end) = bounds.split_once('-').ok_or("Invalid range boundaries")?;
    let start = start.parse::<u64>().map_err(|_| "Invalid start of Content-Range")?;
    let end = end.parse::<u64>().map_err(|_| "Invalid end of Content-Range")?;
    if start != offset || end < start {
        return Err(format!("Unexpected Content-Range {value}, requested byte {offset}"));
    }
    if size != "*" {
        let remote_size = size.parse::<u64>().map_err(|_| "Invalid total in Content-Range")?;
        if let Some(expected) = expected_size {
            if remote_size != expected { return Err("Remote file changed during resume; rescan the folder".into()); }
        }
        if end >= remote_size { return Err("Content-Range exceeds remote file length".into()); }
    }
    Ok(false)
}

// Receipts allow idempotent retries for Workspace exports, which have no Drive MD5 or size.
fn receipt_path(root: &Path, file: &DownloadItem) -> PathBuf {
    let digest = Sha256::digest(file.relative_path.as_bytes());
    root.join(".north-drive-state").join(format!("{digest:x}.json"))
}
async fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = async_fs::File::open(path).await.map_err(|e| e.to_string())?;
    let mut sha = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let n = f.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if n == 0 { break; }
        sha.update(&buffer[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}
fn receipt_source(file: &DownloadItem) -> serde_json::Value {
    // Keep the partial-file fingerprint backward compatible with 0.1.2.
    serde_json::json!({"id":file.id,"size":file.size,"md5":file.md5_checksum,"mime":file.mime_type,"export":file.export_mime})
}
fn completed_source(file: &DownloadItem) -> serde_json::Value {
    serde_json::json!({"source":receipt_source(file),"modifiedTime":file.modified_time})
}
async fn previously_verified(root: &Path, file: &DownloadItem, target: &Path) -> Result<bool, String> {
    if !target.is_file() { return Ok(false); }
    let len = async_fs::metadata(target).await.map_err(|e| e.to_string())?.len();
    if file.md5_checksum.as_ref().is_some_and(|sum| file.size == Some(len) && sum.len() == 32) {
        return verify_md5(target, file.md5_checksum.as_ref().unwrap()).await;
    }
    if file.export_mime.is_some() {
        let receipt = match async_fs::read(receipt_path(root, file)).await {
            Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes).ok(),
            Err(_) => None,
        };
        if let Some(saved) = receipt {
            if saved.get("source") == Some(&completed_source(file))
                && saved.get("length").and_then(|v| v.as_u64()) == Some(len) {
                let actual_hash = sha256_file(target).await?;
                if saved.get("sha256").and_then(|v| v.as_str()) == Some(actual_hash.as_str()) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
async fn record_export_completion(root: &Path, file: &DownloadItem, target: &Path, length: u64) -> Result<(), String> {
    if file.export_mime.is_none() { return Ok(()); }
    let path = receipt_path(root, file);
    let dir = path.parent().ok_or("Invalid receipt directory")?;
    async_fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    let payload = serde_json::json!({
        "source": completed_source(file), "length": length, "sha256": sha256_file(target).await?
    });
    let bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    async_fs::write(&temp, bytes).await.map_err(|e| e.to_string())?;
    async_fs::rename(&temp, &path).await.map_err(|e| e.to_string())
}

async fn download_one(app: &AppHandle, client: &Client, access: &Access, file: &DownloadItem, root: &Path, control: &RunControl) -> Result<(), String> {
    let relative = safe_relative(&file.relative_path)?;
    let target = root.join(relative);
    if let Some(parent) = target.parent() { async_fs::create_dir_all(parent).await.map_err(|e| e.to_string())?; }
    if target.exists() {
        if previously_verified(root, file, &target).await? {
            progress(app, file, file.size.unwrap_or(fs::metadata(&target).map(|m| m.len()).unwrap_or(0)), "completed", 0.0, Some("Already verified".into()));
            return Ok(());
        }
        return Err(format!("Existing destination file differs: {}. Move or rename it before downloading", target.display()));
    }
    let (part, meta) = part_paths(&target);
    let expected = serde_json::to_vec(&receipt_source(file)).map_err(|e| e.to_string())?;
    if part.exists() {
        if fs::read(&meta).ok().as_deref() != Some(expected.as_slice()) {
            return Err(format!("Partial file metadata does not match source: {}. Remove the .northpart files to restart", target.display()));
        }
    } else { async_fs::write(&meta, &expected).await.map_err(|e| e.to_string())?; }
    let mut attempt = 0u32;
    let mut checksum_resets = 0u32;
    loop {
        wait_for_run(control).await?;
        attempt += 1;
        let mut result = download_attempt(app, client, access, file, &part, control).await;
        if result.is_ok() {
            match async_fs::metadata(&part).await {
                Ok(m) => {
                    let length = m.len();
                    if file.size.is_some_and(|size| length != size) {
                        result = Err(format!("Incomplete transfer: expected {} bytes, received {length}", file.size.unwrap_or(0)));
                    } else if let Some(checksum) = &file.md5_checksum {
                        progress(app, file, length, "verifying", 0.0, None);
                        match verify_md5(&part, checksum).await {
                            Ok(true) => {}
                            Ok(false) if checksum_resets == 0 => {
                                checksum_resets += 1;
                                match async_fs::remove_file(&part).await {
                                    Ok(()) => result = Err("MD5 checksum mismatch; retrying a fresh copy".into()),
                                    Err(e) => result = Err(format!("Cannot reset corrupt partial: {e}")),
                                }
                            }
                            Ok(false) => result = Err("MD5 checksum mismatch after clean retry; rescan the folder".into()),
                            Err(e) => result = Err(format!("Cannot verify MD5: {e}")),
                        }
                    }
                }
                Err(e) => result = Err(format!("Cannot inspect partial file: {e}")),
            }
        }
        match result {
            Ok(()) => break,
            Err(e) => {
                if control.cancelled.load(Ordering::SeqCst) { return Err("Cancelled".into()); }
                if attempt >= 8 || !retryable_transfer_error(&e) {
                    return Err(format!("{e} (attempt {attempt}/8; partial download preserved)"));
                }
                let backoff = (1u64 << attempt.min(6)).min(45);
                progress(app, file, fs::metadata(&part).map(|m| m.len()).unwrap_or(0),
                    "retrying", 0.0, Some(format!("Retry {attempt}/7 in {backoff}s: {e}")));
                // Poll cancellation while sleeping instead of trapping users in long backoffs.
                for _ in 0..(backoff * 4) {
                    wait_for_run(control).await?;
                    sleep(Duration::from_millis(250)).await;
                }
            }
        }
    }
    let length = fs::metadata(&part).map_err(|e| e.to_string())?.len();
    if target.exists() { return Err(format!("Destination appeared during transfer: {}", target.display())); }
    async_fs::rename(&part, &target).await.map_err(|e| e.to_string())?;
    let _ = async_fs::remove_file(meta).await;
    // A receipt failure must not mark an otherwise verified file as failed.
    let _ = record_export_completion(root, file, &target, length).await;
    progress(app, file, length, "completed", 0.0, None);
    Ok(())
}

async fn download_attempt(app: &AppHandle, client: &Client, access: &Access, file: &DownloadItem, path: &Path, control: &RunControl) -> Result<(), String> {
    let mut offset = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    // Google Workspace exports do not support byte ranges; partial exports must restart.
    if file.export_mime.is_some() && offset > 0 {
        async_fs::remove_file(path).await.map_err(|e| e.to_string())?;
        offset = 0;
    }
    if file.size.is_some_and(|size| offset > size) {
        async_fs::remove_file(path).await.map_err(|e| e.to_string())?;
        offset = 0;
    }
    if file.size == Some(offset) && offset > 0 { return Ok(()); }
    let endpoint = if file.export_mime.is_some() {
        format!("{DRIVE_BASE}/{}/export", file.id)
    } else { format!("{DRIVE_BASE}/{}", file.id) };
    let mut req = access.request(client, &endpoint);
    if let Some(mime) = &file.export_mime { req = req.query(&[("mimeType", mime.as_str())]); }
    else { req = req.query(&[("alt", "media"), ("supportsAllDrives", "true")]); }
    if let Some(key) = &file.resource_key { req = req.header("X-Goog-Drive-Resource-Keys", format!("{}/{}", file.id, key)); }
    if offset > 0 { req = req.header(header::RANGE, format!("bytes={offset}-")); }
    let mut response = req.send().await.map_err(|e| format!("Network error starting transfer: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Google Drive HTTP {}: {}", status.as_u16(), body.chars().take(400).collect::<String>()));
    }
    let restart = validate_resume_response(
        offset, response.status(), response.headers().get(header::CONTENT_RANGE).and_then(|h| h.to_str().ok()), file.size,
    )?;
    if restart {
        // Server ignored Range (HTTP 200); truncate instead of corrupting by appending.
        offset = 0;
    }
    let expected_response_len = response.content_length();
    let mut f = async_fs::OpenOptions::new().create(true).write(true).append(!restart)
        .truncate(restart).open(path).await.map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut bytes = offset;
    let mut last_update = Instant::now();
    progress(app, file, bytes, "downloading", 0.0, None);
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(120), response.chunk()).await
            .map_err(|_| "Network stalled for 120 seconds; resuming transfer".to_string())?
            .map_err(|e| format!("Transfer interrupted: {e}"))?;
        let Some(chunk) = chunk else { break; };
        wait_for_run(control).await?;
        f.write_all(&chunk).await.map_err(|e| format!("Cannot write downloaded file (check disk space): {e}"))?;
        bytes += chunk.len() as u64;
        if last_update.elapsed() >= Duration::from_millis(280) {
            let rate = (bytes - offset) as f64 / started.elapsed().as_secs_f64().max(0.001);
            progress(app, file, bytes, "downloading", rate, None);
            last_update = Instant::now();
        }
    }
    f.flush().await.map_err(|e| e.to_string())?;
    if let Some(expected) = expected_response_len {
        if bytes - offset != expected {
            return Err(format!("Incomplete HTTP response: expected {expected} bytes, received {}", bytes - offset));
        }
    }
    progress(app, file, bytes, "downloading", 0.0, None);
    Ok(())
}

#[tauri::command]
async fn start_download(app: AppHandle, state: State<'_, Arc<AppState>>, plan: FolderPlan, directory: String) -> Result<(), String> {
    let prefs = state.prefs.lock().unwrap().clone();
    if plan.access_method != "public" { return Err("Only publicly accessible Google Drive folders are supported".into()); }
    if prefs.api_key.trim().is_empty() { return Err("Add a Google Drive API key in Settings before downloading a public folder".into()); }
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
        let result = download_folder(&app_handle, &client, &prefs, &plan, &destination, &run).await;
        {
            let mut active = state_handle.run.lock().unwrap();
            if active.as_ref().is_some_and(|existing| Arc::ptr_eq(existing, &run)) { *active = None; }
        }
        let _ = app_handle.emit("north:summary", result);
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
    let access = Arc::new(Access { api_key: prefs.api_key.clone() });
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
        let app = app.clone(); let root = root.clone(); let client = client.clone(); let access = access.clone();
        let control = run.clone();
        tasks.spawn(async move {
            let result = download_one(&app, &client, &access, &file, &root, &control).await;
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
            let client = Client::builder().connect_timeout(Duration::from_secs(25)).pool_idle_timeout(Duration::from_secs(60)).build()?;
            app.manage(Arc::new(AppState { prefs: Mutex::new(prefs), run: Mutex::new(None), client }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_preferences, set_preferences, scan_folder, start_download, pause_download, cancel_download])
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
    #[test] fn rejects_wrong_host() {assert!(parse_drive_link("https://evil.com/drive/folders/12345678").is_err());assert!(parse_drive_link("http://drive.google.com/drive/folders/12345678").is_err());}
    #[test] fn rejects_path_traversal() { assert!(safe_relative("../footage.mov").is_err()); assert!(safe_relative("A/B.mov").is_ok()); }
    #[test] fn collision_names() {let mut seen=HashSet::new();assert_eq!(unique_name("A.mov",&mut seen),"A.mov");assert_eq!(unique_name("a.MOV",&mut seen),"a (2).MOV");}

    #[test] fn resumed_ranges_are_checked() {
        assert_eq!(validate_resume_response(0, StatusCode::OK, None, Some(100)).unwrap(), false);
        assert_eq!(validate_resume_response(50, StatusCode::PARTIAL_CONTENT, Some("bytes 50-99/100"), Some(100)).unwrap(), false);
        assert_eq!(validate_resume_response(50, StatusCode::OK, None, Some(100)).unwrap(), true);
        assert!(validate_resume_response(50, StatusCode::PARTIAL_CONTENT, Some("bytes 0-49/100"), Some(100)).is_err());
        assert!(validate_resume_response(50, StatusCode::PARTIAL_CONTENT, Some("bytes 50-99/200"), Some(100)).is_err());
        assert!(validate_resume_response(50, StatusCode::PARTIAL_CONTENT, None, Some(100)).is_err());
    }

    #[test] fn public_requests_never_send_google_login_tokens() {
        let client = Client::new();
        let access = Access { api_key: "test-public-api-key".into() };
        let request = access.request(&client, DRIVE_BASE).build().unwrap();
        assert!(request.headers().get(header::AUTHORIZATION).is_none());
        assert!(request.url().query_pairs().any(|(k,v)| k == "key" && v == "test-public-api-key"));
    }
    #[tokio::test]
    async fn missing_api_key_fails_without_login_attempt() {
        let result = resolve_access(&Client::new(), &Preferences::default(), "validFolderId99", &None).await;
        assert!(result.is_err());
        assert!(result.err().unwrap().contains("API key"));
    }
    #[test] fn legacy_preferences_ignore_oauth_settings() {
        let parsed: Preferences = serde_json::from_str(r#"{"apiKey":"test-key","clientId":"legacy-client","saveDirectory":"/tmp","maxParallel":2}"#).unwrap();
        assert_eq!(parsed.api_key, "test-key");
        assert_eq!(parsed.max_parallel, 2);
        assert!(!serde_json::to_string(&parsed).unwrap().contains("clientId"));
    }
    #[test] fn retryable_errors_are_classified() {
        assert!(retryable_transfer_error("Network stalled for 120 seconds"));
        assert!(retryable_transfer_error("Google Drive HTTP 429: rateLimitExceeded"));
        assert!(retryable_transfer_error("Google Drive HTTP 403: userRateLimitExceeded"));
        assert!(!retryable_transfer_error("Google Drive HTTP 403: insufficientFilePermissions"));
        assert!(!retryable_transfer_error("Google Drive HTTP 404: not found"));
    }
}
