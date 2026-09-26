// Draft editor + published-bundle viewer (feature 002, US1). Files of a draft
// are edited as text and saved with the revision they were opened at; if the
// folder changed on disk in the meantime (an external editor, FR-044) the
// save is refused and the researcher must resolve explicitly — nothing is
// ever overwritten silently. Published bundles are shown read-only.
import {$,$$,esc,toast} from './util.js';
import * as svc from './service_client.js';
import {findingsHtml,startExport} from './kb_catalog.js';
import {loadNodeDefs,nodeDef,previewPipe,previewStatus} from './registry.js';
import {createGraphEditor,computationalKey} from './graph_editor.js';
import {inspectorHtml,openInspector,RESEARCH_ONLY,getImage,getDatasetVersion} from './node_inspector.js';

const NO_PII='Do not enter patient names or other identifying information in any field — knowledge bundles are shared as plain files.';
const GRAPH='__graph__';
const E={pubResult:null,release:'knowledge',graph:null,origGraphJson:null,editor:null,geHost:null,mode:null,kind:null,id:null,version:null,revision:null,orig:{},files:{},active:null,findings:[],header:null,conflict:null,external:false,busy:false,onExport:null,view:null};
let pane=null,subscribed=false;

const dirty=()=>Object.keys(E.files).filter(p=>E.files[p]!==E.orig[p]);
const graphDirty=()=>E.kind==='algopipe'&&E.editor&&JSON.stringify(E.editor.getGraph())!==E.origGraphJson;
const isDirty=()=>dirty().length>0||graphDirty();
const changeCount=()=>dirty().length+(graphDirty()?1:0);

function ensurePane(){
  if(pane)return pane;
  pane=document.createElement('section');pane.id='kbpane';pane.hidden=true;
  ($('#center')||document.body).appendChild(pane);
  return pane;
}

export function mount(container){
  // The editor lives over the center area; `container` is only used to
  // ensure the pane exists in the workbench.
  if(container&&!pane){pane=ensurePane()}
  ensurePane();
  if(!subscribed){
    subscribed=true;
    // A computational edit made earlier Preview results out of date (FR-018).
    svc.onEventType(['preview_stale'],p=>{
      if(E.mode!=='draft'||p.pipe_id!==E.id)return;
      if(PV.result&&p.nodes.includes(PV.target))PV.stale=true;
      refreshStatuses();renderPreview();
    });
    svc.onEventType(['kb_bundle_changed_externally'],p=>{
      if(E.mode==='draft'&&p.kind===E.kind&&p.id===E.id&&p.revision!==E.revision){E.external=true;draw()}
    });
  }
}

function close(){
  if(isDirty()&&!confirm('Discard unsaved changes to this draft?'))return;
  E.mode=null;ensurePane().hidden=true;
  document.dispatchEvent(new CustomEvent('kb:changed'));
}

function show(){ensurePane().hidden=false;draw()}

// ---- open ----------------------------------------------------------------
export async function openDraft(kind,id){
  mount();
  if(E.mode==='draft'&&isDirty()&&!(E.kind===kind&&E.id===id)&&!confirm('Discard unsaved changes to the open draft?'))return;
  try{
    const d=await svc.kb.getDraft(kind,id);
    if(kind==='algopipe')defCache=await loadNodeDefs(true).catch(()=>[]);
    E.editor=null;E.geHost=null;
    // A pipe's graph is edited structurally; its raw graph.yaml is not a tab.
    const files={...d.files};if(kind==='algopipe')delete files['graph.yaml'];
    Object.assign(E,{pubResult:E.id===id?E.pubResult:null,mode:'draft',kind,id,version:d.header?.version||null,revision:d.revision,orig:{...files},files:{...files},findings:d.findings||[],header:d.header,conflict:null,external:false,view:null,graph:d.graph||null});
    E.active=kind==='algopipe'?GRAPH:(files['contract.yaml']!==undefined?'contract.yaml':Object.keys(files)[0]);
    if(kind==='algopipe')startGraphEditor(d.graph);
    Object.assign(PV,{running:false,seq:PV.seq+1,result:null,failure:null,error:null,stale:false,target:null,port:null,outputUrl:null,outputText:null});
    show();
  }catch(e){toast(e.message,true)}
}

function startGraphEditor(graph){
  E.geHost=document.createElement('div');
  const defs=(i,v)=>nodeDef(i,v);
  defs.list=()=>defCache;
  E.editor=createGraphEditor({
    container:E.geHost,graph:graph||{schema:'quantify-kb/1',nodes:[],edges:[]},domain:E.header?.domain,defs,
    validate:(g,previous)=>svc.designer.validate({kind:'algopipe',id:E.id,graph:g,previous:previous||undefined,profile:g.target_data_profile||undefined,domain:E.header?.domain||undefined}),
    onChange:()=>updateDirtyUi(),
    onInspect:ref=>openInspector('algonode',ref.id,ref.version),
  });
  E.origGraphJson=JSON.stringify(E.editor.getGraph());
}
let defCache=[]; // node definitions the editor may add (registry cache)

function updateDirtyUi(){
  const s=$('#kbp-save',pane);
  if(s){s.disabled=!isDirty()||E.busy;s.textContent='Save'+(isDirty()?` (${changeCount()})`:'')}
  const t=$('.kbp-tab[data-f="'+GRAPH+'"]',pane);if(t)t.textContent='Graph'+(graphDirty()?' ●':'');
}

export async function openPublished(kind,id,version,opts={}){
  mount();
  try{
    const b=await svc.kb.getBundle(kind,id,version);
    Object.assign(E,{mode:'published',kind,id,version,view:b,findings:b.findings||[],header:b.header,conflict:null,external:false,onExport:opts.onExport||null,files:{},orig:{}});
    show();
  }catch(e){toast(e.message,true)}
}

export function openInvalid(entry){
  mount();
  Object.assign(E,{mode:'invalid',kind:entry.kind,id:entry.id,version:entry.version,view:entry,findings:[],header:null,conflict:null,external:false,files:{},orig:{}});
  show();
  svc.request('GET',`/kb/findings?status=invalid${entry.id?`&id=${encodeURIComponent(entry.id)}`:''}`).then(r=>{
    const mine=(r.bundles||[]).find(b=>(b.id||null)===(entry.id||null)&&(b.version||null)===(entry.version||null))||(r.bundles||[])[0];
    E.findings=mine?.findings||[];if(E.mode==='invalid')draw();
  }).catch(()=>{});
}

// ---- save / publish -------------------------------------------------------
async function save(){
  if(E.busy||!isDirty())return;
  E.busy=true;
  const changed=Object.fromEntries(dirty().map(p=>[p,E.files[p]]));
  const body={base_revision:E.revision};
  if(Object.keys(changed).length)body.files=changed;
  if(graphDirty())body.graph=E.editor.getGraph(); // structured save; the service serializes graph.yaml
  try{
    const r=await svc.kb.saveDraft(E.kind,E.id,body);
    E.revision=r.revision;E.findings=r.findings||[];E.orig={...E.files};E.conflict=null;E.external=false;
    if(E.editor)E.origGraphJson=JSON.stringify(E.editor.getGraph());
    toast('Draft saved');document.dispatchEvent(new CustomEvent('kb:changed'));
  }catch(e){
    if(e.code==='draft_conflict')await showConflict(e.details);
    else toast(e.message,true);
  }
  E.busy=false;draw();
}

async function showConflict(details){
  const theirs=await svc.kb.getDraft(E.kind,E.id);
  const changed=(details?.changed_files?.length?details.changed_files:[...dirty(),...(graphDirty()?['graph.yaml']:[])]);
  E.conflict={theirs,files:changed,revision:theirs.revision};
}

async function resolve(how){
  const c=E.conflict;if(!c)return;
  const theirFiles={...c.theirs.files};if(E.kind==='algopipe')delete theirFiles['graph.yaml'];
  if(how==='theirs'){
    Object.assign(E,{revision:c.theirs.revision,orig:{...theirFiles},files:{...theirFiles},findings:c.theirs.findings||[],conflict:null,external:false});
    if(E.kind==='algopipe')startGraphEditor(c.theirs.graph);
    draw();return;
  }
  // keep mine: re-submit my edits on top of the current on-disk revision.
  E.revision=c.revision;E.orig={...theirFiles};E.conflict=null;E.external=false;
  await save();
}

async function reloadExternal(){
  if(isDirty()&&!confirm('Reload from disk and lose your unsaved edits?'))return;
  await openDraft(E.kind,E.id);
}

async function discardDraft(){
  if(!confirm(`Delete the draft ${E.id}? Published versions are not affected.`))return;
  try{await svc.kb.discardDraft(E.kind,E.id);E.files=E.orig;E.mode=null;ensurePane().hidden=true;toast('Draft discarded');document.dispatchEvent(new CustomEvent('kb:changed'))}
  catch(e){toast(e.message,true)}
}

function publishHtml(){
  const pipe=E.kind==='algopipe',r=E.pubResult;
  return `<div class="kbp-pub"><b>Publish</b> <span class="dim">an immutable, hash-locked version. The draft continues as the next version.</span>
    ${pipe?`<div class="kbfld"><label>Release</label><span><label><input type="radio" name="kbp-rel" value="knowledge" ${E.release!=='executable'?'checked':''}> Knowledge — record the method; <b>not runnable</b></label><br>
      <label><input type="radio" name="kbp-rel" value="executable" ${E.release==='executable'?'checked':''}> Executable — every node verified; can start formal Runs</label></span></div>`:'<div class="note dim">A node is published as knowledge; runnability belongs to a published pipe.</div>'}
    <div class="kbfld"><label for="kbp-vdesc">Version description</label><input id="kbp-vdesc" placeholder="What this version is"></div>
    <button class="btn" id="kbp-publish" ${E.busy||isDirty()?'disabled':''}>Publish ${pipe&&E.release==='executable'?'executable':'knowledge'} release</button>
    ${r?`<div class="kbp-result"><b class="ok">Published ${esc(r.id)}@${esc(r.version)}</b>
      <div class="mono dim">content ${esc(r.content_id)}</div>${r.computational_identity?`<div class="mono dim">computational identity ${esc(r.computational_identity)}</div>`:''}
      <div>Dependencies: ${r.dependency_summary.total} (${r.dependency_summary.unresolved} unresolved)${r.verification_summary?.total!==undefined?` · verified nodes: ${r.verification_summary.verified}/${r.verification_summary.total}`:''}</div>
      ${r.disclosures.length?`<div class="note warnc">Disclosed: ${r.disclosures.map(d=>esc(d.replace(/_/g,' '))).join(', ')}. This does not block publication or Runs; it is recorded with the version.</div>`:''}
      ${r.release_kind==='knowledge'?'<div class="note warnc">Knowledge-only — not runnable. Becoming executable needs a new version.</div>':''}</div>`:''}</div>`;
}

async function publish(){
  if(isDirty()){toast('Save your changes before publishing',true);return}
  const note=$('#kbp-vdesc',pane)?.value.trim()||undefined;
  E.busy=true;draw();
  try{
    const release=E.kind==='algopipe'?(E.release||'knowledge'):'knowledge';
    const r=await svc.kb.publish(E.kind,E.id,{release,base_revision:E.revision,version_description:note});
    toast(`Published ${r.id}@${r.version} (${release==='executable'?'executable':'knowledge-only'})`);
    document.dispatchEvent(new CustomEvent('kb:changed'));
    await openDraft(E.kind,E.id);
    E.pubResult=r;draw();
  }catch(e){
    if(e.code==='not_publishable')E.findings=e.details?.findings||[];
    else toast(e.message,true);
    E.busy=false;draw();
    if(e.code==='not_publishable')toast('Not publishable — every blocking finding is listed',true);
    return;
  }
  E.busy=false;
}

async function duplicate(){
  const newId=$('#kbp-dupid',pane).value.trim()||E.id;
  try{
    await svc.kb.createDraft(E.kind,{id:newId,from:{id:E.id,version:E.version}});
    toast('Draft created');document.dispatchEvent(new CustomEvent('kb:changed'));
    await openDraft(E.kind,newId);
  }catch(e){toast(e.code==='identity_conflict'?'A draft with that ID already exists — choose another ID.':e.message,true)}
}

// ---- Preview (US4) --------------------------------------------------------
// Single-image, incremental, in-memory. Never a Run: nothing here is recorded
// as an official result, and the banner says so on every outcome.
const PV={running:false,seq:0,result:null,failure:null,error:null,inputUrl:null,inputFor:null,outputUrl:null,outputText:null,view:'output',stale:false,target:null,port:null};

async function refreshStatuses(){
  if(!E.editor||E.mode!=='draft')return;
  try{const r=await previewStatus(E.id);E.editor.setStatuses(r.nodes)}catch{}
}

async function runPreview(){
  const image=getImage();
  if(!image){toast('Open an image from the Dataset view first, then preview against it.',true);return}
  if(isDirty()){await save();if(isDirty())return} // Preview runs the *saved* revision, so it is reproducible from it
  const graph=E.editor.getGraph();
  const target=PV.target&&graph.nodes.some(n=>n.instance_id===PV.target)?PV.target:(E.editor.selected()||E.editor.lastNode());
  if(!target){toast('Add a node to preview.',true);return}
  PV.target=target;
  const my=++PV.seq;PV.running=true;PV.error=null;renderPreview();
  try{
    const r=await previewPipe(E.id,E.revision,image,target,PV.port||undefined);
    if(my!==PV.seq)return; // superseded by a newer request: never show an older result (FR-019)
    PV.stale=false;PV.failure=null;PV.result=null;
    if(r.state==='ready'){
      PV.result=r;PV.port=null;
      await loadArtifacts(image,r.artifact_ref,'output');
    }else{
      PV.failure=r;
      if(r.last_successful_artifact_ref)await loadArtifacts(image,r.last_successful_artifact_ref,'output');
      else{PV.outputUrl=null;PV.outputText=null}
    }
  }catch(e){
    if(my!==PV.seq)return;
    PV.error=e.code==='not_previewable'?e:null;
    if(e.code==='stale_reference')toast('The draft changed since it was opened — save it and preview again.',true);
    else if(e.code!=='not_previewable')toast(e.message,true);
    PV.result=null;PV.failure=null;
  }
  if(my===PV.seq)PV.running=false;
  await refreshStatuses();renderPreview();
}

async function loadArtifacts(image,ref,view){
  if(PV.inputFor!==image){
    try{const d=await svc.request('GET',`/image-assets/${image}/display`);PV.inputUrl=await svc.fetchArtifactObjectUrl(d.image_artifact_ref.id);PV.inputFor=image}catch{PV.inputUrl=null}
  }
  PV.outputUrl=null;PV.outputText=null;
  if(ref.kind==='measurement'){
    const blob=await svc.request('GET',`/artifacts/${ref.id}/content`);
    const rows=JSON.parse(await blob.text());
    PV.outputText=Object.entries(rows).map(([k,v])=>`${k}: ${Number.isInteger(v)?v:v.toFixed(4)}`).join('\n');
  }else PV.outputUrl=await svc.fetchArtifactObjectUrl(ref.id);
  PV.view=view;
}

function pvHtml(){
  const g=E.editor?E.editor.getGraph():{nodes:[]};
  const image=getImage();
  const nodes=g.nodes;const sel=PV.target||E.editor?.selected()||'';
  const r=PV.result,f=PV.failure;
  let banner='';
  if(PV.error)banner=`<div class="pv-banner failed">Not previewable — ${esc(PV.error.details?.nodes?.length?'these nodes cannot run: '+PV.error.details.nodes.join(', ')+' (specification only or unavailable)':(PV.error.details?.findings?.[0]?.explanation||PV.error.message))}</div>`;
  else if(f)banner=`<div class="pv-banner failed">Preview failed at ${esc(f.failing_node_id)} — ${esc(f.error.message)}${f.stale?' (showing the last successful result, now stale)':''}</div>`;
  else if(r)banner=`<div class="pv-banner ${r.verification_state}">${r.verification_state==='unverified'?'UNVERIFIED — preview only':'Preview only — every node in the chain is technically verified'}${r.verification_state==='unverified'&&r.unverified_nodes.length?` <span class="dim">(${esc(r.unverified_nodes.join(', '))} not verified)</span>`:''} · not a Run</div>`;
  const shown=r||f;
  const out=PV.outputUrl?`<img src="${esc(PV.outputUrl)}" alt="Preview output">`:PV.outputText?`<pre class="kbp-pre">${esc(PV.outputText)}</pre>`:'<span class="dim">No output yet.</span>';
  const inp=PV.inputUrl?`<img src="${esc(PV.inputUrl)}" alt="Input image">`:'<span class="dim">Input unavailable.</span>';
  const view=PV.view==='input'?`<figure><figcaption>Input</figcaption>${inp}</figure>`:PV.view==='before'?`<figure><figcaption>Before</figcaption>${inp}</figure><figure><figcaption>After</figcaption>${out}</figure>`:`<figure><figcaption>Output${r?` · ${esc(r.port)}`:''}</figcaption>${out}</figure>`;
  return `<div class="sh"><span>Preview</span><span class="mono dim">one image · never recorded as a Run</span></div>
    <div class="pv-bar"><span class="mono ${image?'':'dim'}">${image?'image '+esc(String(image).slice(0,8)):'no image open — pick one in Dataset'}</span>
      <label class="dim" for="pv-target">at</label><select id="pv-target">${nodes.map(n=>`<option value="${esc(n.instance_id)}" ${n.instance_id===sel?'selected':''}>${esc(n.instance_id)}</option>`).join('')}</select>
      <button class="btn" id="pv-run" ${PV.running||!nodes.length?'disabled':''}>${PV.running?'Running…':'Preview'}</button>
      <span class="seg" role="group" aria-label="View"><button data-pv="input" class="${PV.view==='input'?'on':''}">Input</button><button data-pv="output" class="${PV.view==='output'?'on':''}">Output</button><button data-pv="before" class="${PV.view==='before'?'on':''}">Before/after</button></span></div>
    ${banner}${shown?`<div class="pv-view ${PV.stale||(f&&f.stale)?'stale':''}">${PV.stale?'<span class="pv-stale">STALE — edited since</span>':f&&f.stale?'<span class="pv-stale">STALE</span>':''}${view}</div>`:''}`;
}

function renderPreview(){
  const slot=$('#kbp-pv',pane);if(!slot)return;
  slot.innerHTML=pvHtml();
  const run=$('#pv-run',slot);if(run)run.onclick=runPreview;
  const t=$('#pv-target',slot);if(t)t.onchange=()=>{PV.target=t.value};
  $$('[data-pv]',slot).forEach(b=>b.onclick=()=>{PV.view=b.dataset.pv;renderPreview()});
}

// ---- render ---------------------------------------------------------------
function headHtml(title,pills,actions){
  return `<div class="kbp-head"><div><b>${esc(title)}</b> <span class="mono dim">${esc(E.id||'')}${E.version?'@'+esc(E.version):''}</span> ${pills}</div><div class="kbtools">${actions}<button class="tb" id="kbp-close" title="Close">✕</button></div></div>`;
}

function draw(){
  const p=ensurePane();
  if(!E.mode){p.hidden=true;return}
  if(E.mode==='draft')p.innerHTML=draftHtml();
  else if(E.mode==='published')p.innerHTML=publishedHtml();
  else p.innerHTML=invalidHtml();
  bind();
}

function draftHtml(){
  const files=Object.keys(E.files);
  const n=changeCount();
  const banner=E.conflict?`<div class="kbp-banner"><b>This draft changed on disk since you opened it.</b> Nothing was overwritten. Compare, then choose explicitly.
      ${E.conflict.files.map(f=>`<div class="kbp-diff"><div><small>On disk — ${esc(f)}</small><pre>${esc(E.conflict.theirs.files[f]??'(missing)')}</pre></div><div><small>Yours — ${esc(f)}</small><pre>${esc(E.files[f]??(f==='graph.yaml'&&E.editor?JSON.stringify(E.editor.getGraph(),null,2):'(unchanged)'))}</pre></div></div>`).join('')}
      <div class="kbact"><button class="btn" id="kbp-keepmine">Keep mine (overwrite disk)</button><button class="btn ghost" id="kbp-keeptheirs">Keep theirs (discard my edits)</button><button class="btn ghost" id="kbp-later">Decide later</button></div></div>`
    :E.external?`<div class="kbp-banner">A file in this draft was edited outside Rosaray. <button class="btn ghost" id="kbp-reload">Reload from disk</button></div>`:'';
  const errs=E.findings.filter(f=>f.severity==='error').length;
  return headHtml(E.header?.name||E.id,`<span class="pill warnc">draft</span>${E.header?.origin==='paper_derived'?'<span class="pill warnc" title="Extracted from a paper; still a specification">paper-derived</span>':''}${errs?`<span class="pill errc">${errs} blocking</span>`:''}`,
    `<button class="btn" id="kbp-save" ${n&&!E.busy?'':'disabled'}>Save${n?` (${n})`:''}</button><button class="btn ghost" id="kbp-discard">Discard draft</button>`)+banner+`
    <div class="kbp-body">
      <div class="kbp-tabs">${E.kind==='algopipe'?`<button class="kbp-tab ${E.active===GRAPH?'on':''}" data-f="${GRAPH}">Graph${graphDirty()?' ●':''}</button>`:''}${files.map(f=>`<button class="kbp-tab ${f===E.active?'on':''}" data-f="${esc(f)}">${esc(f)}${E.files[f]!==E.orig[f]?' ●':''}</button>`).join('')}</div>
      <div class="note kbwarn">${esc(NO_PII)} ${esc(RESEARCH_ONLY)}</div>
      ${E.active===GRAPH?`<div id="kbp-gslot" class="kbp-gslot"></div>
        <details class="sec"><summary>Target data profile</summary><div class="note">The data this pipe is meant for, as JSON, e.g. <span class="mono">{"profile_version":1,"require":[{"predicate":"pixel_spacing_present","equals":true}]}</span></div>
        <textarea id="kbp-profile" class="kbp-profile" spellcheck="false" aria-label="Target data profile (JSON)">${esc(E.editor?JSON.stringify(E.editor.getGraph().target_data_profile||'',null,1).replace(/^""$/,''):'')}</textarea><div id="kbp-profile-err" class="errc kbmsg"></div></details><div class="pv" id="kbp-pv"></div>`:`<textarea id="kbp-text" spellcheck="false" aria-label="${esc(E.active||'file')}">${esc(E.files[E.active]??'')}</textarea>`}
      ${publishHtml()}
      ${findingsHtml(E.findings)}
    </div>`;
}

function publishedHtml(){
  const v=E.view,h=v.header||{};
  return headHtml(h.name||E.id,`<span class="pill ok">published</span>${E.kind==='algopipe'?`<span class="pill warnc">${esc(v.inspector?.release_kind==='executable'?'executable':'knowledge-only — not runnable')}</span>`:''}`,
    `<button class="btn ghost" id="kbp-export">Export…</button>`)+`<div class="kbp-body">
    <div class="note">Published versions are immutable. To change it, start a draft from it.</div>
    ${inspectorHtml(v.inspector)}
    ${E.kind==='algopipe'?runHtml():''}
    <details class="sec"><summary>Description</summary><pre class="kbp-pre">${esc(v.description||'')}</pre></details>
    <details class="sec"><summary>${v.contract?'Contract':'Graph'} (raw)</summary><pre class="kbp-pre">${esc(JSON.stringify(v.contract||v.graph||{},null,2))}</pre></details>
    <div class="kbp-pub"><b>Start a draft from this version</b><div class="kbfld"><label for="kbp-dupid">Draft ID</label><input id="kbp-dupid" value="${esc(E.id)}"></div>
      <button class="btn" id="kbp-dup">Duplicate as draft</button></div>
    ${findingsHtml(E.findings)}</div>`;
}

// ---- formal Run of a published pipe ------------------------------------------
const RUN={out:null};
const REASON={
  not_executable_release:'This version is a knowledge-only release and can never be run.',
  dependency_unavailable:'A node this version depends on is no longer available or has changed.',
  implementation_unavailable:'A node has no trusted implementation this installation can run.',
  verification_withdrawn:'A node’s technical verification was withdrawn or failed.',
  verification_missing:'A node has no technical verification for its exact implementation.',
  profile_unsatisfied:'The dataset does not satisfy the pipe’s target data profile.',
  prerequisite_unmet:'A node’s prerequisite is not met by the dataset.',
};
function runHtml(){
  const ready=getImage()&&getDatasetVersion();
  const o=RUN.out;
  const body=!o?'':o.error?`<div class="kbp-result"><b class="errc">Run refused — no Run was created</b>${(o.error.details?.reasons||[]).map(r=>`<div class="kbfind error"><span class="mono">${esc(r.code)}</span>${r.node?` · node <b>${esc(r.node)}</b>`:''}${r.predicate?` · <b>${esc(r.predicate)}</b>`:''}<div>${esc(REASON[r.code]||r.code)}</div>${r.detail?.observed!==undefined?`<div class="dim">expected ${esc(JSON.stringify(r.detail.expected))}, observed ${esc(JSON.stringify(r.detail.observed))}</div>`:''}</div>`).join('')||esc(o.error.message)}</div>`
    :`<div class="kbp-result"><b class="ok">Run ${esc(o.status)}</b> <span class="mono dim">${esc(o.run_id)}</span></div>`;
  return `<div class="kbp-pub"><b>Formal Run</b> <span class="dim">An official, recorded Run — not a Preview. Uses the open image and selected Dataset Version.</span>
    <div class="note ${ready?'dim':'warnc'}">${ready?'Ready: the open image and Dataset Version will be used.':'Open an image from the Dataset view first.'}</div>
    <button class="btn" id="kbp-run" ${ready?'':'disabled'}>Start Run</button>${body}</div>`;
}
async function startRun(){
  RUN.out=null;
  try{
    const r=await svc.request('POST','/runs',{dataset_version_id:getDatasetVersion(),image_asset_id:getImage(),algopipe:{id:E.id,version:E.version},seed:1});
    RUN.out={status:'running',run_id:r.run_id};draw();
    for(let n=0;n<100;n++){await new Promise(z=>setTimeout(z,300));const x=await svc.request('GET',`/runs/${r.run_id}`);if(x.status!=='running'){RUN.out={status:x.status,run_id:r.run_id};break}}
  }catch(e){RUN.out={error:e}}
  draw();
}

function invalidHtml(){
  const v=E.view;
  return headHtml(v.name||'Invalid bundle',`<span class="pill errc">invalid</span>`,'')+`<div class="kbp-body">
    <div class="note">This folder could not be read as a valid bundle. It is listed so it is not lost; fix it outside Rosaray, then refresh the catalog.</div>${findingsHtml(E.findings)}</div>`;
}

function bind(){
  const on=(id,fn)=>{const el=$('#'+id,pane);if(el)el.onclick=fn};
  on('kbp-run',startRun);on('kbp-close',close);on('kbp-save',save);on('kbp-discard',discardDraft);on('kbp-publish',publish);on('kbp-dup',duplicate);
  on('kbp-keepmine',()=>resolve('mine'));on('kbp-keeptheirs',()=>resolve('theirs'));on('kbp-reload',reloadExternal);on('kbp-later',()=>{E.conflict=null;draw()});
  on('kbp-export',()=>{if(E.onExport)E.onExport();else startExport({kind:E.kind,id:E.id,version:E.version})});
  $$('.kbp-tab',pane).forEach(t=>t.onclick=()=>{E.active=t.dataset.f;draw()});
  $$('input[name="kbp-rel"]',pane).forEach(r=>r.onchange=()=>{E.release=r.value;draw()});
  const slot=$('#kbp-gslot',pane);
  if(slot&&E.geHost){slot.appendChild(E.geHost);E.editor.refreshDefs();refreshStatuses();renderPreview()}
  const prof=$('#kbp-profile',pane);
  if(prof)prof.onchange=()=>{
    const t=prof.value.trim(),err=$('#kbp-profile-err',pane);
    if(!t){err.textContent='';E.editor.setProfile(null);return}
    try{E.editor.setProfile(JSON.parse(t));err.textContent=''}catch{err.textContent='Not valid JSON — the profile was not changed.'}
  };
  const ta=$('#kbp-text',pane);
  if(ta){ta.oninput=()=>{E.files[E.active]=ta.value;const s=$('#kbp-save',pane);updateDirtyUi();$$('.kbp-tab',pane).forEach(t=>{if(t.dataset.f!==GRAPH)t.textContent=t.dataset.f+(E.files[t.dataset.f]!==E.orig[t.dataset.f]?' ●':'')})};
    ta.onkeydown=e=>{if((e.metaKey||e.ctrlKey)&&e.key.toLowerCase()==='s'){e.preventDefault();save()}}}
}
