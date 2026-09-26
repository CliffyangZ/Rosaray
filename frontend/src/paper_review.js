// Paper → method candidates (feature 002, US5): import an authorized local PDF,
// run offline extraction, and review each candidate — accept, edit, reject, mark
// non-executable, map to an existing node, keep as a specification-only node, or
// defer. Every candidate shows its source page, section and quoted context, and
// anything the paper left unsaid is highlighted as an ambiguity: the UI never
// fills a gap. Results are labelled paper-derived (FR-027) and research-use-only.
import {$,$$,esc,toast} from './util.js';
import * as svc from './service_client.js';
import {loadNodeDefs} from './registry.js';
import {openDraft} from './draft_editor.js';
import {findingsHtml} from './kb_catalog.js';

const S={extractionId:null,papers:[],paper:null,candidates:[],filter:'',progress:null,mapping:null,editing:null,error:null,busy:false};
let host=null,pane=null,subscribed=false;

const CATS=['data_eligibility','preprocessing','model_inference','postprocessing','landmark_extraction','measurement','decision_rule','validation_only','rejected_content'];
const label=c=>String(c).replace(/_/g,' ');
const NO_PII='請勿輸入患者姓名或其他可辨識個人身分的資訊。';

async function loadPapers(){
  if(!svc.isConnected()||svc.currentSessionState()!=='ready'){S.papers=[];drawList();return}
  try{S.papers=(await svc.request('GET','/papers')).papers}catch{S.papers=[]}
  drawList();
}

// ---- left list + import ---------------------------------------------------
function drawList(){
  if(!host)return;
  const online=svc.isConnected()&&svc.currentSessionState()==='ready';
  host.innerHTML=`<div class="sh"><span>論文資料</span></div><div class="scroll">
    ${online?`<div class="kbform" style="padding-bottom:6px">
      <div class="kbfld"><label for="pr-file">PDF 檔案</label><input type="file" id="pr-file" accept="application/pdf,.pdf"></div>
      <div class="kbfld"><label for="pr-title">標題</label><input id="pr-title" placeholder="可選"></div>
      <label class="prcheck"><input type="checkbox" id="pr-auth"> 我有權使用此論文；檔案只保存在本機，不會傳送至外部服務。</label>
      <div class="kbact"><button class="btn" id="pr-import" disabled>匯入論文</button></div><div id="pr-err" class="errc kbmsg"></div></div>`
    :'<div class="note">Connect to the Local Rosaray Service to import papers.</div>'}
    <div class="note kbwarn">${esc(NO_PII)}</div>
    <div class="kblist">${S.papers.length?S.papers.map((p,i)=>`<div class="kbrow" data-i="${i}" tabindex="0" role="button"><div class="kbn"><b>${esc(p.title||'未命名論文')}</b><span class="mono dim">${p.page_count} p</span></div>
      <div class="kbb"><span class="pill ${p.extraction_state==='extracted'?'ok':p.extraction_state==='failed'?'errc':''}">${esc(p.extraction_state)}</span>${p.pages_without_text.length?`<span class="pill warnc" title="These pages have no text layer (scanned); no OCR is done">${p.pages_without_text.length} page(s) without text</span>`:''}</div></div>`).join(''):'<div class="empty">尚無論文。</div>'}</div></div>`;
  const imp=$('#pr-import',host);if(imp)imp.onclick=importPaper;
  const syncImport=()=>{if(imp)imp.disabled=!$('#pr-file',host)?.files?.length||!$('#pr-auth',host)?.checked};
  $('#pr-file',host)?.addEventListener('change',syncImport);
  $('#pr-auth',host)?.addEventListener('change',syncImport);
  $$('.kbrow',host).forEach(r=>{const open=()=>openPaper(S.papers[+r.dataset.i].paper_id);r.onclick=open;r.onkeydown=e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();open()}}});
}

async function importPaper(){
  const f=$('#pr-file',host).files[0],err=$('#pr-err',host);
  err.textContent='';
  if(!f){err.textContent='Choose a PDF file.';return}
  if(!$('#pr-auth',host).checked){err.textContent='Confirm you are authorized to use this paper.';return}
  try{
    const r=await svc.papers.importPaper(f,true);
    toast(r.already_imported?'That paper was already imported.':`Imported (${r.page_count} pages${r.pages_without_text.length?`, ${r.pages_without_text.length} without text`:''})`);
    await loadPapers();openPaper(r.paper_id);
  }catch(e){err.textContent=e.code==='authorization_required'?'Confirm you are authorized to use this paper.':e.code==='bundle_invalid'?'That file is not a readable PDF.':e.message}
}

// ---- review pane ------------------------------------------------------------
function ensurePane(){
  if(pane)return pane;
  pane=document.createElement('section');pane.id='kbreview';pane.hidden=true;
  ($('#center')||document.body).appendChild(pane);
  return pane;
}

async function openPaper(paperId){
  ensurePane();
  try{
    const r=await svc.papers.candidates(paperId);
    S.paper=r.paper;S.candidates=r.candidates;S.mapping=null;S.editing=null;S.error=null;
    pane.hidden=false;drawPane();
  }catch(e){toast(e.message,true)}
}

async function refresh(){if(S.paper)await openPaper(S.paper.paper_id);loadPapers()}

function srcHtml(s){return `<div class="prsrc"><span class="mono dim">p.${s.page}${s.section?' · '+esc(s.section):''}</span><blockquote>${esc(s.quote)}</blockquote></div>`}

function itemHtml(it){
  const v=it.value===null||it.value===undefined?'':`= ${esc(it.value)}`;
  const u=it.value!==null&&it.value!==undefined?(it.unit?` ${esc(it.unit)}`:' <span class="pramb">no unit stated</span>'):'';
  return `<span class="pritem"><b>${esc(it.name)}</b> ${v}${u}</span>`;
}

function candHtml(c,i){
  const claim=c.category==='clinical_claim';
  const p=c.proposed,any=[...p.inputs,...p.outputs,...p.parameters,...p.units,...p.assumptions].length||p.formula;
  const dis=t=>claim?`disabled title="A clinical statement can only be rejected or marked non-executable"`:'';
  return `<div class="prcand ${claim?'claim':''}" data-i="${i}">
    <div class="kbn"><span><span class="pill ${claim?'errc':''}">${esc(label(c.category))}</span> <span class="pill ${c.state==='proposed'?'':'ok'}">${esc(label(c.state))}</span> <span class="pill" title="Extracted from a paper; not a verified method">paper-derived</span></span><span class="mono dim">${c.sources.length} source${c.sources.length>1?'s':''}</span></div>
    ${c.sources.map(srcHtml).join('')}
    ${claim?'<div class="note errc">Looks like a diagnosis or treatment statement, not a reproducible analysis step. It cannot become a method.</div>':''}
    ${any?`<div class="prfields">${[['Inputs',p.inputs],['Outputs',p.outputs],['Parameters',p.parameters],['Assumptions',p.assumptions]].filter(([,l])=>l.length).map(([t,l])=>`<div><small>${t}</small> ${l.map(itemHtml).join('')}</div>`).join('')}${p.formula?`<div><small>Formula</small> <span class="mono">${esc(p.formula.value)}</span></div>`:''}</div>`:''}
    ${c.ambiguities.length?`<div class="prambs"><small>Unresolved in the source</small>${c.ambiguities.map(a=>`<div class="pramb" title="${esc(a.note)}"><span class="mono">${esc(a.code)}</span> ${esc(a.field)} — ${esc(a.note)}</div>`).join('')}</div>`:''}
    <div class="pract">
      <button class="btn" data-a="accept" ${dis()}>Accept as spec-only node</button>
      <button class="btn ghost" data-a="edit" ${dis()}>Edit…</button>
      <button class="btn ghost" data-a="map" ${dis()}>Map to node…</button>
      <button class="btn ghost" data-a="keep_specification_only" ${dis()}>Keep spec-only</button>
      <button class="btn ghost" data-a="mark_non_executable">Non-executable</button>
      <button class="btn ghost" data-a="reject">Reject</button>
      <button class="btn ghost" data-a="defer">Defer</button></div>
    ${S.editing===c.candidate_id?editHtml(c):''}${S.mapping===c.candidate_id?mapHtml():''}
    ${c.draft?'':''}</div>`;
}

function editHtml(c){
  const p=c.proposed;
  const rows=p.parameters.map((x,k)=>`<div class="predit"><input data-k="name" data-r="${k}" value="${esc(x.name)}" aria-label="Parameter name"><input data-k="value" data-r="${k}" value="${esc(x.value??'')}" aria-label="Value" placeholder="value"><input data-k="unit" data-r="${k}" value="${esc(x.unit??'')}" aria-label="Unit" placeholder="unit"></div>`).join('');
  return `<div class="predits"><div class="note kbwarn">${esc(NO_PII)} Values you add here are yours, not the paper's.</div>
    <div class="kbfld"><label for="pe-cat">Category</label><select id="pe-cat">${CATS.map(x=>`<option ${x===c.category?'selected':''}>${x}</option>`).join('')}</select></div>
    ${rows||'<div class="empty">No parameters proposed.</div>'}
    <div class="kbfld"><label for="pe-in">Inputs</label><input id="pe-in" value="${esc(p.inputs.map(x=>x.name).join(', '))}" placeholder="comma separated"></div>
    <div class="kbfld"><label for="pe-out">Outputs</label><input id="pe-out" value="${esc(p.outputs.map(x=>x.name).join(', '))}" placeholder="comma separated"></div>
    <div class="kbact"><button class="btn" id="pe-save">Save edit</button><button class="btn ghost" id="pe-cancel">Cancel</button></div></div>`;
}

function mapHtml(){
  return `<div class="predits"><div class="kbfld"><label for="pm-node">Node</label><select id="pm-node"><option value="">Choose a published node…</option></select></div>
    <div id="pm-find"></div><div class="kbact"><button class="btn" id="pm-go">Map</button><button class="btn ghost" id="pm-cancel">Cancel</button></div></div>`;
}

function drawPane(){
  const p=ensurePane(),paper=S.paper;
  if(!paper){p.hidden=true;return}
  const cs=S.filter?S.candidates.filter(c=>c.state===S.filter):S.candidates;
  const extracting=paper.extraction_state==='extracting';
  p.innerHTML=`<div class="kbp-head"><div><b>${esc(paper.title||'Untitled paper')}</b> <span class="mono dim">${paper.page_count} pages</span> <span class="pill ${paper.extraction_state==='extracted'?'ok':''}">${esc(paper.extraction_state)}</span></div>
      <div class="kbtools"><button class="btn" id="pr-extract" ${extracting?'disabled':''}>${paper.extraction_state==='extracted'?'Extract again':'Extract candidates'}</button>${extracting?'<button class="btn ghost" id="pr-cancel">Cancel</button>':''}<button class="tb" id="pr-close" title="Close">✕</button></div></div>
    <div class="kbp-body">
      <div class="note kbwarn">Research use only. Extraction is rule-based and offline; it proposes, you decide. Nothing here is published or run.</div>
      ${paper.pages_without_text.length?`<div class="note warnc">Pages ${esc(paper.pages_without_text.join(', '))} have no extractable text (scanned images). No OCR is done, so nothing was proposed from them.</div>`:''}
      ${S.progress?`<div class="note">Extracting… page ${S.progress.page} of ${S.progress.count}</div>`:''}
      <div class="pr-filter"><label class="dim" for="pr-state">Show</label><select id="pr-state"><option value="">all (${S.candidates.length})</option>${['proposed','accepted','edited','mapped','non_executable','rejected','deferred'].map(s=>`<option value="${s}" ${S.filter===s?'selected':''}>${label(s)}</option>`).join('')}</select></div>
      ${cs.length?cs.map(candHtml).join(''):`<div class="empty">${paper.extraction_state==='extracted'?'No candidates.':'Nothing extracted yet.'}</div>`}
      ${S.error?findingsHtml(S.error):''}
    </div>`;
  bindPane();
}

function bindPane(){
  const on=(id,fn)=>{const e=$('#'+id,pane);if(e)e.onclick=fn};
  on('pr-close',()=>{pane.hidden=true;S.paper=null});
  on('pr-extract',async()=>{try{const r=await svc.papers.extract(S.paper.paper_id);S.extractionId=r.extraction_id;S.paper.extraction_state='extracting';S.progress=null;drawPane()}catch(e){toast(e.message,true)}});
  on('pr-cancel',async()=>{if(!S.extractionId)return;try{await svc.papers.cancelExtraction(S.paper.paper_id,S.extractionId)}catch(e){toast(e.message,true)}});
  const f=$('#pr-state',pane);if(f)f.onchange=()=>{S.filter=f.value;drawPane()};
  const shown=S.filter?S.candidates.filter(c=>c.state===S.filter):S.candidates;
  $$('.prcand',pane).forEach(el=>{
    const c=shown[+el.dataset.i];
    $$('[data-a]',el).forEach(b=>b.onclick=()=>act(c,b.dataset.a));
  });
  const save=$('#pe-save',pane);
  if(save){const el=save.closest('.prcand');const c=shown[+el.dataset.i];
    save.onclick=()=>saveEdit(c,el);$('#pe-cancel',pane).onclick=()=>{S.editing=null;drawPane()}}
  const go=$('#pm-go',pane);
  if(go){const c=shown[+go.closest('.prcand').dataset.i];
    loadNodeDefs().then(defs=>{const sel=$('#pm-node',pane);if(sel)sel.innerHTML='<option value="">Choose a published node…</option>'+defs.map(d=>`<option value="${esc(d.id)}@${esc(d.version)}">${esc(d.name)} · ${esc(d.id)}@${esc(d.version)}</option>`).join('')}).catch(()=>{});
    go.onclick=()=>doMap(c);$('#pm-cancel',pane).onclick=()=>{S.mapping=null;S.error=null;drawPane()}}
}

async function decide(c,body){
  try{
    const r=await svc.papers.decide(c.candidate_id,body);
    S.error=null;
    if(r.draft){toast(`Draft ${r.draft.id} ${body.action==='accept'||body.action==='keep_specification_only'?'saved (specification only)':'updated'}`);document.dispatchEvent(new CustomEvent('kb:changed'))}
    await refresh();
    return r;
  }catch(e){
    if(e.code==='clinical_claim_not_executable')toast('A clinical statement cannot become a method.',true);
    else if(e.code==='draft_conflict')toast('The draft was edited since; open it and reconcile — nothing was overwritten.',true);
    else if(e.details?.findings){S.error=e.details.findings;drawPane();toast('Not compatible — see the findings.',true)}
    else toast(e.message,true);
  }
}

async function act(c,a){
  if(a==='edit'){S.editing=c.candidate_id;S.mapping=null;drawPane();return}
  if(a==='map'){S.mapping=c.candidate_id;S.editing=null;drawPane();return}
  const r=await decide(c,{action:a});
  if(r?.draft&&confirm(`Open the new draft ${r.draft.id}?`))openDraft('algonode',r.draft.id);
}

async function saveEdit(c,el){
  const proposed=JSON.parse(JSON.stringify(c.proposed));
  proposed.parameters.forEach((p,k)=>{
    const g=key=>el.querySelector(`[data-k="${key}"][data-r="${k}"]`).value.trim();
    p.name=g('name')||p.name;
    const v=g('value');p.value=v===''?null:(isNaN(Number(v))?v:Number(v));
    p.unit=g('unit')||null;
  });
  const names=id=>$('#'+id,el).value.split(',').map(s=>s.trim()).filter(Boolean);
  const keep=(list,ns)=>ns.map(n=>list.find(x=>x.name===n)||{name:n,value:null,unit:null,source_span:null});
  proposed.inputs=keep(proposed.inputs,names('pe-in'));proposed.outputs=keep(proposed.outputs,names('pe-out'));
  S.editing=null;
  await decide(c,{action:'edit',edited:{category:$('#pe-cat',el).value,proposed}});
}

async function doMap(c){
  const v=$('#pm-node',pane).value;if(!v){toast('Choose a node to map to.',true);return}
  const [id,version]=v.split('@');
  S.mapping=null;
  await decide(c,{action:'map_to_node',map_to:{id,version}});
}

export function mount(container){
  host=container;ensurePane();
  if(!subscribed){
    subscribed=true;
    svc.onEventType(['extraction_progress'],p=>{if(S.paper&&p.paper_id===S.paper.paper_id){S.progress={page:p.page,count:p.page_count};if(!pane.hidden)drawPane()}});
    svc.onEventType(['extraction_complete','extraction_cancelled'],()=>{S.progress=null;refresh()});
  }
  drawList();loadPapers();
}
