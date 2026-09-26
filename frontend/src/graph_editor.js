// AlgoPipe graph editor (feature 002, US2): add/remove/move/connect node
// instances through named ports, edit parameters, and see validation findings
// on the exact node / port / edge they belong to. Every computational edit is
// re-validated by the service (debounced); presentation-only edits (moving a
// node) never trigger validation. A client-side command stack gives undo/redo
// (FR-008) — the working graph is sent to the stateless validate endpoint, so
// validation state always matches the graph after each step.
import {esc} from './util.js';

const NODE_W=196,HEAD_H=26,ROW_H=20,UNDO_LIMIT=200;
const clone=x=>JSON.parse(JSON.stringify(x));

/** The part of a graph that affects computation: refs, parameters, edges. */
export function computationalKey(g){
  const nodes=[...g.nodes].sort((a,b)=>a.instance_id.localeCompare(b.instance_id)).map(n=>[n.instance_id,n.ref.id,n.ref.version,n.ref.content_id||'',n.parameters||{}]);
  const edges=g.edges.map(e=>e.from+'>'+e.to).sort();
  return JSON.stringify([nodes,edges,g.target_data_profile||null]);
}

const portsOf=(def,dir)=>def?.contract?.[dir==='in'?'inputs':'outputs']||[];
const shortName=id=>id.split('.').pop().replace(/[^a-z0-9]/gi,'');

export function createGraphEditor(opts){
  const {container,defs,validate,onChange}=opts;
  let G=normalize(clone(opts.graph||{schema:'quantify-kb/1',nodes:[],edges:[]}));
  let statuses={},undo=[],redo=[],sel=null,pending=null,report=null,seq=0,timer=null,lastKey=null,lastValidated=null,drag=null,domain=opts.domain||null;

  function normalize(g){
    g.schema=g.schema||'quantify-kb/1';g.nodes=g.nodes||[];g.edges=g.edges||[];
    g.nodes.forEach((n,i)=>{if(!n.layout||typeof n.layout.x!=='number')n.layout={x:30+(i%3)*230,y:30+Math.floor(i/3)*150}});
    return g;
  }
  const defOf=n=>defs(n.ref.id,n.ref.version);

  container.classList.add('gedit');container.tabIndex=0;
  container.innerHTML=`<div class="ge-bar"><select id="ge-pick" aria-label="Node to add"></select><button class="btn" id="ge-add">Add node</button>
    <button class="btn ghost" id="ge-undo" title="Undo (Ctrl/⌘+Z)">↶ Undo</button><button class="btn ghost" id="ge-redo" title="Redo (Ctrl/⌘+Shift+Z)">↷ Redo</button>
    <button class="btn ghost" id="ge-del" title="Delete selected (Del)">Delete</button><span class="sp"></span><span id="ge-state" class="mono dim"></span></div>
    <div class="ge-main"><div class="ge-canvas" id="ge-canvas"><svg class="ge-edges" id="ge-svg"></svg><div id="ge-nodes"></div></div>
    <div class="ge-side"><div id="ge-params"></div><div id="ge-finds"></div></div></div>`;
  const $=s=>container.querySelector(s);

  function fillPicker(){
    const list=defs.list?defs.list():[];
    $('#ge-pick').innerHTML=list.length?list.map(d=>`<option value="${esc(d.id)}@${esc(d.version)}">${esc(d.name)} · ${esc(d.id)}@${esc(d.version)}</option>`).join(''):'<option value="">No nodes in the knowledge base</option>';
    $('#ge-add').disabled=!list.length;
  }

  // ---- commands ---------------------------------------------------------
  function commit(mutate,label){
    const before=JSON.stringify(G);
    const next=clone(G);mutate(next);
    if(JSON.stringify(next)===before)return;
    undo.push(before);if(undo.length>UNDO_LIMIT)undo.shift();redo=[];
    G=next;afterChange(label);
  }
  function step(from,to){
    if(!from.length)return;
    to.push(JSON.stringify(G));G=JSON.parse(from.pop());
    if(sel?.type==='node'&&!G.nodes.some(n=>n.instance_id===sel.id))sel=null;
    if(sel?.type==='edge'&&!G.edges[sel.idx])sel=null;
    pending=null;afterChange('history');
  }
  const doUndo=()=>step(undo,redo),doRedo=()=>step(redo,undo);

  function afterChange(){
    draw();onChange&&onChange(clone(G));
    if(computationalKey(G)!==lastKey)scheduleValidate();
  }

  function scheduleValidate(){
    clearTimeout(timer);$('#ge-state').textContent='validating…';
    timer=setTimeout(runValidate,150);
  }
  async function runValidate(){
    const my=++seq,key=computationalKey(G),graph=clone(G);
    try{
      const r=await validate(graph,lastValidated);
      if(my!==seq)return; // a newer edit superseded this response
      report=r;lastKey=key;lastValidated=graph;
    }catch(e){if(my!==seq)return;report={valid:false,findings:[],error:e.message}}
    draw();
  }

  // ---- findings ---------------------------------------------------------
  const findingsFor=pred=>(report?.findings||[]).filter(f=>pred(f.subject?.ref||'',f.subject?.type));
  const nodeFindings=id=>findingsFor((r,t)=>r===id||r.startsWith(id+'.')||(t==='edge'&&(r.startsWith(id+'.')||r.includes('->'+id+'.'))));
  const edgeFindings=e=>findingsFor((r,t)=>t==='edge'&&r===`${e.from}->${e.to}`);
  const worst=fs=>fs.some(f=>f.severity==='error')?'error':fs.some(f=>f.severity==='warning')?'warning':null;

  // ---- render -----------------------------------------------------------
  function nodeHeight(n){const d=defOf(n);return HEAD_H+Math.max(portsOf(d,'in').length,portsOf(d,'out').length,1)*ROW_H+10}

  function draw(){
    $('#ge-undo').disabled=!undo.length;$('#ge-redo').disabled=!redo.length;$('#ge-del').disabled=!sel;
    const st=$('#ge-state');
    st.textContent=!report?'':report.error?'validation unavailable':report.valid?'✓ valid':`${report.findings.filter(f=>f.severity==='error').length} error(s)`;
    st.className='mono '+(report?.valid?'ok':report&&!report.error?'errc':'dim');
    const nodes=$('#ge-nodes');
    nodes.innerHTML=G.nodes.map(n=>{
      const d=defOf(n),fs=nodeFindings(n.instance_id),w=worst(fs);
      const ins=portsOf(d,'in'),outs=portsOf(d,'out');
      const rows=Math.max(ins.length,outs.length,1);
      const row=i=>{
        const a=ins[i],b=outs[i];
        const dot=(p,dir)=>p?`<span class="ge-port ${dir} ${pending&&pending.node===n.instance_id&&pending.port===p.port_id&&dir==='out'?'pend':''}" data-node="${esc(n.instance_id)}" data-port="${esc(p.port_id)}" data-dir="${dir}" title="${esc(p.name||p.port_id)} · ${esc(p.artifact_kind||'?')} · ${esc(p.unit??'no unit')}"></span>`:'';
        return `<div class="ge-row"><span class="ge-l">${dot(a,'in')}<i>${esc(a?.port_id||'')}</i></span><span class="ge-r"><i>${esc(b?.port_id||'')}</i>${dot(b,'out')}</span></div>`;
      };
      return `<div class="ge-node ${sel?.type==='node'&&sel.id===n.instance_id?'sel':''} ${w||''}" data-id="${esc(n.instance_id)}" style="left:${n.layout.x}px;top:${n.layout.y}px;width:${NODE_W}px;height:${nodeHeight(n)}px">
        <div class="ge-head" data-drag="${esc(n.instance_id)}"><b>${esc(d?.name||n.ref.id)}</b><span class="mono dim">${esc(n.instance_id)}</span>${fs.length?`<span class="pill ${w==='error'?'errc':'warnc'}">${fs.length}</span>`:''}${statuses[n.instance_id]&&statuses[n.instance_id]!=='idle'?`<span class="ge-st ${esc(statuses[n.instance_id])}" title="Preview status">${esc(statuses[n.instance_id])}</span>`:''}</div>
        ${d?Array.from({length:rows},(_,i)=>row(i)).join(''):`<div class="ge-row errc">definition not found</div>`}</div>`;
    }).join('');
    const w=Math.max(600,...G.nodes.map(n=>n.layout.x+NODE_W+40)),h=Math.max(360,...G.nodes.map(n=>n.layout.y+nodeHeight(n)+40));
    const canvas=$('#ge-canvas');$('#ge-nodes').style.cssText=`position:relative;width:${w}px;height:${h}px`;
    const svg=$('#ge-svg');svg.setAttribute('width',w);svg.setAttribute('height',h);
    drawEdges();drawSide();bindCanvas(canvas);
  }

  function dotCenter(node,port,dir){
    const el=container.querySelector(`.ge-port[data-node="${CSS.escape(node)}"][data-port="${CSS.escape(port)}"][data-dir="${dir}"]`);
    if(!el)return null;
    const r=el.getBoundingClientRect(),c=$('#ge-nodes').getBoundingClientRect();
    return {x:r.left-c.left+r.width/2,y:r.top-c.top+r.height/2};
  }
  function drawEdges(){
    const svg=$('#ge-svg');
    svg.innerHTML=G.edges.map((e,i)=>{
      const [fi,fp]=e.from.split('.'),[ti,tp]=e.to.split('.');
      const a=dotCenter(fi,fp,'out'),b=dotCenter(ti,tp,'in');
      if(!a||!b)return '';
      const fs=edgeFindings(e),w=worst(fs),dx=Math.max(40,Math.abs(b.x-a.x)/2);
      const d=`M${a.x},${a.y} C${a.x+dx},${a.y} ${b.x-dx},${b.y} ${b.x},${b.y}`;
      return `<path class="ge-hit" data-edge="${i}" d="${d}"/><path class="ge-edge ${w||''} ${sel?.type==='edge'&&sel.idx===i?'sel':''}" d="${d}"/>`;
    }).join('');
    svg.querySelectorAll('.ge-hit').forEach(p=>p.onclick=ev=>{ev.stopPropagation();sel={type:'edge',idx:+p.dataset.edge};draw()});
  }

  function drawSide(){
    const P=$('#ge-params'),F=$('#ge-finds');
    if(sel?.type==='node'){
      const n=G.nodes.find(x=>x.instance_id===sel.id),d=n&&defOf(n);
      P.innerHTML=n?`<div class="sh"><span>${esc(d?.name||n.ref.id)}</span><span class="mono">${esc(n.instance_id)}</span></div>
        ${d?(d.contract.parameters||[]).map(p=>paramField(n,p)).join('')||'<div class="empty">No parameters.</div>':'<div class="note errc">This node version is not in the knowledge base.</div>'}
        <div class="fld"><label>Version</label><span class="mono">${esc(n.ref.id)}@${esc(n.ref.version)}</span></div>
        <div class="fld"><span></span><button class="btn ghost" id="ge-inspect">Inspect node</button></div>`:'';
      const insp=P.querySelector('#ge-inspect');if(insp)insp.onclick=()=>opts.onInspect&&opts.onInspect(n.ref);
      P.querySelectorAll('[data-param]').forEach(el=>{
        const pid=el.dataset.param,type=el.dataset.type;
        el.onchange=()=>{
          let v;
          if(type==='boolean')v=el.checked;
          else if(type==='number'||type==='integer')v=el.value===''?undefined:Number(el.value);
          else v=el.value===''?undefined:el.value;
          commit(g=>{const t=g.nodes.find(x=>x.instance_id===sel.id);t.parameters=t.parameters||{};if(v===undefined)delete t.parameters[pid];else t.parameters[pid]=v;if(!Object.keys(t.parameters).length)delete t.parameters});
        };
      });
    }else P.innerHTML='<div class="note">Select a node to edit its parameters. Click an output port, then an input port, to connect them.</div>';
    const all=report?.findings||[];
    F.innerHTML=all.length?`<div class="sh"><span>Validation</span><span class="mono">${all.length}</span></div>`+all.map(f=>`<div class="kbfind ${esc(f.severity)}" data-ref="${esc(f.subject?.ref||'')}" data-type="${esc(f.subject?.type||'')}"><div><span class="pill ${f.severity==='error'?'errc':f.severity==='warning'?'warnc':''}">${esc(f.severity)}</span> <span class="mono">${esc(f.code)}</span> <span class="mono dim">${esc(f.subject?.ref||'')}</span></div><div>${esc(f.explanation)}</div><div class="dim">→ ${esc(f.action)}</div></div>`).join(''):(report&&!report.error?'<div class="note ok">No findings.</div>':'');
    F.querySelectorAll('.kbfind').forEach(el=>el.onclick=()=>{
      const ref=el.dataset.ref;const inst=ref.split(/[.>-]/)[0];
      if(G.nodes.some(n=>n.instance_id===inst)){sel={type:'node',id:inst};draw()}
    });
  }

  function paramField(n,p){
    const cur=n.parameters?.[p.parameter_id];const id='gp-'+p.parameter_id;
    const hint=p.default!==undefined&&p.default!==null?`default ${p.default}`:p.required?'required':'';
    let ctl;
    if(p.type==='enum')ctl=`<select id="${id}" data-param="${esc(p.parameter_id)}" data-type="enum"><option value="">${hint?'('+esc(hint)+')':'—'}</option>${(p.allowed||[]).map(o=>`<option ${cur===o?'selected':''}>${esc(o)}</option>`).join('')}</select>`;
    else if(p.type==='boolean')ctl=`<input type="checkbox" id="${id}" data-param="${esc(p.parameter_id)}" data-type="boolean" ${cur===true?'checked':''}>`;
    else if(p.type==='number'||p.type==='integer')ctl=`<input type="number" id="${id}" data-param="${esc(p.parameter_id)}" data-type="${p.type}" value="${cur??''}" placeholder="${esc(hint)}" ${p.range?.min!=null?`min="${p.range.min}"`:''} ${p.range?.max!=null?`max="${p.range.max}"`:''} step="${p.type==='integer'?1:'any'}">`;
    else ctl=`<input id="${id}" data-param="${esc(p.parameter_id)}" data-type="string" value="${esc(cur??'')}" placeholder="${esc(hint)}">`;
    return `<div class="fld gp"><label for="${id}" title="${esc(p.meaning||'')}">${esc(p.parameter_id)}${p.unit?` <span class="dim">(${esc(p.unit)})</span>`:''}</label>${ctl}</div>`;
  }

  // ---- interaction ------------------------------------------------------
  function bindCanvas(canvas){
    canvas.onclick=ev=>{if(ev.target===canvas||ev.target.id==='ge-nodes'){sel=null;pending=null;draw()}};
    container.querySelectorAll('.ge-node').forEach(el=>el.onclick=ev=>{
      if(ev.target.closest('.ge-port'))return;
      ev.stopPropagation();sel={type:'node',id:el.dataset.id};draw();
    });
    container.querySelectorAll('.ge-port').forEach(el=>el.onclick=ev=>{
      ev.stopPropagation();
      const node=el.dataset.node,port=el.dataset.port,dir=el.dataset.dir;
      if(dir==='out'){
        pending=pending&&pending.node===node&&pending.port===port?null:{node,port};draw();return;
      }
      if(!pending){return}
      const from=`${pending.node}.${pending.port}`,to=`${node}.${port}`;
      pending=null;
      // Connecting is always allowed: the validator explains anything wrong with it.
      if(G.edges.some(e=>e.from===from&&e.to===to)){draw();return}
      commit(g=>g.edges.push({from,to}));
    });
    container.querySelectorAll('[data-drag]').forEach(h=>h.onpointerdown=ev=>{
      if(ev.button!==0)return;
      const id=h.dataset.drag,n=G.nodes.find(x=>x.instance_id===id);
      sel={type:'node',id};
      drag={id,sx:ev.clientX,sy:ev.clientY,ox:n.layout.x,oy:n.layout.y,before:JSON.stringify(G),moved:false};
      h.setPointerCapture(ev.pointerId);
      h.onpointermove=e=>{
        if(!drag)return;
        const nx=Math.max(0,drag.ox+e.clientX-drag.sx),ny=Math.max(0,drag.oy+e.clientY-drag.sy);
        if(nx!==n.layout.x||ny!==n.layout.y){drag.moved=true;n.layout.x=nx;n.layout.y=ny;
          const el=container.querySelector(`.ge-node[data-id="${CSS.escape(id)}"]`);if(el){el.style.left=nx+'px';el.style.top=ny+'px'}drawEdges()}
      };
      h.onpointerup=()=>{
        h.onpointermove=h.onpointerup=null;
        if(drag?.moved){undo.push(drag.before);if(undo.length>UNDO_LIMIT)undo.shift();redo=[];
          // Presentation only: notify the owner, but never re-validate.
          onChange&&onChange(clone(G))}
        drag=null;draw();
      };
    });
  }

  function addNode(){
    const [id,version]=($('#ge-pick').value||'').split('@');
    if(!id)return;
    const base=shortName(id);let i=1;while(G.nodes.some(n=>n.instance_id===base+i))i++;
    const inst=base+i;
    const maxY=Math.max(0,...G.nodes.map(n=>n.layout.y+nodeHeight(n)));
    commit(g=>{g.nodes.push({instance_id:inst,ref:{id,version},layout:{x:30+(g.nodes.length%3)*230,y:g.nodes.length<3?30:maxY+20}})});
    sel={type:'node',id:inst};draw();
  }
  function deleteSel(){
    if(!sel)return;
    if(sel.type==='node'){const id=sel.id;commit(g=>{g.nodes=g.nodes.filter(n=>n.instance_id!==id);g.edges=g.edges.filter(e=>e.from.split('.')[0]!==id&&e.to.split('.')[0]!==id)})}
    else{const i=sel.idx;commit(g=>{g.edges.splice(i,1)})}
    sel=null;draw();
  }

  $('#ge-add').onclick=addNode;$('#ge-undo').onclick=doUndo;$('#ge-redo').onclick=doRedo;$('#ge-del').onclick=deleteSel;
  container.onkeydown=ev=>{
    const m=ev.metaKey||ev.ctrlKey,typing=/INPUT|SELECT|TEXTAREA/.test(ev.target.tagName);
    if(m&&ev.key.toLowerCase()==='z'&&!typing){ev.preventDefault();ev.shiftKey?doRedo():doUndo()}
    else if(m&&ev.key.toLowerCase()==='y'&&!typing){ev.preventDefault();doRedo()}
    else if((ev.key==='Delete'||ev.key==='Backspace')&&!typing){ev.preventDefault();deleteSel()}
    else if(ev.key==='Escape'){pending=null;draw()}
  };

  fillPicker();draw();scheduleValidate();
  return {
    getGraph:()=>clone(G),
    setGraph(g){G=normalize(clone(g));undo=[];redo=[];sel=null;pending=null;lastKey=null;lastValidated=null;fillPicker();draw();scheduleValidate()},
    setDomain(d){domain=d},
    /** The pipe's target data profile (undoable; affects prerequisite checks). */
    setProfile(p){commit(g=>{if(p)g.target_data_profile=p;else delete g.target_data_profile})},
    refreshDefs(){fillPicker();draw()},
    /** Preview status per node (idle/stale/queued/running/ready/failed/cancelled). */
    setStatuses(map){statuses=map||{};draw()},
    selected:()=>sel?.type==='node'?sel.id:null,
    lastNode:()=>G.nodes.length?G.nodes[G.nodes.length-1].instance_id:null,
    undo:doUndo,redo:doRedo,canUndo:()=>undo.length>0,canRedo:()=>redo.length>0,
    report:()=>report,
    validateNow:runValidate,
  };
}
