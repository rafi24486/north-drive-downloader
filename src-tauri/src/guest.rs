use super::*;
use scraper::{Html, Selector};
use url::Url;

const FOLDER_VIEW: &str = "https://drive.google.com/embeddedfolderview";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/125 Safari/537.36";
#[derive(Clone)]
struct GuestEntry { id:String, name:String, mime:String, folder:bool, key:Option<String> }

fn google_host(url:&Url)->bool {
    let h=url.host_str().unwrap_or("");
    url.scheme()=="https" && (h=="drive.google.com" || h=="docs.google.com" ||
        h=="drive.usercontent.google.com" || h=="drive.googleusercontent.com" ||
        h=="googleusercontent.com" || h.ends_with(".googleusercontent.com"))
}
fn child_id(u:&Url, marker:&str)->Option<String>{
    let parts:Vec<_>=u.path_segments()?.collect();
    let n=parts.iter().position(|s|*s==marker)?;
    let id=*parts.get(n+1)?;
    if valid_id(id){Some(id.into())}else{None}
}
fn entry_from_link(href:&str,title:&str)->Option<GuestEntry>{
    let u=Url::parse(href).ok()?;
    if !google_host(&u){return None;}
    let host=u.host_str()?;
    let name=sanitized_name(title);
    if host=="drive.google.com" {
        if let Some(id)=child_id(&u,"folders"){
            return Some(GuestEntry{id,name,mime:FOLDER_MIME.into(),folder:true,
                key:u.query_pairs().find(|(k,_)|k=="resourcekey").map(|(_,v)|v.into_owned())});
        }
        if let Some(id)=child_id(&u,"d"){
            return Some(GuestEntry{id,name,mime:"application/octet-stream".into(),folder:false,key:None});
        }
    }
    if host=="docs.google.com" {
        let kind=u.path_segments()?.next()?;
        let (mime,ext)=match kind{
            "document"=>("application/vnd.google-apps.document",".docx"),
            "spreadsheets"=>("application/vnd.google-apps.spreadsheet",".xlsx"),
            "presentation"=>("application/vnd.google-apps.presentation",".pptx"),
            "drawings"=>("application/vnd.google-apps.drawing",".pdf"),
            _=>return None,
        };
        let id=child_id(&u,"d")?;
        let name=if name.to_ascii_lowercase().ends_with(ext){name}else{format!("{name}{ext}")};
        return Some(GuestEntry{id,name,mime:mime.into(),folder:false,key:None});
    }
    None
}
fn parse_page(html:&str)->Result<(String,Vec<GuestEntry>),String>{
    let doc=Html::parse_document(html);
    let title_selector=Selector::parse("title").map_err(|e|e.to_string())?;
    let title=doc.select(&title_selector).next().map(|n|n.text().collect::<String>()).unwrap_or_default();
    let lower=title.to_ascii_lowercase();
    if title.trim().is_empty() || lower.contains("sign in") || lower.contains("accounts.google.com"){
        return Err("Google will not show this folder to guests. Set General access to Anyone with the link.".into());
    }
    let links=Selector::parse("a[href]").map_err(|e|e.to_string())?;
    let mut entries=Vec::new();let mut seen=HashSet::new();
    for anchor in doc.select(&links){
        let Some(href)=anchor.value().attr("href") else{continue};
        let label=anchor.text().collect::<Vec<_>>().join(" ");
        let label=label.trim();
        if label.is_empty(){continue;}
        if let Some(e)=entry_from_link(href,label){if seen.insert(e.id.clone()){entries.push(e);}}
    }
    let name=title.trim().trim_end_matches(" - Google Drive");
    Ok((sanitized_name(name),entries))
}
fn guest_client()->Result<Client,String>{
    Client::builder().user_agent(USER_AGENT).connect_timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|attempt|{
            if attempt.previous().len()>8{return attempt.error("Too many redirects");}
            if google_host(attempt.url()){attempt.follow()}else{attempt.error("Google download redirected to an untrusted host")}
        })).build().map_err(|e|e.to_string())
}
async fn scan_one(client:&Client,id:&str,key:Option<&str>)->Result<(String,Vec<GuestEntry>),String>{
    let mut req=client.get(FOLDER_VIEW).query(&[("id",id)]);
    if let Some(key)=key{req=req.query(&[("resourcekey",key)]);}
    let res=req.send().await.map_err(|e|format!("Unable to open public folder: {e}"))?;
    if !res.status().is_success(){return Err(format!("Guest folder access failed (HTTP {}). Confirm anyone with the link can view it.",res.status()));}
    let text=res.text().await.map_err(|e|e.to_string())?;
    if text.len()>8_000_000{return Err("Google returned an unusually large folder page".into());}
    parse_page(&text)
}
pub(super) async fn scan_folder(link:&str)->Result<FolderPlan,String>{
    let (id,key)=parse_drive_link(link)?;
    let client=guest_client()?;
    let mut pending=VecDeque::from([(id.clone(),String::new(),key.clone())]);
    let mut visited=HashSet::new();let mut files=Vec::new();let mut empty=Vec::new();
    let mut name=String::new();
    while let Some((folder,path,resource_key))=pending.pop_front(){
        if !visited.insert(folder.clone()){continue;}
        if visited.len()>1500{return Err("Too many subfolders for Guest Edition".into());}
        let (folder_name,children)=scan_one(&client,&folder,resource_key.as_deref()).await?;
        if name.is_empty(){name=folder_name;}
        if children.is_empty() && !path.is_empty(){empty.push(path.clone());}
        let mut names=HashSet::new();
        for child in children{
            let leaf=unique_name(&child.name,&mut names);
            let relative=if path.is_empty(){leaf.clone()}else{format!("{path}/{leaf}")};
            if child.folder{pending.push_back((child.id,relative,child.key));}
            else{
                if files.len()>=10000{return Err("Guest Edition supports up to 10,000 files per folder".into());}
                files.push(DownloadItem{id:child.id,name:leaf,relative_path:relative,size:None,
                    mime_type:child.mime,md5_checksum:None,resource_key:child.key,export_mime:None});
            }
        }
    }
    Ok(FolderPlan{folder_id:id,folder_name:name,files,empty_folders:empty,total_bytes:0,
        warnings:vec!["Public guest mode cannot inspect file sizes or calculate Google-provided checksums. Completed downloads are not cryptographically verified.".into()],
        access_method:"guest".into(),folder_resource_key:key})
}
fn staged_paths(root:&Path,rel:&Path)->(PathBuf,PathBuf){
    // Staging inside a hidden folder keeps Finder free of unplayable temporary MP4s.
    // The finalized file is moved into the visible folder after successful transfer.
    part_paths(&root.join(".northdrive-partials").join(rel))
}
async fn migrate_legacy_parts(target:&Path,part:&Path,meta:&Path)->Result<(),String>{
    if part.exists(){return Ok(());}
    let (old_part,old_meta)=part_paths(target);
    if !old_part.exists(){return Ok(());}
    if let Some(parent)=part.parent(){async_fs::create_dir_all(parent).await.map_err(|e|e.to_string())?;}
    if old_meta.exists() && !meta.exists(){
        async_fs::rename(&old_meta,meta).await.map_err(|e|format!("Could not move old resume metadata: {e}"))?;
    }
    if !meta.exists(){
        return Err(format!("Earlier temporary download has no resume metadata: {}. Keep it as-is, or move it out before starting over.",old_part.display()));
    }
    async_fs::rename(&old_part,part).await.map_err(|e|format!("Could not move earlier partial download: {e}"))?;
    Ok(())
}
fn looks_like_error_page(bytes:&[u8])->bool{
    let head=String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    let s=head.trim_start_matches(|c:char|c.is_whitespace() || c=='\u{feff}').to_ascii_lowercase();
    s.starts_with("<!doctype html") || s.starts_with("<html") || s.starts_with("<?xml")
        || s.starts_with("{\"error\"") || s.starts_with("{\"code\"")
}
fn file_url(f:&DownloadItem)->Result<Url,String>{
    let id=&f.id;
    let raw=match f.mime_type.as_str(){
        "application/vnd.google-apps.document"=>format!("https://docs.google.com/document/d/{id}/export?format=docx"),
        "application/vnd.google-apps.spreadsheet"=>format!("https://docs.google.com/spreadsheets/d/{id}/export?format=xlsx"),
        "application/vnd.google-apps.presentation"=>format!("https://docs.google.com/presentation/d/{id}/export?format=pptx"),
        "application/vnd.google-apps.drawing"=>format!("https://docs.google.com/drawings/d/{id}/export/pdf"),
        _=>format!("https://drive.google.com/uc?export=download&id={id}"),
    };
    Url::parse(&raw).map_err(|e|e.to_string())
}
fn next_confirm(html:&str,base:&Url)->Result<Url,String>{
    let doc=Html::parse_document(html);
    let form=Selector::parse("form#download-form,form[action*='download']").map_err(|e|e.to_string())?;
    for el in doc.select(&form){
        if let Some(action)=el.value().attr("action"){
            let mut url=base.join(action).map_err(|e|e.to_string())?;
            if !google_host(&url){return Err("Google returned an untrusted download URL".into());}
            let inputs=Selector::parse("input[name]").map_err(|e|e.to_string())?;
            {
                let mut query=url.query_pairs_mut();
                for inp in el.select(&inputs){
                    if let (Some(k),Some(v))=(inp.value().attr("name"),inp.value().attr("value")){
                        query.append_pair(k,v);
                    }
                }
            }
            return Ok(url);
        }
    }
    let links=Selector::parse("a[href]").map_err(|e|e.to_string())?;
    for el in doc.select(&links){
        if let Some(href)=el.value().attr("href"){
            if href.contains("export=download") || href.contains("confirm="){
                if let Ok(url)=base.join(href){if google_host(&url){return Ok(url);}}
            }
        }
    }
    if let Some(pos)=html.find("\"downloadUrl\":\""){
        let text=&html[pos+15..];
        if let Some(end)=text.find('"'){
            let decoded=text[..end].replace("\\u003d","=").replace("\\u0026","&").replace("\\/","/");
            if let Ok(url)=Url::parse(&decoded){if google_host(&url){return Ok(url);}}
        }
    }
    Err("Google returned a webpage instead of the download. This file may be restricted, rate-limited, or require unsupported confirmation.".into())
}
async fn transfer_attempt(app:&AppHandle,client:&Client,file:&DownloadItem,part:&Path,run:&RunControl)->Result<(),String>{
    let offset=fs::metadata(part).map(|m|m.len()).unwrap_or(0);
    let export=file.mime_type.starts_with("application/vnd.google-apps.");
    if export && offset>0{return Err("Google documents do not support partial resume. Remove the .northpart file and retry.".into());}
    let mut url=file_url(file)?;
    let mut cookie=String::new();
    let mut download=None;
    for _ in 0..4 {
        wait_for_run(run).await?;
        if !google_host(&url){return Err("Untrusted download host".into());}
        let mut req=client.get(url.clone());
        if offset>0{req=req.header(header::RANGE,format!("bytes={offset}-"));}
        if !cookie.is_empty()&&url.host_str()==Some("drive.google.com"){req=req.header(header::COOKIE,&cookie);}
        let resp=req.send().await.map_err(|e|e.to_string())?;
        if !resp.status().is_success(){return Err(format!("Google download failed: HTTP {}",resp.status()));}
        let content_type=resp.headers().get(header::CONTENT_TYPE).and_then(|v|v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        if !content_type.contains("text/html"){
            let media_name=file.name.to_ascii_lowercase();
            let media=media_name.ends_with(".mp4") || media_name.ends_with(".mov") || media_name.ends_with(".mxf")
                || media_name.ends_with(".mkv") || media_name.ends_with(".avi");
            if media && (content_type.starts_with("text/") || content_type.contains("json") || content_type.contains("xml")){
                return Err(format!("Google returned {} rather than video data. Check sharing permissions or download limits.",content_type));
            }
            if offset>0 {
                if resp.status()!=StatusCode::PARTIAL_CONTENT{return Err("Google refused to resume partial file. Remove its .northpart files to restart.".into());}
                let header_range=resp.headers().get(header::CONTENT_RANGE).and_then(|v|v.to_str().ok()).unwrap_or("");
                if !header_range.starts_with(&format!("bytes {offset}-")){return Err("Invalid resumed byte range".into());}
            }
            download=Some(resp);break;
        }
        if offset>0{return Err("Google showed a confirmation page for a partial file; remove .northpart files to restart.".into());}
        cookie=resp.headers().get_all(header::SET_COOKIE).iter()
            .filter_map(|v|v.to_str().ok()).filter_map(|v|v.split(';').next())
            .take(12).collect::<Vec<_>>().join("; ");
        let base=resp.url().clone();
        let html=resp.text().await.map_err(|e|e.to_string())?;
        if html.len()>2_000_000{return Err("Google returned an oversized HTML page, not a file".into());}
        url=next_confirm(&html,&base)?;
    }
    let mut response=download.ok_or("Google did not return a downloadable file")?;
    let expected_segment=response.content_length();
    let mut output=async_fs::OpenOptions::new().create(true).append(true).open(part).await.map_err(|e|e.to_string())?;
    let mut bytes=offset;let start=Instant::now();let mut last=Instant::now();
    let mut first_chunk=offset==0;
    progress(app,file,bytes,"downloading",0.0,None);
    loop{
        wait_for_run(run).await?;
        let next=tokio::time::timeout(Duration::from_secs(90),response.chunk()).await
            .map_err(|_|"Transfer stalled for 90 seconds. It will retry and resume where possible.".to_string())?
            .map_err(|e|e.to_string())?;
        let Some(chunk)=next else{break};
        if first_chunk{
            first_chunk=false;
            if looks_like_error_page(&chunk){
                return Err("Google returned an HTML or JSON error page instead of a downloadable file.".into());
            }
        }
        output.write_all(&chunk).await.map_err(|e|e.to_string())?;
        bytes+=chunk.len() as u64;
        if last.elapsed()>Duration::from_millis(350){
            let speed=(bytes-offset) as f64/start.elapsed().as_secs_f64().max(0.01);
            progress(app,file,bytes,"downloading",speed,None);
            last=Instant::now();
        }
    }
    output.flush().await.map_err(|e|e.to_string())?;
    if bytes==0{return Err("Google returned an empty file".into());}
    if let Some(expected)=expected_segment{
        let received=bytes-offset;
        if received!=expected{
            return Err(format!("Incomplete Google response: received {received} of {expected} bytes. Retrying remaining bytes."));
        }
    }
    Ok(())
}
async fn download_one(app:&AppHandle,client:&Client,file:&DownloadItem,root:&Path,run:&RunControl)->Result<(),String>{
    let rel=safe_relative(&file.relative_path)?;
    let target=root.join(&rel);
    if let Some(parent)=target.parent(){async_fs::create_dir_all(parent).await.map_err(|e|e.to_string())?;}
    if target.exists(){return Err(format!("Destination already exists: {}. North Drive never overwrites it.",target.display()));}
    let (part,meta)=staged_paths(root,&rel);
    if let Some(parent)=part.parent(){async_fs::create_dir_all(parent).await.map_err(|e|e.to_string())?;}
    migrate_legacy_parts(&target,&part,&meta).await?;
    let marker=serde_json::to_vec(&serde_json::json!({"id":file.id,"mime":file.mime_type})).map_err(|e|e.to_string())?;
    if part.exists() && fs::read(&meta).ok().as_deref()!=Some(marker.as_slice()){
        return Err("A partial download has different source metadata. Remove .northpart files to retry.".into());
    }
    if !part.exists(){async_fs::write(&meta,&marker).await.map_err(|e|e.to_string())?;}
    let mut error=String::new();
    for attempt in 0..4{
        wait_for_run(run).await?;
        match transfer_attempt(app,client,file,&part,run).await{
            Ok(())=>{
                if target.exists(){return Err("Target file already exists; refusing overwrite".into());}
                async_fs::rename(&part,&target).await.map_err(|e|e.to_string())?;
                let _=async_fs::remove_file(&meta).await;
                let bytes=fs::metadata(&target).map(|m|m.len()).unwrap_or(0);
                progress(app,file,bytes,"completed",0.0,None);return Ok(());
            }
            Err(e)=>{error=e;if attempt<3{
                progress(app,file,fs::metadata(&part).map(|m|m.len()).unwrap_or(0),"retrying",0.0,Some(error.clone()));
                sleep(Duration::from_secs(1<<(attempt+1))).await;
            }}
        }
    }
    Err(format!("{error} (4 attempts)"))
}
pub(super) async fn download_folder(app:&AppHandle,plan:&FolderPlan,destination:&Path,run:&Arc<RunControl>)->RunSummary{
    if plan.access_method!="guest"{return RunSummary{completed:0,failed:plan.files.len(),cancelled:false,errors:vec!["Only public guest folders are supported".into()]};}
    let root=destination.join(sanitized_name(&plan.folder_name));
    let mut errors=Vec::new();
    if let Err(e)=async_fs::create_dir_all(&root).await{return RunSummary{completed:0,failed:plan.files.len(),cancelled:false,errors:vec![e.to_string()]};}
    for path in &plan.empty_folders{
        match safe_relative(path){
            Ok(p)=>if let Err(e)=async_fs::create_dir_all(root.join(p)).await{errors.push(e.to_string());},
            Err(e)=>errors.push(e),
        }
    }
    let client=match guest_client(){Ok(c)=>c,Err(e)=>return RunSummary{completed:0,failed:plan.files.len(),cancelled:false,errors:vec![e]}};
    let mut tasks=tokio::task::JoinSet::new();let mut completed=0;let mut failed=0;
    for file in plan.files.clone(){
        if run.cancelled.load(Ordering::SeqCst){break;}
        while tasks.len()>=4 {
            if let Some(job)=tasks.join_next().await {
                match job{Ok(Ok(()))=>completed+=1,Ok(Err(e))=>{failed+=1;errors.push(e)},Err(e)=>{failed+=1;errors.push(e.to_string())}}
            }
        }
        let app=app.clone();let client=client.clone();let root=root.clone();let run=run.clone();
        tasks.spawn(async move{
            let result=download_one(&app,&client,&file,&root,&run).await;
            if let Err(ref e)=result{
                let downloaded=safe_relative(&file.relative_path).ok()
                    .map(|rel|staged_paths(&root,&rel).0)
                    .and_then(|path|fs::metadata(path).ok())
                    .map(|m|m.len()).unwrap_or(0);
                progress(&app,&file,downloaded,"failed",0.0,Some(e.clone()));
            }
            result.map_err(|e|format!("{}: {e}",file.relative_path))
        });
    }
    while let Some(job)=tasks.join_next().await {
        match job{Ok(Ok(()))=>completed+=1,Ok(Err(e))=>{failed+=1;errors.push(e)},Err(e)=>{failed+=1;errors.push(e.to_string())}}
    }
    RunSummary{completed,failed,cancelled:run.cancelled.load(Ordering::SeqCst),errors}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]fn parse_fixture(){
        let html=r#"<html><head><title>Client Assets - Google Drive</title></head><body>
        <a href="https://drive.google.com/file/d/1abcdefghijklmnopqrstuvwxyz123/view">A.mov</a>
        <a href="https://drive.google.com/drive/folders/1abcdefghijklmnopqrstuvwxyz124">Footage</a></body></html>"#;
        let (name,items)=parse_page(html).unwrap();
        assert_eq!(name,"Client Assets");assert_eq!(items.len(),2);assert!(items[1].folder);
    }
    #[test]fn sniff_fake_video_pages(){
        assert!(looks_like_error_page(b"<!DOCTYPE html><html>Drive quota exceeded</html>"));
        assert!(looks_like_error_page(b" {\"error\":\"notFound\"}"));
        assert!(!looks_like_error_page(b"\0\0\0\x18ftypisom\0\0\0\0"));
    }
    #[test]fn temporary_files_are_hidden(){
        let root=Path::new("/tmp/Client Assets");
        let rel=Path::new("CLIPS/C1456.MP4");
        let (part,meta)=staged_paths(root,rel);
        assert!(part.to_string_lossy().contains(".northdrive-partials/CLIPS/C1456.MP4.northpart"));
        assert!(meta.to_string_lossy().ends_with(".northpart.json"));
    }
    #[test]fn block_untrusted_hosts(){
        let u=Url::parse("https://attacker.example/drive/folders/1abcdefghijklmnopqrstuvwxyz123").unwrap();
        assert!(!google_host(&u));
    }
}
