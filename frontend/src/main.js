import './styles.css';
import rosarayIcon from './clarosa-ai-icon.png';
import {$,$$,ic,toast,hash} from './util.js';
import {dice} from './algo.js';
import {DEFS,dflt,exec} from './registry.js';
import {samples,getImg} from './samples.js';
import * as svc from './service_client.js';

// ---- state ----
let nodes=[],edges=[],selId=null,pending=null,uid=1;
let openTabs=[],activeId=null,mode='input',results=null,lastRun=null;
const runs=[];
// ---- local service (Rosaray Service) explorer state ----
let svcDatasets=[],svcDatasetId=null,svcVersions=[],svcVersionId=null,svcImages=[],svcThumbObserver=null;
let svcOpenReqId=0; // bumped per svcOpenImage() call so a slower, superseded request can never overwrite a newer selection (FR-039)
const svcThumbRequests=new Map(); // image_id -> in-flight request_id, for scroll-away cancellation
const svcThumbCache=new Map(); // image_id -> data URL, so re-rendering the list doesn't re-fetch
// ---- local service official Runs (User Story 4) ----
let svcRuns=[]; // GET /runs?dataset_version_id= results for the selected Dataset Version — durable, traceable, distinct from the local in-browser `runs` prototype above (Constitution Principle III)
function resetGraph(){
  nodes=[];edges=[];uid=1;
  const seq=[['source',12,10],['normalize',176,84],['gaussian',12,158],['threshold',176,232],['morphology',12,306],['area',176,380]];
  seq.forEach(([t,x,y])=>addNode(t,x,y,true));
  for(let i=0;i<nodes.length-1;i++)edges.push({from:nodes[i].id,to:nodes[i+1].id});
  selId=nodes[3].id;results=null;
}
function addNode(type,x,y,quiet){const n={id:'n'+uid++,type,x:Math.max(4,x),y:Math.max(4,y),params:dflt(type),ms:null,err:false};nodes.push(n);if(!quiet){selId=n.id;renderGraph()}return n}
const nodeById=id=>nodes.find(n=>n.id===id);
const activeSample=()=>samples.find(s=>s.id===activeId);

// ---- local service (Rosaray Service) integration (User Story 2) ----
// Boot with ?rosarayPort=<port>&rosaraySession=<token> from the service's
// stdout line to connect; without it the workspace starts empty.
function decodeBlobToLuma(blob){
  return new Promise((resolve,reject)=>{
    const img=new Image();
    img.onload=()=>{const w=img.width,h=img.height,c=document.createElement('canvas');c.width=w;c.height=h;
      const x=c.getContext('2d');x.drawImage(img,0,0);const px=x.getImageData(0,0,w,h).data,d=new Float32Array(w*h);
      for(let i=0;i<w*h;i++)d[i]=.299*px[i*4]+.587*px[i*4+1]+.114*px[i*4+2];
      URL.revokeObjectURL(img.src);resolve({w,h,d})};
    img.onerror=reject;img.src=URL.createObjectURL(blob)});
}
function decodeBlobToBinaryMask(blob,w,h){
  return new Promise((resolve,reject)=>{
    const img=new Image();
    img.onload=()=>{const c=document.createElement('canvas');c.width=w;c.height=h;
      const x=c.getContext('2d');x.drawImage(img,0,0,w,h);const px=x.getImageData(0,0,w,h).data,gt=new Uint8Array(w*h);
      for(let i=0;i<w*h;i++)gt[i]=px[i*4]>127?1:0;
      URL.revokeObjectURL(img.src);resolve(gt)};
    img.onerror=reject;img.src=URL.createObjectURL(blob)});
}

async function svcListDatasets(){
  try{
    const r=await svc.request('GET','/datasets');svcDatasets=r.datasets;
    if(!svcDatasetId&&svcDatasets.length)svcDatasetId=svcDatasets[0].id;
    if(svcDatasetId)await svcListVersions();
  }catch(e){toast('Could not load datasets: '+e.message,true)}
  renderLeft();
}
async function svcSelectDataset(id){svcDatasetId=id;svcVersionId=null;svcImages=[];await svcListVersions()}
async function svcListVersions(){
  if(!svcDatasetId)return;
  try{const r=await svc.request('GET',`/datasets/${svcDatasetId}/versions`);svcVersions=r.versions;
    if(!svcVersionId&&svcVersions.length)svcVersionId=svcVersions[svcVersions.length-1].id;
    await svcListImages();}
  catch(e){toast('Could not load dataset versions: '+e.message,true)}
  renderLeft();
}
async function svcListImages(){
  if(!svcVersionId)return;
  try{const r=await svc.request('GET',`/dataset-versions/${svcVersionId}/images`);svcImages=r.images}
  catch(e){toast('Could not load images: '+e.message,true);svcImages=[]}
}
async function svcSelectVersion(id){svcVersionId=id;await svcListImages();svcRuns=[];if(btab==='runs')await svcRefreshRuns();renderLeft()}

async function svcScanFolder(path){
  try{
    const r=await svc.request('POST','/import-batches',{source_selection:'folder_scan',paths:[path],metadata_manifest:null,dataset_display_name:'Rosaray Dataset'});
    const importable=r.candidates.filter(c=>c.classification==='importable').map(c=>c.source_ref);
    if(!importable.length){toast('No importable images found in that folder',true);return}
    const c=await svc.request('POST',`/import-batches/${r.batch_id}/confirm`,{confirmed_source_refs:importable});
    svcDatasetId=c.dataset_id;svcVersionId=c.dataset_version_id;
    toast(`Imported ${importable.length} image(s) into a new Dataset Version`);
    await svcListVersions();
  }catch(e){toast('Import failed: '+e.message,true)}
}

async function svcOpenImage(imageId){
  const reqId=++svcOpenReqId; // any earlier in-flight svcOpenImage() call is now stale
  try{
    const desc=await svc.request('GET',`/image-assets/${imageId}/display`);
    if(reqId!==svcOpenReqId)return; // a newer selection superseded this one — never overwrite it (FR-039)
    const imgBlob=await svc.request('GET',`/artifacts/${desc.image_artifact_ref.id}/content`);
    if(reqId!==svcOpenReqId)return;
    const{w,h,d}=await decodeBlobToLuma(imgBlob);
    if(reqId!==svcOpenReqId)return;
    let gt=null;
    if(desc.reference_mask_ref){
      const maskBlob=await svc.request('GET',`/artifacts/${desc.reference_mask_ref.id}/content`);
      if(reqId!==svcOpenReqId)return;
      gt=await decodeBlobToBinaryMask(maskBlob,w,h);
      if(reqId!==svcOpenReqId)return;
    }
    const sid='svc-'+imageId;
    const existing=samples.findIndex(s=>s.id===sid);
    const s={id:sid,name:`image-${imageId.slice(0,8)}.png`,patient:desc.display_metadata.patient_id||'—',
      split:desc.display_metadata.split||'train',spacing:.05,serviceImageId:imageId,
      overlayDisabledReason:desc.reference_mask_unavailable_reason||null,data:{w,h,d,gt}};
    if(existing>=0)samples[existing]=s;else samples.push(s);
    openSample(sid);
  }catch(e){if(reqId===svcOpenReqId)toast('Could not open image: '+e.message,true)}
}

function svcObserveThumbnails(container){
  if(svcThumbObserver)svcThumbObserver.disconnect();
  svcThumbObserver=new IntersectionObserver(entries=>{
    entries.forEach(en=>{
      const id=en.target.dataset.thumbFor;
      if(en.isIntersecting)svcLoadThumbnail(id,en.target);
      else svcCancelThumbnail(id);
    });
  },{root:container,rootMargin:'80px'});
  $$('[data-thumb-for]',container).forEach(el=>svcThumbObserver.observe(el));
}
async function svcLoadThumbnail(imageId,el){
  if(svcThumbCache.has(imageId)){el.style.backgroundImage=svcThumbCache.get(imageId);el.style.backgroundSize='cover';return}
  if(svcThumbRequests.has(imageId))return; // already in flight
  try{
    const r=await svc.request('GET',`/image-assets/${imageId}/thumbnail`);
    if(r.state==='ready'){const url=`url(data:image/png;base64,${r.data_base64})`;svcThumbCache.set(imageId,url);el.style.backgroundImage=url;el.style.backgroundSize='cover';return}
    if(r.state==='generating'&&r.request_id&&r.request_id!=='00000000-0000-0000-0000-000000000000'){
      svcThumbRequests.set(imageId,r.request_id);
      setTimeout(()=>{svcThumbRequests.delete(imageId);svcLoadThumbnail(imageId,el)},400);
    }
  }catch{/* thumbnail is best-effort; a failure just leaves the placeholder */}
}
function svcCancelThumbnail(imageId){
  const reqId=svcThumbRequests.get(imageId);
  if(reqId){svc.request('DELETE',`/requests/${reqId}`).catch(()=>{});svcThumbRequests.delete(imageId)}
}

// ---- official Runs against the Local Rosaray Service (User Story 4) ----
// Converts the client-side node graph into the `PipelineGraph` shape
// (domain/pipeline_snapshot.rs) the service hashes into a content-
// equivalence key — the same shape both `POST /preview` and `POST /runs`
// take, so an official Run and a Preview of the same inputs agree.
function toPipelineGraph(){
  return{
    nodes:nodes.map(n=>({node_id:n.id,node_type:n.type,implementation_version:'1',canonical_parameters:n.params||{},reproducible:true})),
    edges:edges.map(e=>({from:e.from,to:e.to})),
  };
}
// The pipeline's sink (no outgoing edge) is the Run's `target_node_id` —
// there is exactly one per the single-input-edge graph shape this UI builds.
const sinkNode=()=>nodes.find(n=>!edges.some(e=>e.from===n.id))||nodes[nodes.length-1];

async function svcRunOfficial(){
  const s=activeSample();
  if(!s||!s.serviceImageId){toast('Open an image from the Dataset panel first — official Runs need a service-backed image.',true);return}
  if(!svcVersionId){toast('No Dataset Version selected.',true);return}
  const bad=validateGraph();if(bad){toast(bad,true);return}
  const target=sinkNode();if(!target){toast('Pipeline has no nodes to run.',true);return}
  try{
    const r=await svc.request('POST','/runs',{
      dataset_version_id:svcVersionId,image_asset_id:s.serviceImageId,
      pipeline_snapshot:toPipelineGraph(),target_node_id:target.id,seed:42,
      run_policy:{retain_intermediates:false},
    });
    toast(`Official Run started: ${r.run_id.slice(0,8)}…`);
    setB('runs');
  }catch(e){toast('Could not start official Run: '+e.message,true)}
}
// ---- Export / Import bundles (User Story 5) ----
// The export credential is independent of the project passphrase and is only
// ever typed into the prompt and posted once — never stored (FR-035).
async function svcExportBundle(){
  if(!svc.isConnected()||svc.currentSessionState()!=='ready'){toast('Connect to the local service first.',true);return}
  if(!svcVersionId){toast('Select a Dataset Version in the Dataset panel to export.',true);return}
  await svcRefreshRuns();
  const runIds=svcRuns.filter(r=>r.status==='succeeded').map(r=>r.id);
  const credential=window.prompt(`Export credential for this bundle (${runIds.length} official Run${runIds.length===1?'':'s'} included). You will need it to open the bundle — it is not stored anywhere.`);
  if(!credential)return;
  try{
    const {result,blob}=await svc.exportBundle([svcVersionId],runIds,credential);
    const a=document.createElement('a');
    a.href=URL.createObjectURL(blob);a.download=`rosaray-${result.export_bundle_id.slice(0,8)}.rsybundle`;
    document.body.appendChild(a);a.click();a.remove();setTimeout(()=>URL.revokeObjectURL(a.href),1000);
    toast(`Bundle exported — ${result.manifest.content_list.length} content items, Preview excluded`);
  }catch(e){toast('Export failed: '+(e.code==='credential_required'?'a credential is required':e.message),true)}
}
const BUNDLE_IMPORT_ERRORS={
  credential_invalid:'Wrong export credential — nothing was imported.',
  credential_required:'A credential is required to open a bundle.',
  bundle_tampered:'This bundle is damaged or has been modified — nothing was imported.',
  bundle_incompatible:'This bundle was made with an incompatible data contract version — nothing was imported.'
};
async function svcImportBundleFile(file){
  if(!file)return;
  if(!svc.isConnected()||svc.currentSessionState()!=='ready'){toast('Connect to the local service first.',true);return}
  const credential=window.prompt(`Export credential for ${file.name}:`);
  if(!credential)return;
  try{
    const r=await svc.importBundle(file,credential);
    toast(`Bundle imported — ${r.imported_dataset_version_ids.length} Dataset Version(s), ${r.imported_run_record_ids.length} Run(s)`);
    await svcListDatasets();
    renderLeft();
  }catch(e){toast(BUNDLE_IMPORT_ERRORS[e.code]||('Import failed: '+e.message),true)}
}
function svcPickBundle(){
  const input=document.createElement('input');
  input.type='file';input.accept='.rsybundle';
  input.onchange=()=>svcImportBundleFile(input.files[0]);
  input.click();
}
async function svcRefreshRuns(){
  if(!svc.isConnected()||svc.currentSessionState()!=='ready'||!svcVersionId){svcRuns=[];return}
  try{const r=await svc.request('GET',`/runs?dataset_version_id=${svcVersionId}`);svcRuns=r.runs}
  catch{svcRuns=[]}
}

let lastSvcState=null;
svc.subscribeEvents(
  ev=>{
    if(ev.type==='thumbnail_ready'){const el=$(`[data-thumb-for="${ev.payload.image_asset_id}"]`);if(el)svcLoadThumbnail(ev.payload.image_asset_id,el);return}
    if(ev.type==='run_completed'||ev.type==='run_failed'){
      if(ev.type==='run_failed')toast('Official Run failed: '+(ev.payload.error_summary||'unknown error'),true);
      else toast('Official Run completed');
      svcRefreshRuns().then(()=>{if(btab==='runs')renderBottom()});
    }
  },
  state=>{const wasDown=lastSvcState&&lastSvcState!=='ready';lastSvcState=state;
    if(leftView==='data')renderLeft();
    paint(); // refresh the stale-content banner if the open image came from the service
    if(state==='ready'){
      // Reconnection procedure (event-bus.md): never trust stale data — re-fetch on the transition back to ready.
      if(!svcDatasetId)svcListDatasets();
      else if(wasDown)svcListVersions();
      if(wasDown&&btab==='runs')svcRefreshRuns().then(renderBottom);
    }
  }
);


// ---- menu bar ----
const MENUS={
  File:[['Import Image…','⌘O',()=>$('#file').click()],['Import Model (.onnx)…','',()=>toast('Model import goes through the local API: POST /api/projects/:id/assets')],'-',['Save Project','⌘S',()=>toast('Project saved to local SQLite (prototype)')],['Export Bundle…','',()=>svcExportBundle()],['Import Bundle…','',()=>svcPickBundle()]],
  Edit:[['Delete Selected Node','⌫',()=>delSel()],['Clear Pipeline','',()=>{nodes=[];edges=[];selId=null;renderGraph();toast('Pipeline cleared')}],['Reset Pipeline','',()=>{resetGraph();renderGraph();toast('Pipeline reset to default')}]],
  View:[['Toggle Side Bar','⌘B',()=>toggle('left')],['Toggle Pipeline Panel','⌥⌘B',()=>toggle('right')],['Toggle Bottom Panel','⌘J',()=>toggleBottom()],'-',['Fit Image to Window','',()=>fit()],['Actual Pixels (1:1)','',()=>zoomTo(1)],'-',['Theme: Light','',()=>setTheme('light')],['Theme: Dark','',()=>setTheme('dark')]],
  Pipeline:[['Run Pipeline','⌘↵',()=>run()],['Run via Service (official)…','',()=>svcRunOfficial()],['Validate Pipeline','',()=>{const e=validateGraph();toast(e||'Pipeline is valid: types match, no cycles',!!e)}]],
  Help:[['About Rosaray','',()=>toast('Rosaray — research prototype, not for diagnosis.')]]
};
function buildMenus(){
  const mb=$('#menubar');
  mb.innerHTML='<div class="brand"><img src="'+rosarayIcon+'" alt="" aria-hidden="true">Rosaray</div>'+Object.keys(MENUS).map(k=>`<button class="mb" data-k="${k}">${k}</button>`).join('')+'<span class="spacer"></span><span class="proj">Intraoral photo analysis · local project</span>';
  let cur=null;
  const close=()=>{$$('.dd').forEach(e=>e.remove());$$('.mb.open').forEach(b=>b.classList.remove('open'));cur=null};
  const open=b=>{close();cur=b.dataset.k;b.classList.add('open');const dd=document.createElement('div');dd.className='dd';dd.style.left=b.offsetLeft+'px';
    MENUS[cur].forEach(it=>{if(it==='-'){dd.appendChild(document.createElement('hr'));return}
      const x=document.createElement('button');x.innerHTML=`<span>${it[0]}</span><span>${it[1]}</span>`;x.onclick=()=>{close();it[2]()};dd.appendChild(x)});
    mb.appendChild(dd)};
  $$('.mb',mb).forEach(b=>{b.onclick=e=>{e.stopPropagation();cur===b.dataset.k?close():open(b)};b.onmouseenter=()=>{if(cur&&cur!==b.dataset.k)open(b)}});
  document.addEventListener('click',close);
}
function setTheme(t){document.documentElement.dataset.theme=t}


// ---- panels ----
let leftView='files';
function buildActivity(){
  const a=$('#activity');
  a.innerHTML=[['files','Explorer'],['data','Dataset'],['book','Evidence']].map(([k,t])=>`<button class="ab" data-v="${k}" title="${t}">${ic(k)}</button>`).join('')+'<span class="grow"></span><button class="ab" id="setb" title="Toggle theme">'+ic('gear')+'</button>';
  a.onclick=e=>{const b=e.target.closest('.ab');if(!b)return;
    if(b.id==='setb'){const dark=getComputedStyle(document.body).backgroundColor==='rgb(13, 18, 20)';setTheme(dark?'light':'dark');return}
    if(b.dataset.v===leftView&&!$('#left').classList.contains('hidden')){toggle('left');return}
    leftView=b.dataset.v;$('#left').classList.remove('hidden');renderLeft()};
}
function toggle(w){$('#'+w).classList.toggle('hidden');requestAnimationFrame(()=>{renderLeft();if(w!=='right')fit()})}
function toggleBottom(){$('#bottom').classList.toggle('hidden')}

function renderLeft(){
  const L=$('#left'),hid=L.classList.contains('hidden');
  $$('.ab[data-v]').forEach(b=>b.classList.toggle('on',!hid&&b.dataset.v===leftView));
  if(hid)return;
  if(leftView==='files'){
    L.innerHTML=`<div class="sh"><span>Explorer</span><button class="tb" title="Import image" id="imp">＋</button></div>
    <div class="scroll">
    <details class="sec" open><summary>Images <span class="count">${samples.length}</span></summary>
      ${samples.length?samples.map(s=>`<div class="row ${s.id===activeId?'on':''}" data-s="${s.id}"><span class="thumb" data-t="${s.id}"></span><span class="nm">${s.name}</span><span class="pill ${s.split}">${s.split.slice(0,5)}</span></div>`).join(''):'<div class="empty">No image open. Import an image or select one from Dataset.</div>'}
    </details>
    ${samples.some(s=>getImg(s).gt)?`<details class="sec" open><summary>Masks <span class="count">${samples.filter(s=>getImg(s).gt).length}</span></summary>
      ${samples.filter(s=>getImg(s).gt).map(s=>`<div class="row" data-s="${s.id}"><span class="thumb" style="background:var(--mask);opacity:.7"></span><span class="nm">${s.name.replace('.png','_ref.png')}</span><span class="pill">paired</span></div>`).join('')}
    </details>`:''}
    <details class="sec" open><summary>Models <span class="count">0</span></summary><div class="empty">No ONNX models yet.<br>File → Import Model…</div></details>
    <details class="sec"><summary>Runs <span class="count">${runs.length}</span></summary>${runs.length?runs.slice().reverse().map(r=>`<div class="row"><span class="nm mono">${r.id}</span><span class="pill">${r.dice==null?'—':'Dice '+r.dice.toFixed(2)}</span></div>`).join(''):'<div class="empty">Run the pipeline to create an immutable run.</div>'}</details>
    </div>`;
    $$('.row[data-s]',L).forEach(r=>r.onclick=()=>openSample(r.dataset.s));
    $$('[data-t]',L).forEach(t=>{const s=samples.find(x=>x.id===t.dataset.t),im=getImg(s),c=document.createElement('canvas');c.width=c.height=16;const x=c.getContext('2d'),tmp=toCanvas(im.w,im.h,im.d);x.drawImage(tmp,0,0,16,16);t.style.background=`url(${c.toDataURL()})`;t.style.backgroundSize='cover'});
    $('#imp').onclick=()=>$('#file').click();
  }else if(leftView==='data'){
    renderDatasetExplorer(L);
  }else{
    L.innerHTML=`<div class="sh"><span>Evidence</span></div><input class="search" id="q" placeholder="Search PubMed / Hugging Face…" aria-label="Search evidence">
      <div class="note">Evidence attaches to a pipeline node, not to the project. Select a node, search, then attach a result.<br><br>Search runs through the local API (<span class="mono">/api/search/pubmed</span>); only the query text leaves this machine. Not connected in this prototype.</div>`;
  }
}
const fingerprint=()=>hash(samples.map(s=>[s.id,s.patient,s.split,s.seed].join(':')).sort().join('|'));

const SVC_STATE_LABEL={connecting:'Connecting…',ready:'Connected',unavailable:'Service unavailable',access_denied:'Access denied'};
function renderDatasetExplorer(L){
  if(!svc.isConnected()){
    L.innerHTML=`<div class="sh"><span>Dataset</span></div><div class="scroll">
      <div class="note">Not connected to the Local Rosaray Service. Start Rosaray with <span class="mono">./scripts/launch.sh</span> to browse encrypted, versioned datasets here.</div></div>`;
    return;
  }
  const st=svc.currentSessionState();
  if(st!=='ready'){
    // Never keep showing a prior Dataset/image list as current once the
    // connection drops — that would look live when it is stale (FR-039).
    L.innerHTML=`<div class="sh"><span>Dataset</span><span class="pill ${st==='connecting'?'warnc':'errc'}">${SVC_STATE_LABEL[st]||st}</span></div>
      <div class="scroll"><div class="note">${st==='connecting'?'Reconnecting to the Local Rosaray Service…':'Lost connection to the Local Rosaray Service. The Dataset and image lists are hidden until reconnected, so nothing stale is shown as current.'}</div>
      ${st==='unavailable'?'<button class="btn" id="retrybtn">Retry connection</button>':''}
      </div>`;
    $('#retrybtn',L)&&($('#retrybtn').onclick=()=>{if(!svc.retryConnect())toast('Not connected to the local service',true)});
    return;
  }
  L.innerHTML=`<div class="sh"><span>Dataset</span><span class="pill ok">${SVC_STATE_LABEL[st]||st}</span></div>
    <div class="scroll">
    <div class="fld"><label for="scanpath">Folder to import (absolute path)</label><input id="scanpath" type="text" placeholder="/path/to/images"></div>
    <button class="btn" id="scanbtn">Scan &amp; import folder</button>
    ${svcDatasets.length>1?`<div class="fld"><label for="dspick">Dataset</label><select id="dspick">${svcDatasets.map(d=>`<option value="${d.id}" ${d.id===svcDatasetId?'selected':''}>${d.display_name}</option>`).join('')}</select></div>`:''}
    ${svcVersions.length?`<div class="fld"><label for="verpick">Dataset Version</label><select id="verpick">${svcVersions.slice().reverse().map(v=>`<option value="${v.id}" ${v.id===svcVersionId?'selected':''}>${v.created_at} · ${v.image_count} image(s) · ${v.validation_summary.status}</option>`).join('')}</select></div>`:''}
    ${svcVersionId?renderSvcImageList():'<div class="empty">No Dataset Version yet — scan a folder above.</div>'}
    </div>`;
  $('#scanbtn',L).onclick=()=>{const p=$('#scanpath',L).value.trim();if(p)svcScanFolder(p)};
  $('#dspick',L)&&($('#dspick').onchange=e=>svcSelectDataset(e.target.value));
  $('#verpick',L)&&($('#verpick').onchange=e=>svcSelectVersion(e.target.value));
  $$('.row[data-svc-img]',L).forEach(r=>r.onclick=()=>svcOpenImage(r.dataset.svcImg));
  if(svcVersionId)svcObserveThumbnails($('.scroll',L));
}
function renderSvcImageList(){
  const v=svcVersions.find(x=>x.id===svcVersionId);
  const findingBadge=v&&v.validation_summary.status!=='ok'?`<span class="pill ${v.validation_summary.status==='blocked'?'errc':'warnc'}">${v.validation_summary.status} · ${v.validation_summary.finding_ids.length} finding(s)</span>`:'';
  return `<details class="sec" open><summary>Images <span class="count">${svcImages.length}</span> ${findingBadge}</summary>
    ${svcImages.map(im=>`<div class="row" data-svc-img="${im.id}">
      <span class="thumb" data-thumb-for="${im.id}"></span>
      <span class="nm">${im.display_name}${im.source_status!=='available'?' <em>('+im.source_status+')</em>':''}</span>
      <span class="pill ${im.split||''}">${im.split||'—'}</span>
      <span class="pill ${im.reference_mask_status==='valid'?'':im.reference_mask_status==='invalid'?'warnc':''}">${im.reference_mask_status}</span>
    </div>`).join('')}
    </details>`;
}


// ---- viewer ----
const cv=$('#cv'),wrap=$('#stagewrap'),stage=$('#stage');
let view={s:1,x:0,y:0};
function toCanvas(w,h,d,maskAlpha){const c=document.createElement('canvas');c.width=w;c.height=h;const x=c.getContext('2d'),id=x.createImageData(w,h);
  for(let i=0;i<w*h;i++){const v=d[i];id.data[i*4]=id.data[i*4+1]=id.data[i*4+2]=v;id.data[i*4+3]=255}x.putImageData(id,0,0);return c}
function paint(){
  updateModeButtons();
  const s=activeSample();
  if(!s){
    cv.width=1;cv.height=1;
    $('#hud').innerHTML='<b>No image open</b><br>Import an image, or select one from Dataset.';
    $('#legend').innerHTML='';$('#scalebar').innerHTML='';
    return;
  }
  const im=getImg(s);cv.width=im.w;cv.height=im.h;const x=cv.getContext('2d');
  const vis=results&&results.sampleId===activeId?results:null;
  if(mode==='output'&&vis&&vis.visual){const v=vis.visual;
    x.drawImage(toCanvas(v.w,v.h,v.kind==='mask'?v.d.map(m=>m*255):v.d),0,0)}
  else x.drawImage(toCanvas(im.w,im.h,im.d),0,0);
  // Overlay mode shows the reference (ground-truth) mask, per US2 acceptance
  // scenario 3 — not the pipeline's predicted output mask.
  if(mode==='overlay'&&im.gt){const id=x.getImageData(0,0,im.w,im.h),c=getComputedStyle(document.documentElement).getPropertyValue('--mask').trim(),
    rgb=parseInt(c.slice(1),16),R=rgb>>16,G=(rgb>>8)&255,B=rgb&255;
    for(let i=0;i<im.gt.length;i++)if(im.gt[i]){id.data[i*4]=id.data[i*4]*.45+R*.55;id.data[i*4+1]=id.data[i*4+1]*.45+G*.55;id.data[i*4+2]=id.data[i*4+2]*.45+B*.55}x.putImageData(id,0,0)}
  const noRes=mode==='output'&&!vis;
  const svcStale=s.serviceImageId&&svc.isConnected()&&svc.currentSessionState()!=='ready';
  $('#hud').innerHTML=`${svcStale?'<div class="errc">⚠ Service disconnected — this content may be stale</div>':''}<b>${s.name}</b><br>${im.w} × ${im.h} px · ${s.patient} · ${s.split}${s.serviceImageId?' · reference mask '+(im.gt?'available':'unavailable'):''}<br>${noRes?'<span style="color:var(--mask)">No result yet — press Run pipeline</span>':'mode: '+mode}`;
  $('#legend').innerHTML=mode==='overlay'&&im.gt?'<span><i></i>reference mask</span>':'';
  const mm=Math.round(100/s.spacing/view.s/10)*10||10;
  $('#scalebar').innerHTML=`<div style="width:${(10/s.spacing)*view.s}px"></div>10 mm`;
  applyView();
}
function updateModeButtons(){
  const s=activeSample();
  if(!s){
    mode='input';
    $$('#modes button').forEach(b=>b.classList.toggle('on',b.dataset.m===mode));
    $$('[data-m]').forEach(b=>{b.disabled=b.dataset.m!=='input';b.title=b.disabled?'Open an image first.':''});
    return;
  }
  $$('[data-m]').forEach(b=>{b.disabled=false;b.title=''});
  const reason=s.overlayDisabledReason||(getImg(s).gt?null:'No reference mask available for this image.');
  const btn=$('[data-m="overlay"]',$('#modes'));
  if(btn){btn.disabled=!!reason;btn.title=reason||''}
  if(reason&&mode==='overlay'){mode='input';$$('#modes button').forEach(x=>x.classList.toggle('on',x.dataset.m===mode))}
}
function applyView(){stage.style.transform=`translate(${view.x}px,${view.y}px) scale(${view.s})`;cv.style.imageRendering=view.s>3?'pixelated':'auto';const s=activeSample();if(s)$('#scalebar').lastChild&&($('#scalebar').firstChild.style.width=(10/s.spacing)*view.s+'px')}
function fit(){const s=activeSample();if(!s)return;const im=getImg(s),W=wrap.clientWidth,H=wrap.clientHeight;if(!W)return;
  view.s=Math.min(W/im.w,H/im.h)*.92;view.x=(W-im.w*view.s)/2;view.y=(H-im.h*view.s)/2;applyView()}
function zoomTo(k,cx=wrap.clientWidth/2,cy=wrap.clientHeight/2){const n=Math.max(.2,Math.min(16,k));view.x=cx-(cx-view.x)*n/view.s;view.y=cy-(cy-view.y)*n/view.s;view.s=n;applyView()}
wrap.addEventListener('wheel',e=>{e.preventDefault();const r=wrap.getBoundingClientRect();zoomTo(view.s*(e.deltaY<0?1.12:1/1.12),e.clientX-r.left,e.clientY-r.top)},{passive:false});
let pan=null;
wrap.addEventListener('pointerdown',e=>{pan={x:e.clientX-view.x,y:e.clientY-view.y};wrap.classList.add('pan');wrap.setPointerCapture(e.pointerId)});
wrap.addEventListener('pointermove',e=>{
  if(pan){view.x=e.clientX-pan.x;view.y=e.clientY-pan.y;applyView()}
  const r=wrap.getBoundingClientRect(),s=activeSample();if(!s)return;const im=getImg(s),ix=Math.floor((e.clientX-r.left-view.x)/view.s),iy=Math.floor((e.clientY-r.top-view.y)/view.s);
  $('#px').textContent=ix>=0&&iy>=0&&ix<im.w&&iy<im.h?`x ${ix}  y ${iy}  I ${im.d[iy*im.w+ix]|0}`:'—'});
wrap.addEventListener('pointerup',()=>{pan=null;wrap.classList.remove('pan')});
$('#zin').onclick=()=>zoomTo(view.s*1.25);$('#zout').onclick=()=>zoomTo(view.s/1.25);$('#zfit').onclick=fit;$('#z1').onclick=()=>zoomTo(1);
$('#modes').onclick=e=>{const b=e.target.closest('button');if(!b||b.disabled)return;
  mode=b.dataset.m;$$('#modes button').forEach(x=>x.classList.toggle('on',x===b));paint()};

function renderTabs(){
  $('#tabs').innerHTML=openTabs.map(id=>{const s=samples.find(x=>x.id===id);return s?`<div class="tab ${id===activeId?'on':''}" data-id="${id}"><span>${s.name}</span><b data-x="${id}" title="Close">✕</b></div>`:''}).join('');
  $$('.tab').forEach(t=>t.onclick=e=>{if(e.target.dataset.x){closeTab(e.target.dataset.x);return}openSample(t.dataset.id)});
}
function openSample(id){
  if(!samples.some(s=>s.id===id))return;
  if(!openTabs.includes(id))openTabs.push(id);activeId=id;renderTabs();renderLeft();paint();fit();renderBottom();
}
function closeTab(id){
  openTabs=openTabs.filter(x=>x!==id);
  if(activeId===id)activeId=openTabs[0]||null;
  renderTabs();renderLeft();paint();if(activeId){fit()}renderBottom();
}
$('#file').onchange=e=>{const f=e.target.files[0];if(!f)return;const img=new Image();img.onload=()=>{
  const k=Math.min(1,1400/Math.max(img.width,img.height)),w=Math.round(img.width*k),h=Math.round(img.height*k),c=document.createElement('canvas');c.width=w;c.height=h;
  const x=c.getContext('2d');x.drawImage(img,0,0,w,h);const px=x.getImageData(0,0,w,h).data,d=new Float32Array(w*h);
  for(let i=0;i<w*h;i++)d[i]=.299*px[i*4]+.587*px[i*4+1]+.114*px[i*4+2];
  const s={id:'u'+Date.now(),name:f.name,patient:'—',split:'train',spacing:.05,data:{w,h,d,gt:null}};samples.push(s);toast(`Imported ${f.name} (${w}×${h}) — patient ID and split need review`);openSample(s.id)};
  img.src=URL.createObjectURL(f);e.target.value=''};


// ---- pipeline UI ----
function renderPalette(){
  $('#palette').innerHTML=Object.entries(DEFS).filter(([k])=>k!=='source').map(([k,d])=>`<div class="chip" draggable="true" data-t="${k}" title="${d.in||'—'} → ${d.out}"><i class="dot c-${d.cat}"></i>${d.label}</div>`).join('');
  $$('.chip').forEach(c=>{c.ondragstart=e=>e.dataTransfer.setData('text/plain',c.dataset.t);c.ondblclick=()=>{addNode(c.dataset.t,12+(nodes.length%2)*164,10+nodes.length*74)}});
}
const gin=$('#gin');
$('#graph').ondragover=e=>e.preventDefault();
$('#graph').ondrop=e=>{e.preventDefault();const t=e.dataTransfer.getData('text/plain');if(!DEFS[t])return;const r=gin.getBoundingClientRect();addNode(t,e.clientX-r.left-78,e.clientY-r.top-20)};
const summ=n=>{const p=n.params;return n.type==='threshold'?(p.mode==='otsu'?'otsu':'t='+p.value)+(p.invert?' · inv':''):n.type==='morphology'?p.op+' r'+p.radius:n.type==='gaussian'?'σ '+p.sigma:n.type==='normalize'?`${p.lo}–${p.hi} pct`:n.type==='area'?p.spacing+' mm/px':n.type==='onnx'?'no model':DEFS[n.type].out};
function renderGraph(){
  $$('.node',gin).forEach(e=>e.remove());
  nodes.forEach(n=>{const d=DEFS[n.type],el=document.createElement('div');el.className='node'+(n.id===selId?' sel':'')+(n.err?' err':'');el.style.left=n.x+'px';el.style.top=n.y+'px';el.dataset.id=n.id;
    el.innerHTML=(d.in?`<span class="port in" data-p="in"></span>`:'')+`<div class="nh"><i class="dot c-${d.cat}"></i>${d.label}</div><div class="ns">${summ(n)}</div>${n.ms!=null?`<span class="ms">${n.ms.toFixed(0)} ms</span>`:''}`+(d.out?`<span class="port out ${pending===n.id?'pend':''}" data-p="out"></span>`:'');
    el.onpointerdown=e=>{
      const p=e.target.dataset.p;if(p){e.stopPropagation();e.preventDefault();port(n,p);return}
      selId=n.id;const sx=e.clientX-n.x,sy=e.clientY-n.y;el.setPointerCapture(e.pointerId);let moved=false;
      el.onpointermove=ev=>{moved=true;n.x=Math.max(0,ev.clientX-sx);n.y=Math.max(0,ev.clientY-sy);el.style.left=n.x+'px';el.style.top=n.y+'px';drawEdges()};
      el.onpointerup=()=>{el.onpointermove=el.onpointerup=null;renderGraph()}};
    gin.appendChild(el)});
  drawEdges();renderInspector();$('#gstat').textContent=nodes.length+' nodes';
}
function drawEdges(){
  $('#edges').innerHTML=edges.map(e=>{const a=nodeById(e.from),b=nodeById(e.to);if(!a||!b)return'';
    const x1=a.x+78,y1=a.y+52,x2=b.x+78,y2=b.y,dy=Math.max(30,Math.abs(y2-y1)/2);
    return`<path class="${DEFS[a.type].out}" d="M${x1},${y1} C${x1},${y1+dy} ${x2},${y2-dy} ${x2},${y2}"/>`}).join('');
}
function port(n,p){
  if(p==='out'){pending=pending===n.id?null:n.id;renderGraph();if(pending)toast('Now click the input port (top) of the next node');return}
  if(!pending){toast('Click an output port (bottom of a node) first');return}
  const a=nodeById(pending);pending=null;
  if(a.id===n.id){renderGraph();return}
  if(DEFS[a.type].out!==DEFS[n.type].in){toast(`Type mismatch: ${DEFS[a.type].label} outputs ${DEFS[a.type].out}, ${DEFS[n.type].label} expects ${DEFS[n.type].in}`,true);renderGraph();return}
  if(reach(n.id,a.id)){toast('That link would create a cycle',true);renderGraph();return}
  edges=edges.filter(e=>e.to!==n.id);edges.push({from:a.id,to:n.id});renderGraph();
}
const reach=(from,to)=>{const seen=new Set(),st=[from];while(st.length){const c=st.pop();if(c===to)return true;if(seen.has(c))continue;seen.add(c);edges.filter(e=>e.from===c).forEach(e=>st.push(e.to))}return false};
function delSel(){if(!selId)return;const n=nodeById(selId);if(n&&n.type==='source'){toast('Image source cannot be removed',true);return}nodes=nodes.filter(n=>n.id!==selId);edges=edges.filter(e=>e.from!==selId&&e.to!==selId);selId=null;renderGraph()}
function renderInspector(){
  const n=nodeById(selId),b=$('#ibody');
  if(!n){$('#ititle').textContent='Inspector';$('#isub').textContent='';b.innerHTML='<div class="note">Select a node to edit its parameters.</div>';return}
  const d=DEFS[n.type];$('#ititle').textContent=d.label;$('#isub').textContent=`${d.in||'—'} → ${d.out}`;
  b.innerHTML=(d.params.length?d.params.map(p=>{
    if(p.t==='range')return`<div class="fld"><label for="p_${p.k}">${p.l}</label><input id="p_${p.k}" type="range" min="${p.min}" max="${p.max}" step="${p.step}" value="${n.params[p.k]}" ${n.type==='threshold'&&p.k==='value'&&n.params.mode==='otsu'?'disabled':''}><output>${n.params[p.k]}</output></div>`;
    if(p.t==='select')return`<div class="fld"><label for="p_${p.k}">${p.l}</label><select id="p_${p.k}">${p.o.map(o=>`<option ${o==n.params[p.k]?'selected':''}>${o}</option>`).join('')}</select></div>`;
    return`<div class="fld"><label for="p_${p.k}">${p.l}</label><input id="p_${p.k}" type="checkbox" ${n.params[p.k]?'checked':''}></div>`}).join(''):'<div class="note">No parameters.</div>')+
    `<div class="note" style="padding-top:6px">Evidence: none attached (0)</div>`;
  d.params.forEach(p=>{const el=$('#p_'+p.k,b);if(!el)return;el.oninput=el.onchange=()=>{
    n.params[p.k]=p.t==='check'?el.checked:p.t==='range'?+el.value:el.value;
    if(p.t==='range')el.nextElementSibling.textContent=el.value;
    if(p.t!=='range'){renderGraph();return}
    const s=$(`.node[data-id="${n.id}"] .ns`);if(s)s.textContent=summ(n)}});
}
$('#graph').onpointerdown=e=>{if(e.target.id==='graph'||e.target.id==='gin'||e.target.id==='edges'){selId=null;pending=null;renderGraph()}};


// ---- run ----
function validateGraph(){
  const srcs=nodes.filter(n=>n.type==='source');if(srcs.length!==1)return'Pipeline needs exactly one Image source';
  for(const e of edges)if(DEFS[nodeById(e.from).type].out!==DEFS[nodeById(e.to).type].in)return'Edge type mismatch';
  return null;
}
function run(){
  if(!activeSample()){toast('Open or import an image before running the pipeline.',true);return}
  const bad=validateGraph();if(bad){toast(bad,true);return}
  nodes.forEach(n=>{n.err=false;n.ms=null});
  const s=activeSample(),img=getImg(s),ctx={img,notes:[]},out={},steps=[],src=nodes.find(n=>n.type==='source');
  let visual=null,mask=null,table=null;
  const walk=n=>{const t0=performance.now(),inp=(edges.find(e=>e.to===n.id)||{}).from;
    try{const r=exec(n.type,n.params,inp?out[inp]:null,ctx);n.ms=performance.now()-t0;out[n.id]=r;steps.push({n,ms:n.ms});
      if(r.kind==='image'||r.kind==='mask')visual=r;if(r.kind==='mask')mask=r;if(r.kind==='table')table=r.rows}
    catch(err){n.err=true;renderGraph();toast(err.message,true);throw err}
    edges.filter(e=>e.from===n.id).forEach(e=>walk(nodeById(e.to)))};
  try{walk(src)}catch(e){return}
  const warnings=[];const orphan=nodes.filter(n=>!out[n.id]);if(orphan.length)warnings.push(`${orphan.length} node(s) not connected to the source were skipped`);
  if(!img.gt)warnings.push('No reference mask for this image — Dice not computed');
  const dc=mask&&img.gt&&mask.d.length===img.gt.length?dice(mask.d,img.gt):null;
  results={sampleId:s.id,visual,mask,table,steps,dice:dc,notes:ctx.notes};
  const r={id:'run-'+hash(Date.now()+s.id).slice(0,6),t:new Date(),img:s.name,dice:dc,area:table?table.mm2:null,fp:fingerprint(),graph:hash(JSON.stringify(nodes.map(n=>[n.type,n.params]))),warnings,seed:42};
  runs.push(r);lastRun=r;
  renderGraph();paint();renderLeft();setB('metrics');
  toast(`Run complete — ${steps.length} steps${dc!=null?', Dice '+dc.toFixed(3):''}`);
  if(mode==='input'){mode='overlay';$$('#modes button').forEach(b=>b.classList.toggle('on',b.dataset.m===mode));paint()}
}
$('#runbtn').onclick=run;


// ---- bottom panel ----
let btab='metrics';
function setB(k){btab=k;$$('#btabs button[data-b]').forEach(b=>b.classList.toggle('on',b.dataset.b===k));$('#bottom').classList.remove('hidden');renderBottom();if(k==='runs')svcRefreshRuns().then(renderBottom)}
$('#btabs').onclick=e=>{const b=e.target.closest('button');if(!b)return;if(b.id==='bclose')return toggleBottom();setB(b.dataset.b)};
function renderBottom(){
  const B=$('#bbody');
  if(btab==='metrics'){
    if(!activeSample()){B.innerHTML='<div class="note">No image open. Import an image or select one from Dataset to begin.</div>';return}
    if(!results||results.sampleId!==activeId){B.innerHTML='<div class="note">No run for this image yet. Press <b>Run pipeline</b> (⌘↵) to compute mask, Dice and area.</div>';return}
    const t=results.table;
    B.innerHTML=`<div class="metrics">
      <div class="metric"><small>Dice vs reference</small><strong>${results.dice==null?'—':results.dice.toFixed(3)}</strong></div>
      <div class="metric"><small>Area</small><strong>${t?t.mm2.toFixed(1):'—'}<em>mm²</em></strong></div>
      <div class="metric"><small>Pixels</small><strong>${t?t.px:'—'}</strong></div>
      <div class="metric"><small>Components</small><strong>${t?t.cc:'—'}</strong></div></div>
      <table class="tbl"><tr><th>Step</th><th>Node</th><th>ms</th></tr>${results.steps.map((s,i)=>`<tr><td>${i+1}</td><td>${DEFS[s.n.type].label}</td><td>${s.ms.toFixed(1)}</td></tr>`).join('')}</table>
      <div class="note">${results.notes.join(' · ')} · preview resolution</div>`;
  }else if(btab==='runs'){
    const local=runs.length?`<table class="tbl"><tr><th>Run</th><th>Image</th><th>Dice</th><th>Area mm²</th><th>Dataset fp</th><th>Graph</th><th>Time</th></tr>${runs.slice().reverse().map(r=>`<tr><td>${r.id}</td><td>${r.img}</td><td>${r.dice==null?'—':r.dice.toFixed(3)}</td><td>${r.area==null?'—':r.area.toFixed(1)}</td><td>${r.fp}</td><td>${r.graph}</td><td>${r.t.toLocaleTimeString()}</td></tr>`).join('')}</table><div class="note">Runs are immutable snapshots: graph, parameters, dataset fingerprint and seed.</div>`:'<div class="note">No runs yet.</div>';
    const svcSection=svc.isConnected()?`<div class="sh" style="margin-top:10px"><span>Official Runs (service)</span><button class="tb" title="Run current pipeline as an official Run" id="svcrunbtn">▶</button></div>`+
      (svcRuns.length?`<table class="tbl"><tr><th>Run</th><th>Status</th><th>Seed</th><th>Dataset fp</th><th>Started</th><th>Note</th></tr>${svcRuns.slice().reverse().map(r=>`<tr><td class="mono">${r.id.slice(0,8)}</td><td>${r.status}${r.status==='running'?' …':''}</td><td>${r.seed}</td><td class="mono">${r.dataset_fingerprint.slice(0,10)}</td><td>${new Date(r.started_at).toLocaleTimeString()}</td><td>${r.error_summary||'—'}</td></tr>`).join('')}</table><div class="note">Official Runs are durable and fully traceable — dataset fingerprint, image, pipeline and seed are recorded atomically on success (FR-018/FR-020).</div>`:'<div class="note">No official Runs yet for this Dataset Version. Open a Dataset image, then Pipeline → Run via Service (official).</div>'):'';
    B.innerHTML=local+svcSection;
    $('#svcrunbtn',B)&&($('#svcrunbtn').onclick=()=>svcRunOfficial());
  }else{
    const byPatient={};samples.forEach(s=>(byPatient[s.patient]??=new Set()).add(s.split));
    const leak=Object.entries(byPatient).filter(([p,v])=>p!=='—'&&v.size>1);
    const items=[[leak.length?'errc':'ok',leak.length?'✕':'✓',leak.length?'Patient leakage: '+leak.map(l=>l[0]).join(', '):'No patient appears in more than one split'],
      ['ok','✓','Every reference mask is paired to an image'],
      ...samples.filter(s=>s.patient==='—').map(s=>['warnc','!',`${s.name}: patient ID missing — set it before splitting`]),
      ['ok','✓',bad=>0||'Pipeline types match, no cycles']];
    const ge=validateGraph();items[items.length-1]=[ge?'errc':'ok',ge?'✕':'✓',ge||'Pipeline types match, no cycles'];
    B.innerHTML=items.map(i=>`<div class="row" style="padding-left:14px;cursor:default"><span class="${i[0]}">${i[1]}</span><span>${i[2]}</span></div>`).join('');
  }
}


// ---- resizers ----
$$('.rz,.rzy').forEach(z=>{z.onpointerdown=e=>{
  const t=z.dataset.target,el=$('#'+t),vert=z.classList.contains('rzy');z.classList.add('drag');z.setPointerCapture(e.pointerId);
  const start=vert?e.clientY:e.clientX,size=vert?el.offsetHeight:el.offsetWidth;
  z.onpointermove=ev=>{const dx=(vert?ev.clientY:ev.clientX)-start;
    const v=t==='left'?size+dx:t==='right'?size-dx:size-dx;const c=Math.max(vert?90:200,Math.min(vert?480:520,v));
    el.style.setProperty(t==='left'?'--lw':t==='right'?'--rw':'--bh',c+'px');
    if(t==='left')el.style.width=c+'px';if(t==='right')el.style.width=c+'px';if(vert)el.style.height=c+'px'};
  z.onpointerup=()=>{z.classList.remove('drag');z.onpointermove=z.onpointerup=null;fit()}}});


// ---- keys boot ----
addEventListener('keydown',e=>{const m=e.metaKey||e.ctrlKey;
  if(m&&e.key==='Enter'){e.preventDefault();run()}
  else if(m&&e.key.toLowerCase()==='b'){e.preventDefault();toggle(e.altKey?'right':'left')}
  else if(m&&e.key.toLowerCase()==='j'){e.preventDefault();toggleBottom()}
  else if((e.key==='Delete'||e.key==='Backspace')&&!/INPUT|SELECT/.test(document.activeElement.tagName))delSel()});
addEventListener('resize',fit);

buildMenus();buildActivity();renderPalette();resetGraph();renderGraph();renderTabs();
if(innerWidth<760){$('#left').classList.add('hidden');$('#right').classList.add('hidden')}
svc.autoConnect();
renderLeft();paint();setB('metrics');
requestAnimationFrame(()=>{$('#toast').hidden=true});
