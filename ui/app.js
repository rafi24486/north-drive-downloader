(() => {
  'use strict';
  const tauri = !!window.__TAURI__?.core?.invoke;
  const invoke = (command, args = {}) => window.__TAURI__.core.invoke(command, args);
  const $ = (s) => document.querySelector(s);
  const $$ = (s) => [...document.querySelectorAll(s)];
  const IC = {
    home:'<path d="m3 10 9-7 9 7v10a1 1 0 0 1-1 1h-5v-7H9v7H4a1 1 0 0 1-1-1z"/>',
    download:'<path d="M12 3v12m-5-5 5 5 5-5M4 17v3h16v-3"/>',
    check:'<rect x="3" y="3" width="18" height="18" rx="4"/><path d="m8 12 3 3 5-6"/>',
    users:'<circle cx="12" cy="8" r="4"/><path d="M4 21v-2a8 8 0 0 1 16 0v2"/>',
    settings:'<path d="M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.9l.06.06-1.9 1.9-.06-.06A1.7 1.7 0 0 0 16 18.5a1.7 1.7 0 0 0-1 1.6V21h-3v-.9a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.34l-.06.06-1.9-1.9.06-.06A1.7 1.7 0 0 0 7.5 15a1.7 1.7 0 0 0-1.6-1H5v-3h.9a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.34-1.9L7.1 8 9 6.1l.06.06A1.7 1.7 0 0 0 11 6.5 1.7 1.7 0 0 0 12 4.9V4h3v.9a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.34l.06-.06 1.9 1.9-.06.06A1.7 1.7 0 0 0 19.5 10a1.7 1.7 0 0 0 1.6 1h.9v3h-.9a1.7 1.7 0 0 0-1.7 1z" transform="translate(-1.5 -1.5) scale(1.12)"/>',
    shield:'<path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/><path d="m9 12 2 2 4-4"/>',
    link:'<path d="M10 13a5 5 0 0 0 7.1 0l3-3A5 5 0 0 0 13 2.9l-1.7 1.7"/><path d="M14 11a5 5 0 0 0-7.1 0l-3 3A5 5 0 0 0 11 21.1l1.7-1.7"/>',
    folder:'<path d="M3 6a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
    folders:'<path d="M2 7h7l2 3h9v9H2z"/><path d="M5 4h8l2 3h6v10"/>',
    'chevron-down':'<path d="m6 9 6 6 6-6"/>',
    scan:'<path d="M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3M7 12h10"/>',
    zap:'<path d="m13 2-9 12h7l-1 8 10-13h-7z"/>',
    infinity:'<path d="M12 12c-4-6-10-6-10 0s6 6 10 0c4-6 10-6 10 0s-6 6-10 0z"/>',
    'cloud-download':'<path d="M20 17a4 4 0 0 0-2-7.5A7 7 0 0 0 5 9a5 5 0 0 0 0 10h4"/><path d="M12 12v9m-4-4 4 4 4-4"/>',
    activity:'<path d="M3 12h4l3-7 4 14 3-7h4"/>',
    'hard-drive':'<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M3 14h18M17 16.5h.01"/>',
    files:'<path d="M7 3H4a1 1 0 0 0-1 1v15h12"/><path d="M9 2h8l4 4v15H9zM17 2v5h4"/>',
    sparkles:'<path d="m12 3 2 6 6 2-6 2-2 6-2-6-6-2 6-2z"/><path d="m19 17 .8 2.2L22 20l-2.2.8L19 23l-.8-2.2L16 20l2.2-.8z"/>',
    search:'<circle cx="11" cy="11" r="7"/><path d="m16 16 5 5"/>',
    pause:'<path d="M8 5v14M16 5v14"/>',
    play:'<path d="m8 5 11 7-11 7z"/>',
    stop:'<rect x="5" y="5" width="14" height="14" rx="2"/>',
    file:'<path d="M5 2h10l5 5v15H5zM15 2v6h5"/>',
    video:'<rect x="3" y="5" width="18" height="14" rx="3"/><path d="m10 9 5 3-5 3z"/>',
    music:'<path d="M9 18V5l12-2v13M9 9l12-2"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>'
  };
  const icon = (name) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${IC[name] || IC.file}</svg>`;
  $$('[data-icon]').forEach(el => { el.innerHTML = icon(el.dataset.icon); });
  const escapeHtml = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const formatBytes = bytes => { if (bytes === null || bytes === undefined) return '—'; if(bytes < 1024) return `${bytes} B`; const units=['KB','MB','GB','TB','PB']; let n=bytes/1024, i=0; while(n>=1024&&i<units.length-1){n/=1024;i++;} return `${n>=100?n.toFixed(0):n>=10?n.toFixed(1):n.toFixed(2)} ${units[i]}`; };
  const formatSpeed = b => b >= 1048576 ? `${(b/1048576).toFixed(1)} MB/s` : `${(b/1024).toFixed(0)} KB/s`;
  const state = { plan: null, progress: {}, running:false, paused:false, currentFilter:'all', connected:false, activePage:'home', speedHistory:Array(40).fill(0), demo:!tauri, preferences:{maxParallel:4,saveDirectory:''}, summary:null, managedOAuth:false, profile:null, profileError:'' };
  let toastTimer;
  function toast(text, error=false) {const el=$('#toast');el.textContent=text;el.classList.toggle('error',error);el.classList.add('show');clearTimeout(toastTimer);toastTimer=setTimeout(()=>el.classList.remove('show'),4600);}
  function goPage(page) {if(!['home','downloads','completed','accounts','settings'].includes(page))return; state.activePage=page;$$('.page').forEach(el=>el.classList.toggle('visible',el.id===`page-${page}`));$$('.nav-item').forEach(el=>el.classList.toggle('active',el.dataset.page===page));$('#crumbName').textContent=({home:'Overview',downloads:'Downloads',completed:'Completed',accounts:'Accounts',settings:'Settings'})[page];if(page==='downloads'||page==='completed')renderFiles();}
  $$('[data-page]').forEach(el=>el.addEventListener('click',()=>goPage(el.dataset.page)));
  // macOS menu-style shortcuts; keep text-editing shortcuts untouched.
  document.addEventListener('keydown', e => {
    if (!e.metaKey || e.altKey || e.ctrlKey) return;
    const page = ({'1':'home','2':'downloads','3':'completed',',':'settings'})[e.key];
    if (page) { e.preventDefault(); goPage(page); }
  });
  $('#settingsIcon').addEventListener('click',()=>goPage('settings'));
  $('#accountBtn').addEventListener('click',()=>toast('Guest mode: public links only'));
  $('#sidebarAccountBtn').addEventListener('click',()=>toast('Guest mode: no login required'));
  $('#topAccountBtn').addEventListener('click',()=>toast('Guest mode: no login required'));

  $('#parallel').addEventListener('input',ev=>$('#parallelValue').textContent=`${ev.target.value} files`);
  function hydratePrefs() {$('#parallel').value=state.preferences.maxParallel||4;$('#parallelValue').textContent=`${$('#parallel').value} files`;$('#savePath').value=state.preferences.saveDirectory||'';}
  // Treat Google sub as the stable internal ID; display name and photo are presentation only.
  function profileDisplayName(){
    return state.profile?.name || state.profile?.email?.split('@')[0] || 'Google account';
  }
  function accountAvatar(selector, fallback) {
    const el=$(selector); if(!el)return;
    const name=profileDisplayName();
    const initial=state.profile?.name?.trim().charAt(0).toUpperCase() || fallback;
    el.replaceChildren();el.textContent=initial;
    const picture=state.profile?.picture;
    if(!picture)return;
    try{
      const u=new URL(picture);
      if(u.protocol!=='https:' || !(u.hostname==='googleusercontent.com' || u.hostname.endsWith('.googleusercontent.com')))return;
      const img=document.createElement('img');
      img.alt='';
      img.referrerPolicy='no-referrer';
      img.loading='lazy';
      img.addEventListener('error',()=>{el.replaceChildren();el.textContent=initial},{once:true});
      img.src=u.href;
      el.replaceChildren(img);
    }catch(e){/* fallback initials */}
  }
  function showConnectionIssue(error) {
    state.profileError=error?String(error):'';
    const el=$('#connectionIssue');
    el.hidden=!state.profileError;
    el.textContent=state.profileError;
  }
  function updateAccount(){
    const name=profileDisplayName();
    const signedIn=!!state.connected;
    const label=signedIn?(state.profile?.name||'Connected'):'Not connected';
    $('#accountStatus').textContent=label;
    $('#accountPageStatus').textContent=signedIn?(state.profile?.email||'Signed in to Google'):'Not connected';
    $('#accountsName').textContent=signedIn?name:'Google Drive';
    $('#sidebarAccountName').textContent=signedIn?name:'Not connected';
    $('#sidebarAccountEmail').textContent=signedIn?(state.profile?.email||'Google account'):'Continue with Google';
    $('#selectedAccount').textContent=signedIn?name:'Connect Google';
    $('.status-dot').classList.toggle('connected',signedIn);
    $('#sidebarAccountDot').classList.toggle('connected',signedIn);
    $('#connectBtn').style.display=signedIn?'none':'';
    $('#disconnectBtn').style.display=signedIn?'':'none';
    $('#connectBtn').disabled=!state.managedOAuth;
    $('#accountHelp').textContent=!state.managedOAuth?'Google sign-in has not been enabled for this build. Contact the app publisher.':signedIn?'Google sign-in has been verified against the Drive API. Your tokens are stored in macOS Keychain.':'Sign in with your existing Google account. We never ask for your Google password.';
    $('#diagnoseBtn').style.display=signedIn?'inline-flex':'none';
    showConnectionIssue(state.profileError);
    for(const [selector,fallback] of [['#sidebarAvatar','N'],['#topAccountBtn','N'],['#selectorAvatar','G'],['#accountsAvatar','G']])accountAvatar(selector,fallback);
  }
  function updateGuest(){
    $('#accountStatus').textContent='Guest mode';
    $('#selectedAccount').textContent='Guest · Public links';
    $('#sidebarAccountName').textContent='Guest mode';
    $('#sidebarAccountEmail').textContent='No sign-in required';
  }
  async function savePrefs(){
    const prefs={maxParallel:Number($('#parallel').value),saveDirectory:$('#savePath').value.trim()};
    if(tauri){try{await invoke('set_preferences',{preferences:prefs});state.preferences=prefs;toast('Settings saved');}catch(e){toast(String(e),true)}}
    else{state.preferences=prefs;toast('Settings saved in interface preview only');}
  }
  $('#saveSettingsBtn').addEventListener('click',savePrefs);
  $('#connectBtn').addEventListener('click',async()=>{
    if(!tauri){toast('Google sign-in is available in the macOS app',true);return;}
    if(!state.managedOAuth){toast('Google sign-in is not configured by the publisher yet',true);return;}
    $('#connectBtn').disabled=true;$('#connectBtn').textContent='Opening browser…';
    try{
      const profile=await invoke('connect_google');
      state.profile=profile;state.profileError='';state.connected=true;
      updateAccount();toast('Signed in as '+profileDisplayName());
    }catch(e){showConnectionIssue(e);goPage('accounts');toast('Google sign-in failed — see the error below',true)}
    finally{$('#connectBtn').textContent='Continue with Google';updateAccount();}
  });
  $('#diagnoseBtn').addEventListener('click',async()=>{
    if(!tauri)return;
    const btn=$('#diagnoseBtn');btn.disabled=true;btn.textContent='Checking Google…';
    try{
      state.profile=await invoke('get_google_profile');
      state.connected=true;state.profileError='';
      updateAccount();toast('Google account and Drive API are working');
    }catch(e){
      state.connected=false;state.profile=null;
      showConnectionIssue(e);updateAccount();
      toast('Google connection failed — reconnect your account',true);
    }finally{btn.disabled=false;btn.textContent='Check Google connection';}
  });
  $('#disconnectBtn').addEventListener('click',async()=>{
    if(!tauri){toast('Preview mode only');return;}
    try{
      await invoke('disconnect');state.connected=false;state.profile=null;state.profileError='';
      updateAccount();toast('Google disconnected');
    }catch(e){toast(String(e),true)}
  });
  $('#browseBtn').addEventListener('click',async()=>{if(!tauri){toast('Folder picker is available in the compiled macOS app');return;}try{const selected=await window.__TAURI__.dialog.open({directory:true,multiple:false,title:'Choose download folder'});if(typeof selected==='string'){$('#savePath').value=selected;const prefs={...state.preferences,saveDirectory:selected};await invoke('set_preferences',{preferences:prefs});state.preferences=prefs;}}catch(e){toast(String(e),true)}});
  $('#linkInput').addEventListener('keydown',e=>{if(e.key==='Enter')analyze()});
  $('#analyzeBtn').addEventListener('click',analyze);
  async function analyze(){const link=$('#linkInput').value.trim();if(!/https:\/\/(drive|docs)\.google\.com\//.test(link)){toast('Paste a valid Google Drive folder link',true);return;}const btn=$('#analyzeBtn');btn.disabled=true;btn.textContent='Scanning folder…';try{let plan;if(tauri){plan=await invoke('scan_folder',{link})}else{await new Promise(res=>setTimeout(res,700));plan=demoPlan()}state.plan=plan;state.progress={};state.summary=null;state.running=false;for(const f of plan.files) state.progress[f.id]={bytes:0,status:'queued',speed:0};render();goPage('downloads');toast(`${plan.files.length} file${plan.files.length===1?'':'s'} discovered • ${formatBytes(plan.totalBytes)}${plan.warnings?.length?` • ${plan.warnings.length} warnings`:''}`)}catch(e){toast(String(e),true)}finally{btn.disabled=false;btn.innerHTML=`${icon('scan')} Analyze folder <span class="arrow">→</span>`}}
  function demoPlan(){const entries=[['A001_C001.MOV',128*1024**3/10],['A001_C002.MOV',84*1024**3/10],['B_Cam_Lifestyle.mp4',42*1024**3/10],['Drone_Footage.mov',166*1024**3/10],['Audio_Master.wav',21*1024**3/10],['Graphics/Logo_Animation.mov',67*1024**3/10],['Raw_Footage/Camera_A/Interview_01.mov',72*1024**3],['Music/Track.wav',1.2*1024**3],['References/Lookbook.pdf',174*1024**2]];return{folderId:'demo-folder',folderName:'Client Project',files:entries.map(([path,size],i)=>({id:`demo-${i}`,name:path.split('/').at(-1),relativePath:path,size:Math.floor(size),mimeType:'application/octet-stream',md5Checksum:null,resourceKey:null,exportMime:null})),emptyFolders:[],totalBytes:entries.reduce((a,[,n])=>a+Math.floor(n),0),warnings:[],accessMethod:'public',folderResourceKey:null}}
  async function beginDownload(){if(!state.plan){toast('Analyze a folder first',true);return;}const destination=$('#savePath').value.trim();if(!destination&&!state.demo){toast('Choose a destination folder first',true);goPage('home');return;}if(state.running){toast('A download is already running');return;}try{if(tauri){await invoke('start_download',{plan:state.plan,directory:destination})}state.running=true;state.paused=false;state.summary=null;render();toast(state.demo?'Demo transfer preview started':'Download started');if(state.demo)startDemoProgress()}catch(e){toast(String(e),true)}}
  async function togglePause(){if(!state.running)return;const paused=!state.paused;if(tauri){try{await invoke('pause_download',{paused})}catch(e){toast(String(e),true);return}}state.paused=paused;render();}
  async function cancelDownload(){if(!state.running)return;if(tauri){try{await invoke('cancel_download')}catch(e){toast(String(e),true);return}}else{state.running=false;state.summary={cancelled:true,completed:0,failed:0,errors:[]}}state.paused=false;render();toast('Transfer cancellation requested; partial files are preserved')}
  function progressData(file){return state.progress[file.id]||{bytes:0,status:'queued',speed:0}}
  function bytesComplete(){return state.plan?.files.reduce((sum,f)=>sum+Math.min(progressData(f).bytes||0,f.size||Number.MAX_SAFE_INTEGER),0)||0}
  function completedCount(){return state.plan?.files.filter(f=>progressData(f).status==='completed').length||0}
  function failedCount(){return state.plan?.files.filter(f=>progressData(f).status==='failed').length||0}
  function activeCount(){return state.plan?.files.filter(f=>progressData(f).status==='downloading').length||0}
  function currentSpeed(){return state.plan?.files.reduce((sum,f)=>sum+(progressData(f).status==='downloading'?progressData(f).speed||0:0),0)||0}
  function overallPercent(){if(state.plan?.accessMethod==='guest'){const total=state.plan?.files.length||0;return total?Math.round(completedCount()/total*100):0;}return Math.min(100,Math.round(bytesComplete()/Math.max(state.plan?.totalBytes||1,1)*100));}
  function projectMarkup(){if(!state.plan)return `<div class="empty-large"><div class="empty-icon">${icon('cloud-download')}</div><h3>Ready when you are</h3><p>Paste a Drive folder link above to begin your next project.</p></div>`;const processed=completedCount();const total=state.plan.files.length;const p=overallPercent();const label=state.summary?.cancelled?'Cancelled':state.summary?'Finished':state.paused?'Paused':state.running?'Downloading':'Ready to download';return `<div class="project-card"><div class="project-icon">${icon('folder')}</div><div class="project-main"><h3>${escapeHtml(state.plan.folderName)}</h3><p>${state.plan.accessMethod==='guest'?`${formatBytes(bytesComplete())} received (total unknown)`:`${formatBytes(bytesComplete())} of ${formatBytes(state.plan.totalBytes)}`} <span style="color:#634f68"> · </span> ${processed} of ${total} files <span style="color:#634f68"> · </span> ${label}</p><div class="progress-track"><div class="progress-fill" style="width:${p}%"></div></div><div class="progress-info"><span>${state.plan.accessMethod==='guest'?'Guest · public folder':'Public link'} • ${state.plan.warnings?.length||0} warnings</span><span class="progress-percent">${p}%</span></div></div><div class="project-actions">${!state.running?`<button class="btn-primary" id="startProject" ${state.summary&&!state.summary.cancelled?'disabled':''}>${icon('download')} ${state.summary?'Finished':'Start'}</button>`:`<button class="round-action" id="pauseProject" title="${state.paused?'Resume':'Pause'}">${icon(state.paused?'play':'pause')}</button><button class="round-action" id="cancelProject" title="Cancel">${icon('stop')}</button>`}</div></div>`}
  function wireProjectControls(){['#startProject','#startProjectOther'].forEach(s=>$(s)?.addEventListener('click',beginDownload));['#pauseProject','#pauseProjectOther'].forEach(s=>$(s)?.addEventListener('click',togglePause));['#cancelProject','#cancelProjectOther'].forEach(s=>$(s)?.addEventListener('click',cancelDownload));}
  function renderProject(){$('#activeProject').innerHTML=projectMarkup();$('#projectSummary').innerHTML=state.plan?`<div class="active-project">${projectMarkup().replace('id="startProject"','id="startProjectOther"').replace('id="pauseProject"','id="pauseProjectOther"').replace('id="cancelProject"','id="cancelProjectOther"')}</div>`:'';wireProjectControls();}
  function renderFiles(){const files=state.plan?.files||[];$('#filesCount').textContent=files.length;$('#downloadCount').textContent=state.running?activeCount():files.length;
    const search=$('#fileSearch').value.toLowerCase();const visible=files.filter(f=>{const d=progressData(f);return(state.currentFilter==='all'||state.currentFilter===d.status||(state.currentFilter==='downloading'&&d.status==='retrying'))&&f.relativePath.toLowerCase().includes(search)});
    const html=visible.map(f=>{const d=progressData(f);const ratio=f.size?Math.min(100,Math.round(d.bytes/f.size*100)):d.status==='completed'?100:0;const progressLabel=f.size?`${ratio}%`:d.status==='completed'?'Done':d.bytes?formatBytes(d.bytes):'—';const type=/\.(mp4|mov|mxf|mkv|avi)$/i.test(f.relativePath)?'video':/\.(mp3|wav|aif|aiff)$/i.test(f.relativePath)?'music':'file';let status=d.status==='failed'?'Issue':d.status==='completed'?'Completed':d.status==='verifying'?'Verifying':d.status==='downloading'?'Downloading':d.status==='retrying'?'Retrying':'Queued';return `<div class="file-row"><div class="filename"><div class="file-icon">${icon(type)}</div><div class="name-data"><b title="${escapeHtml(f.relativePath)}">${escapeHtml(f.name)}</b><small title="${escapeHtml(f.relativePath)}">${escapeHtml(f.relativePath)}</small></div></div><div class="file-size">${formatBytes(f.size)}</div><div class="mini-progress"><div class="mini-track"><i style="width:${ratio}%"></i></div><small>${progressLabel}</small></div><div class="file-status ${escapeHtml(d.status)}" title="${escapeHtml(d.message||'')}">${status}</div></div>`}).join('');$('#fileRows').innerHTML=html||`<div class="files-empty">${files.length?'No matching files':'No files yet. Analyze a folder to get started.'}</div>`;
    const completed=files.filter(f=>progressData(f).status==='completed');$('#completedRows').innerHTML=completed.map(f=>`<div class="file-row"><div class="filename"><div class="file-icon">${icon('check')}</div><div class="name-data"><b>${escapeHtml(f.name)}</b><small>${escapeHtml(f.relativePath)}</small></div></div><div class="file-size">${formatBytes(f.size)}</div><div class="mini-progress"><div class="mini-track"><i style="width:100%"></i></div><small>100%</small></div><div class="file-status completed">Completed</div></div>`).join('')||'<div class="files-empty">Nothing completed yet.</div>';
  }
  const speedSeries=[];
  if (state.demo) {
    for(let i=0;i<39;i++) speedSeries.push(15 + 5*Math.sin(i*.33) + 2*Math.sin(i*.91));
  }
  function updateInsights(){const sp=currentSpeed();let value=sp/1048576;$('#speedCard').innerHTML=`${value.toFixed(1)} <span>MB/s</span>`;$('#totalTransferred').innerHTML=`${(bytesComplete()/1024**3).toFixed(2)} <small>GB</small>`;$('#transferSubtext').textContent=state.plan?(state.plan.accessMethod==='guest'?'Guest mode · total size unknown':`of ${formatBytes(state.plan.totalBytes)} total`):'No active transfer';$('#processedFiles').innerHTML=`${completedCount()} <small>/ ${state.plan?.files.length||0}</small>`;speedSeries.push(value);if(speedSeries.length>40)speedSeries.shift();const peak=Math.max(.1,...speedSeries);$('#speedPeak').textContent=`PEAK ${peak.toFixed(1)} MB/s`;const points=speedSeries.map((n,i)=>`${(i/39*256).toFixed(1)},${(66-60*n/peak).toFixed(1)}`);if(points.length>1){$('#chartLine').setAttribute('d','M'+points.join(' L'));$('#chartArea').setAttribute('d','M'+points.join(' L')+' L256,72 L0,72Z')}}
  function renderWarnings(){const warnings=state.plan?.warnings||[];$('#warningList').innerHTML=warnings.length?`<details class="warnings"><summary>${warnings.length} guest note${warnings.length===1?'':'s'} <span>Show details</span></summary><div>${warnings.map(w=>`<p>${escapeHtml(w)}</p>`).join('')}</div></details>`:'';}
  function render(){renderProject();renderWarnings();renderFiles();updateInsights()}
  let renderPending=false;
  function queueRender(){if(renderPending)return;renderPending=true;setTimeout(()=>{renderPending=false;render()},150)}
  $('#fileSearch').addEventListener('input',renderFiles);
  $$('.tab').forEach(tab=>tab.addEventListener('click',()=>{state.currentFilter=tab.dataset.filter;$$('.tab').forEach(t=>t.classList.toggle('active',t===tab));renderFiles()}));
  $('#clearViewBtn').addEventListener('click',()=>{if(state.running){toast('Finish or cancel the active download first',true);return;}state.plan=null;state.progress={};state.summary=null;render();toast('Local transfer view cleared. Downloaded files are untouched.')});
  let demoTimer;
  function startDemoProgress(){clearInterval(demoTimer);demoTimer=setInterval(()=>{if(!state.running){clearInterval(demoTimer);return}if(state.paused)return;let active=0;for(const f of state.plan.files){const d=state.progress[f.id];if(d.status==='completed')continue;if(active>=4)break;active++;d.status='downloading';const increment=Math.min(28*1048576,f.size-d.bytes);d.bytes+=increment;d.speed=increment*2;if(d.bytes>=f.size){d.status='completed';d.speed=0}}if(completedCount()===state.plan.files.length){state.running=false;state.summary={completed:completedCount(),failed:0,cancelled:false,errors:[]};toast('Demo download complete (no real files created)');clearInterval(demoTimer)}queueRender()},500)}
  async function initialize(){if(tauri){try{
    state.preferences=await invoke('get_preferences');
    state.connected=false;
    state.profile=null;
    state.managedOAuth=false;
    window.__TAURI__.event.listen('north:progress',event=>{const p=event.payload;if(!p?.id)return;state.progress[p.id]={bytes:p.bytes??0,status:p.status,speed:p.speed??0,message:p.message};queueRender()});window.__TAURI__.event.listen('north:summary',event=>{state.summary=event.payload;state.running=false;state.paused=false;render();if(state.summary.cancelled)toast('Transfer cancelled. Partial files kept.');else if(state.summary.failed)toast(`${state.summary.failed} file(s) failed. Inspect Issues for details.`,true);else toast(`Downloaded ${state.summary.completed} files successfully!`)});}catch(e){toast(`App initialization issue: ${e}`,true)}}else{$('#previewBanner').style.display='block';$('#linkInput').value='https://drive.google.com/drive/folders/DEMO-FOLDER';state.plan=demoPlan();for(const f of state.plan.files) state.progress[f.id]={bytes:0,status:'queued',speed:0};state.progress[state.plan.files[0].id]={bytes:Math.floor(state.plan.files[0].size*.42),status:'downloading',speed:22*1048576};state.progress[state.plan.files[2].id]={bytes:state.plan.files[2].size,status:'completed',speed:0};state.progress[state.plan.files[1].id]={bytes:Math.floor(state.plan.files[1].size*.14),status:'downloading',speed:8*1048576};state.running=true;$('#savePath').value='/Users/you/Downloads/Client Projects';}
    hydratePrefs();if(state.demo)$('#savePath').value='/Users/you/Downloads/Client Projects';updateGuest();render();setInterval(updateInsights,1500);}
  initialize();
})();
