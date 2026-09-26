// Node / pipe inspector (feature 002, US3): what a researcher needs to decide
// whether to trust a method. Five status dimensions are shown side by side and
// deliberately never merged into one "verified"; the three evidence types stay
// in separate groups; unknown information reads "unknown" rather than being
// hidden. Every view carries the research-use-only label (FR-033).
import {$,$$,esc,toast} from './util.js';
import * as svc from './service_client.js';

export const RESEARCH_ONLY='Research use only — not for diagnosis or treatment.';
let overlay=null,currentImage=null,currentVersion=null,current=null;
export const getDatasetVersion=()=>currentVersion;
export const setDatasetVersion=id=>{currentVersion=id||null};

/** The Image Asset the researcher has open, to check node prerequisites against. */
export const getImage=()=>currentImage;
export function setImage(imageAssetId){currentImage=imageAssetId||null;if(overlay&&!overlay.hidden&&current)openInspector(current.kind,current.id,current.version)}

const DIM={
  maturity:{label:'Maturity',help:'How far the method has been implemented: specification only → implemented → technically verified.'},
  technical_verification:{label:'Technical verification',help:'Whether this exact definition and implementation passed its own tests.'},
  dataset_validation:{label:'Dataset validation',help:'Whether it was validated against a reference standard on a dataset. Separate from technical verification.'},
  availability:{label:'Availability',help:'Whether this version can be used: available, deprecated or unavailable.'},
  deprecation:{label:'Deprecation',help:'A notice appended to the version’s history; the version itself is never changed.'},
};
const tone=(dim,v)=>{
  if(dim==='availability')return v==='available'?'ok':v==='deprecated'?'warnc':'errc';
  if(dim==='technical_verification'||dim==='dataset_validation')return v==='passed'?'ok':(v==='failed'||v==='withdrawn'||v==='stale')?'errc':'';
  if(dim==='maturity')return v==='technically_verified'?'ok':v==='implemented'?'warnc':'';
  return '';
};
const label=v=>String(v??'unknown').replace(/_/g,' ');

function statusHtml(st){
  return `<div class="nis" role="list" aria-label="Status dimensions">${Object.entries(DIM).map(([k,d])=>{
    const raw=st[k];const v=k==='deprecation'?(raw?.deprecated?'deprecated':'none'):raw;
    const extra=k==='deprecation'&&raw?.deprecated?`<div class="dim">${esc(raw.reason||'')}${raw.replacement?` → ${esc(raw.replacement.id)}@${esc(raw.replacement.version)}`:''}</div>`:'';
    return `<div class="nisc" role="listitem" title="${esc(d.help)}"><small>${d.label}</small><b class="${tone(k,v)}">${esc(label(v))}</b>${extra}</div>`;
  }).join('')}</div>`;
}

const kv=(k,v)=>`<div class="nikv"><small>${esc(k)}</small><div>${v===undefined||v===null||v===''?'<span class="dim">unknown</span>':esc(v)}</div></div>`;

function portsHtml(ports){
  if(!ports)return '';
  const rows=(l,dir)=>l.map(p=>`<tr><td class="mono">${dir} ${esc(p.port_id)}</td><td>${esc(p.artifact_kind)}</td><td>${esc(p.unit)}</td><td>${esc(p.coordinate_space)}</td><td>${esc(p.calibration)}</td></tr>`).join('');
  return `<details class="sec" open><summary>Ports</summary><table class="tbl"><thead><tr><th>Port</th><th>Kind</th><th>Unit</th><th>Space</th><th>Calibration</th></tr></thead><tbody>${rows(ports.inputs,'in')}${rows(ports.outputs,'out')}${!ports.inputs.length&&!ports.outputs.length?'<tr><td colspan="5" class="dim">No ports declared.</td></tr>':''}</tbody></table></details>`;
}
function paramsHtml(ps){
  if(!ps)return '';
  return `<details class="sec" open><summary>Parameters <span class="count">${ps.length}</span></summary>${ps.length?`<table class="tbl"><thead><tr><th>Parameter</th><th>Type</th><th>Allowed / range</th><th>Unit</th><th>Default</th></tr></thead><tbody>${ps.map(p=>`<tr><td class="mono">${esc(p.parameter_id)}${p.required?' *':''}</td><td>${esc(p.type)}</td><td>${p.allowed?esc(p.allowed.join(', ')):p.range?esc(`${p.range.min??'−∞'} – ${p.range.max??'∞'}`):'<span class="dim">unknown</span>'}</td><td>${esc(p.unit||'unknown')}</td><td>${p.default===undefined||p.default===null?'—':esc(p.default)}</td></tr>`).join('')}</tbody></table>`:'<div class="empty">No parameters.</div>'}</details>`;
}
function evidenceHtml(ev){
  if(!ev)return '';
  const grp=(title,items,help)=>`<div class="niev"><b>${title}</b> <span class="dim">${esc(help)}</span>${items.length?items.map(e=>`<div class="row mono"><span class="nm">${esc(e.evidence_id)}</span><span class="pill">${esc(e.patient_data_status)}</span></div>`).join(''):'<div class="empty">None recorded.</div>'}</div>`;
  return `<details class="sec" open><summary>Evidence</summary>
    ${grp('Method source',ev.method_source,'where the method is described')}
    ${grp('Technical verification',ev.technical_verification,'tests of this implementation')}
    ${grp('Dataset validation',ev.dataset_validation,'validation against a reference on data')}</details>`;
}
function compatHtml(c){
  if(!c)return `<div class="note dim">${currentImage?'':'Open an image from the Dataset view to check this node’s prerequisites against it.'}</div>`;
  if(c.compatible)return `<div class="note ok">The selected image satisfies this node’s prerequisites.</div>`;
  return `<div class="nicompat"><b class="errc">Not compatible with the selected image</b>${c.unmet.map(u=>`<div class="kbfind error"><div><span class="pill errc">error</span> <span class="mono">${esc(u.code)}</span> <span class="mono dim">${esc(u.predicate)}</span></div><div>${esc(u.explanation)}</div><div class="dim">→ ${esc(u.action)}</div></div>`).join('')}</div>`;
}
function implHtml(i){
  if(!i)return '';
  const t={none:'No implementation — this is a specification only.',untrusted:'An implementation is declared but not trusted on this machine, so it will not run.',unavailable:'An implementation is declared that this installation cannot run.',available:'A trusted built-in implementation is available.'};
  return `<div class="nikv"><small>Implementation</small><div><b>${esc(label(i.status))}</b> <span class="dim">${esc(t[i.status]||'')}</span>${i.declared?`<div class="mono dim">${esc(i.declared.implementation_id)} v${esc(i.declared.implementation_version)} · trust: ${esc(i.effective_trust)}</div>`:''}</div></div>`;
}
function historyHtml(h,am){
  const rows=[...(h||[]).map(x=>`<div class="row mono"><span class="nm">verification · ${esc(x.type||'')} · ${esc(x.event)}</span><span class="dim">${esc((x.at||'').slice(0,10))}</span></div>`),...(am||[]).map(x=>`<div class="row mono"><span class="nm">${esc(x.kind)}${x.reason?' — '+esc(x.reason):''}</span><span class="dim">${esc((x.at||'').slice(0,10))}</span></div>`)];
  return `<details class="sec"><summary>History <span class="count">${rows.length}</span></summary>${rows.join('')||'<div class="empty">No amendments or verification events.</div>'}</details>`;
}

/** The inspector body for `GET /kb/{kind}/{id}/{ver}`'s `inspector` block. */
export function inspectorHtml(i){
  if(!i)return '';
  const isPipe=i.kind==='algopipe';
  return `<div class="ni">
    <div class="note kbwarn">${esc(RESEARCH_ONLY)}</div>
    ${statusHtml(i.status)}
    <div class="nigrid">${kv('Purpose',i.purpose)}${kv('Intended use',i.intended_use)}${kv('Limitations',i.limitations)}${kv('Domain',i.domain)}${kv('Version',i.version)}${kv('Content ID',i.content_id)}</div>
    ${isPipe?`<div class="nikv"><small>Release</small><div><b class="${i.release_kind==='executable'?'ok':'warnc'}">${esc(i.release_kind==='executable'?'executable':'knowledge-only — not runnable')}</b></div></div>
      <details class="sec" open><summary>Dependencies <span class="count">${(i.dependencies||[]).length}</span></summary>${(i.dependencies||[]).map(d=>`<div class="row mono"><span class="nm">${esc(d.instance_id)} → ${esc(d.ref)}</span><span class="pill ${d.availability==='available'?'ok':'errc'}">${esc(d.availability)}</span></div>`).join('')}</details>`
    :`${implHtml(i.implementation)}${portsHtml(i.ports)}${paramsHtml(i.parameters)}
      <details class="sec"><summary>Prerequisites</summary>${(i.prerequisites||[]).length?i.prerequisites.map(p=>`<div class="row mono"><span class="nm">${esc(p.predicate)}</span><span class="dim">${esc(JSON.stringify(p.args||{}))}</span></div>`).join(''):'<div class="empty">None declared.</div>'}</details>
      <div class="nigrid">${kv('Deterministic',i.reproducibility?.deterministic)}${kv('Cacheable',i.reproducibility?.cacheable)}${kv('Needs seed',i.reproducibility?.needs_seed)}</div>
      ${compatHtml(i.compatibility)}${evidenceHtml(i.evidence)}`}
    ${historyHtml(i.verification_history,i.amendments)}
  </div>`;
}

function ensureOverlay(){
  if(overlay)return overlay;
  overlay=document.createElement('section');overlay.id='kbinspect';overlay.hidden=true;
  ($('#center')||document.body).appendChild(overlay);
  return overlay;
}
export function mount(container){ensureOverlay()}
function close(){ensureOverlay().hidden=true;current=null}

/** Opens the inspector for a published version as an overlay. */
export async function openInspector(kind,id,version){
  const o=ensureOverlay();current={kind,id,version};
  try{
    const q=currentImage?`?image_asset_id=${encodeURIComponent(currentImage)}`:'';
    const v=await svc.request('GET',`/kb/${kind}/${id}/${version}${q}`);
    const i=v.inspector;
    o.innerHTML=`<div class="kbp-head"><div><b>${esc(i.name==='unknown'?id:i.name)}</b> <span class="mono dim">${esc(id)}@${esc(version)}</span></div><div class="kbtools"><button class="tb" id="ni-close" title="Close">✕</button></div></div>
      <div class="kbp-body">${inspectorHtml(i)}${(i.findings||[]).length?`<details class="sec"><summary>Findings <span class="count">${i.findings.length}</span></summary>${i.findings.map(f=>`<div class="kbfind ${esc(f.severity)}"><span class="mono">${esc(f.code)}</span> ${esc(f.explanation)}</div>`).join('')}</details>`:''}</div>`;
    o.hidden=false;$('#ni-close',o).onclick=close;
  }catch(e){toast(e.code==='not_found'?'That version is not in the knowledge base (open the published version, not a draft).':e.message,true)}
}
