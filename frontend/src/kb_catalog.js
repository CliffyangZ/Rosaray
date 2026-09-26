// Read-only catalog of methods authored by the local Rosaray Service.
import {$,$$,esc} from './util.js';
import * as svc from './service_client.js';
import {inspectorHtml} from './node_inspector.js';

const S={entries:[],q:'',kind:'',release:'',loading:false,error:null,selected:null,detail:null,detailError:null};
let host=null,detailHost=null,searchTimer=null,listRequest=0,detailRequest=0,subscribed=false;
const key=e=>[e.kind,e.id,e.version||'',e.status].join(':');
const pretty=v=>String(v||'').replace(/_/g,' ');

export async function load(){
  const request=++listRequest;
  if(!svc.isConnected()||svc.currentSessionState()!=='ready'){
    ++detailRequest;
    S.entries=[];S.selected=null;S.detail=null;S.detailError=null;
    S.error='請連線至本機 Rosaray Service，才能查看演算法。';S.loading=false;drawList();drawDetail();return;
  }
  S.loading=true;drawList();
  try{
    const filters={q:S.q,kind:S.kind,release:S.release==='draft'?'':S.release,status:S.release==='draft'?'draft':'',limit:200};
    const entries=[];
    let after;
    do{
      const r=await svc.kb.listEntries({...filters,after});
      if(request!==listRequest)return;
      entries.push(...(r.entries||[]));
      after=r.next;
    }while(after);
    if(request!==listRequest)return;
    S.entries=entries;S.error=null;
    if(S.selected&&!S.entries.some(e=>key(e)===key(S.selected))){S.selected=null;S.detail=null;S.detailError=null;drawDetail()}
    else if(S.selected)select(S.entries.find(e=>key(e)===key(S.selected)));
  }catch(e){
    if(request!==listRequest)return;
    ++detailRequest;
    S.entries=[];S.selected=null;S.detail=null;S.detailError=null;
    S.error=e.message||'無法載入演算法目錄。';drawDetail();
  }
  S.loading=false;drawList();
}

function badges(e){
  const items=[`<span class="pill ${e.status==='invalid'?'errc':e.status==='draft'?'warnc':'ok'}">${esc(pretty(e.status))}</span>`];
  if(e.kind==='algopipe'&&e.status==='published')items.push(`<span class="pill ${e.release_kind==='executable'?'ok':'warnc'}">${esc(e.release_kind==='executable'?'executable':'knowledge only')}</span>`);
  if(e.availability&&e.availability!=='available')items.push(`<span class="pill errc">${esc(pretty(e.availability))}</span>`);
  if(e.verification&&e.verification!=='none')items.push(`<span class="pill">${esc(pretty(e.verification))}</span>`);
  return items.join('');
}

function drawList(){
  if(!host)return;
  const group=(kind,title)=>{
    const items=S.entries.map((entry,index)=>({entry,index})).filter(x=>x.entry.kind===kind);
    return items.length?`<details class="kb-group" open><summary>${title}<span>${items.length}</span></summary>${items.map(({entry:e,index})=>`<button class="kbrow kbrow-button ${S.selected&&key(e)===key(S.selected)?'selected':''}" data-i="${index}" aria-pressed="${!!S.selected&&key(e)===key(S.selected)}"><span class="kbn"><b>${esc(e.name||e.id||'未命名')}</b><span class="mono dim">${esc(e.id||'—')}${e.version?'@'+esc(e.version):''}</span></span><span class="kbb">${badges(e)}</span></button>`).join('')}</details>`:'';
  };
  const body=S.loading?'<div class="empty">載入中…</div>':S.error?`<div class="note errc">${esc(S.error)}</div>`:S.entries.length?group('algopipe','AlgoPipes')+group('algonode','AlgoNodes')+group(null,'其他項目'):'<div class="empty">沒有符合條件的演算法。後端建立或發佈後會顯示在這裡。</div>';
  host.innerHTML=`<div class="sh"><span>演算法目錄</span><button class="tb" id="kb-refresh" title="重新讀取目錄" aria-label="重新讀取目錄">⟳</button></div>
    <div class="kb-subhead">由本機服務提供 · 唯讀</div><div class="scroll"><input class="search" id="kb-q" placeholder="搜尋名稱、用途或 ID" aria-label="搜尋演算法" value="${esc(S.q)}">
    <div class="kb-catalog-filters"><label>種類<select id="kb-kind"><option value="">全部</option><option value="algopipe" ${S.kind==='algopipe'?'selected':''}>AlgoPipe</option><option value="algonode" ${S.kind==='algonode'?'selected':''}>AlgoNode</option></select></label>
    <label>版本<select id="kb-release"><option value="">全部</option><option value="draft" ${S.release==='draft'?'selected':''}>草稿</option><option value="knowledge" ${S.release==='knowledge'?'selected':''}>知識版本</option><option value="executable" ${S.release==='executable'?'selected':''}>可執行</option></select></label></div>
    <div class="kblist">${body}</div></div>`;
  $('#kb-refresh',host).onclick=()=>load();
  $('#kb-q',host).oninput=e=>{
    S.q=e.target.value;clearTimeout(searchTimer);
    searchTimer=setTimeout(()=>{load().then(()=>{const input=$('#kb-q',host);input?.focus();input?.setSelectionRange(input.value.length,input.value.length)})},250);
  };
  $('#kb-kind',host).onchange=e=>{S.kind=e.target.value;load()};
  $('#kb-release',host).onchange=e=>{S.release=e.target.value;load()};
  $$('.kbrow[data-i]',host).forEach(row=>{row.onclick=()=>select(S.entries[Number(row.dataset.i)])});
}

function findingsHtml(findings){
  if(!findings?.length)return '';
  return `<details class="sec" open><summary>檢查結果 <span class="count">${findings.length}</span></summary>${findings.map(f=>`<div class="kbfind ${esc(f.severity||'')}"><span class="mono">${esc(f.code||'')}</span><div>${esc(f.explanation||'')}</div>${f.action?`<div class="dim">${esc(f.action)}</div>`:''}</div>`).join('')}</details>`;
}
function jsonSection(title,value){
  if(value===null||value===undefined)return '';
  return `<details class="sec"><summary>${title}</summary><pre class="kbp-pre">${esc(JSON.stringify(value,null,2))}</pre></details>`;
}
function drawDetail(){
  if(!detailHost)return;
  const e=S.selected;
  if(!e){detailHost.innerHTML='<div class="kb-detail-empty"><h2>選取演算法</h2><p>從左側目錄查看後端建立的 AlgoPipe 與 AlgoNode。</p></div>';return}
  const d=S.detail;
  detailHost.innerHTML=`<div class="kb-detail-head"><div><div class="eyebrow">${esc(e.kind==='algopipe'?'ALGOPIPE':'ALGONODE')} · ${esc(pretty(e.status))}</div><h2>${esc(d?.header?.name||e.name||e.id||'未命名')}</h2><span class="mono dim">${esc(e.id||'—')}${e.version?'@'+esc(e.version):''}</span></div><div class="kbb">${badges(e)}</div></div>
    ${S.detailError?`<div class="note errc">${esc(S.detailError)}</div>`:!d?'<div class="note">載入詳細資料中…</div>':`<div class="kb-detail-body">
      ${d.inspector?inspectorHtml(d.inspector):'<div class="note">此項目由後端管理，前端僅供檢視。</div>'}
      ${d.description?`<details class="sec" open><summary>說明</summary><pre class="kbp-pre">${esc(d.description)}</pre></details>`:''}
      ${jsonSection('Graph',d.graph)}${jsonSection('Contract',d.contract)}
      ${d.files?`<details class="sec"><summary>草稿檔案</summary>${Object.entries(d.files).map(([name,body])=>`<details class="sec"><summary>${esc(name)}</summary><pre class="kbp-pre">${esc(body)}</pre></details>`).join('')}</details>`:''}
      ${findingsHtml(d.findings)}</div>`}`;
}
async function select(entry){
  if(!entry)return;
  S.selected=entry;S.detail=null;S.detailError=null;drawList();drawDetail();
  const request=++detailRequest;
  try{
    let detail;
    if(entry.status==='invalid'){
      const r=await svc.request('GET',`/kb/findings?status=invalid${entry.id?`&id=${encodeURIComponent(entry.id)}`:''}`);
      const match=(r.bundles||[]).find(x=>x.id===entry.id&&x.kind===entry.kind&&x.version===entry.version);
      detail={findings:match?.findings||[]};
    }else if(entry.status==='draft')detail=await svc.kb.getDraft(entry.kind,entry.id);
    else detail=await svc.kb.getBundle(entry.kind,entry.id,entry.version);
    if(request!==detailRequest)return;
    S.detail=detail;
  }catch(err){if(request!==detailRequest)return;S.detailError=err.message||'無法載入詳細資料。'}
  drawDetail();
}
export function mount(container,details){
  host=container;detailHost=details;
  if(!subscribed){subscribed=true;svc.onEventType(['kb_scan_complete'],()=>load())}
  drawList();drawDetail();load();
}
