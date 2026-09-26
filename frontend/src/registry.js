import {boxBlur,otsu,morph,components} from './algo.js';
import {request,subscribeEvents} from './service_client.js';

// ---- Node definitions from the knowledge base (feature 002, T050) ----------
// The catalog is the source of truth for what nodes exist. `DEFS` below is the
// prototype's legacy in-browser catalog and is kept only so the existing local
// `exec` path keeps working until Preview moves to the service (T069).
const kbDefs=new Map(); // `${id}@${version}` -> {id,version,name,summary,contract,maturity,...}
let kbDefsLoad=null;

/**
 * Loads every published AlgoNode's definition (ports, parameters) from the
 * service. Cached; pass `force` to refetch after the catalog changes.
 * Resolves to an array of definitions, newest version last per id.
 */
export function loadNodeDefs(force){
  if(kbDefsLoad&&!force)return kbDefsLoad;
  kbDefsLoad=(async()=>{
    const {entries}=await request('GET','/kb/entries?kind=algonode&status=published&limit=500');
    const details=await Promise.all(entries.map(e=>request('GET',`/kb/algonode/${e.id}/${e.version}`).then(d=>({e,d})).catch(()=>null)));
    kbDefs.clear();
    for(const x of details){
      if(!x||!x.d.contract)continue;
      const {e,d}=x;
      kbDefs.set(`${e.id}@${e.version}`,{id:e.id,version:e.version,name:e.name||e.id,summary:e.summary||'',domain:e.domain||null,maturity:e.maturity||null,availability:e.availability,contract:d.contract});
    }
    return [...kbDefs.values()].sort((a,b)=>a.name.localeCompare(b.name));
  })().catch(err=>{kbDefsLoad=null;throw err});
  return kbDefsLoad;
}
/** Cached definition for a node reference, or undefined if not loaded/unknown. */
export const nodeDef=(id,version)=>kbDefs.get(`${id}@${version}`);

export const DEFS={
  source:{label:'Image source',cat:'Data',in:null,out:'Image2D',params:[]},
  normalize:{label:'Normalize',cat:'Preprocess',in:'Image2D',out:'Image2D',params:[
    {k:'lo',l:'Low pct',t:'range',min:0,max:20,step:.5,v:1},{k:'hi',l:'High pct',t:'range',min:80,max:100,step:.5,v:99}]},
  gaussian:{label:'Gaussian blur',cat:'Preprocess',in:'Image2D',out:'Image2D',params:[
    {k:'sigma',l:'Sigma',t:'range',min:.5,max:6,step:.5,v:2}]},
  threshold:{label:'Threshold',cat:'Segment',in:'Image2D',out:'Mask2D',params:[
    {k:'mode',l:'Mode',t:'select',o:['otsu','manual'],v:'manual'},{k:'value',l:'Value',t:'range',min:0,max:255,step:1,v:170},{k:'invert',l:'Invert',t:'check',v:false}]},
  morphology:{label:'Morphology',cat:'Segment',in:'Mask2D',out:'Mask2D',params:[
    {k:'op',l:'Operation',t:'select',o:['open','close','erode','dilate'],v:'open'},{k:'radius',l:'Radius',t:'range',min:1,max:8,step:1,v:2}]},
  onnx:{label:'ONNX segmentation',cat:'AI',in:'Image2D',out:'Mask2D',params:[
    {k:'size',l:'Input size',t:'select',o:['256','384','512'],v:'256'},{k:'thr',l:'Threshold',t:'range',min:.1,max:.9,step:.05,v:.5}]},
  area:{label:'Area measurement',cat:'Analyze',in:'Mask2D',out:'Table',params:[
    {k:'spacing',l:'Pixel mm',t:'range',min:.01,max:1,step:.01,v:.05}]}
};
export const dflt=t=>Object.fromEntries(DEFS[t].params.map(p=>[p.k,p.v]));
export function exec(type,p,inp,ctx){
  if(type==='source')return{kind:'image',w:ctx.img.w,h:ctx.img.h,d:Float32Array.from(ctx.img.d)};
  if(type==='normalize'){const s=Float32Array.from(inp.d).sort(),lo=s[Math.floor(s.length*p.lo/100)],hi=s[Math.min(s.length-1,Math.floor(s.length*p.hi/100))],k=255/Math.max(1e-6,hi-lo);
    return{...inp,d:inp.d.map(v=>Math.max(0,Math.min(255,(v-lo)*k)))}}
  if(type==='gaussian'){const r=Math.max(1,Math.round(p.sigma*.9));return{...inp,d:boxBlur(boxBlur(inp.d,inp.w,inp.h,r),inp.w,inp.h,r)}}
  if(type==='threshold'){const th=p.mode==='otsu'?otsu(inp.d):p.value;ctx.notes.push('Threshold = '+th);
    return{kind:'mask',w:inp.w,h:inp.h,d:Uint8Array.from(inp.d,v=>(v>th)!==p.invert?1:0)}}
  if(type==='morphology')return{...inp,d:morph(inp.d,inp.w,inp.h,p.op,p.radius)};
  if(type==='onnx')throw new Error('ONNX segmentation: no model asset attached. Import an .onnx model first (File → Import Model).');
  if(type==='area'){let n=0;for(const v of inp.d)n+=v;return{kind:'table',w:inp.w,h:inp.h,rows:{px:n,mm2:n*p.spacing*p.spacing,cc:components(inp.d,inp.w,inp.h)}}}
}

// ---- AlgoPipe draft Preview (feature 002, US4) -----------------------------
// The service builds the graph from the draft at `revision`, runs the target's
// ancestor cone through the trusted built-in executors, and labels the outcome
// verified/unverified. Resolves to the raw contract body:
//   {state:'ready',artifact_ref,verification_state,unverified_nodes,node_reuse,...}
//   {state:'failed',failing_node_id,error,last_successful_artifact_ref,stale}
// A 409 not_previewable rejects with ServiceError (details.nodes / .findings).
export const previewPipe=(pipeId,revision,imageAssetId,targetNodeId,port)=>
  request('POST','/preview',{pipe_id:pipeId,revision,image_asset_id:imageAssetId,target_node_id:targetNodeId,...(port?{port}:{})});
export const previewStatus=(pipeId,revision)=>
  request('GET',`/preview/status?pipe_id=${encodeURIComponent(pipeId)}${revision?`&revision=${encodeURIComponent(revision)}`:''}`);

// ---- Preview request flow (User Story 3, contracts/local-service-api.md
// §Preview, event-bus.md). The Local Rosaray Service owns content-
// equivalence caching and execution; this layer only tracks which request
// is "current" per (image_asset_id, target_node_id) selection so a
// preview_ready/preview_failed event for a since-superseded selection is
// discarded rather than displayed as current (FR-017).

const latestRequestBySelection=new Map();
let unsubscribePreviewEvents=null;

const selectionKey=(imageAssetId,targetNodeId)=>`${imageAssetId}::${targetNodeId}`;

/**
 * Fires `POST /preview` for `targetNodeId` against `pipelineSnapshot`
 * (the `{nodes, edges}` graph shape from data-model.md PipelineSnapshot).
 * Resolves to `{state:'ready', artifactRef, reused, requestContextId}` or
 * `{state:'stale', error, failingNodeId, lastSuccessfulArtifactRef}` —
 * never throws for a pipeline-level failure, only for a transport/session
 * error (ServiceError, per service_client.js).
 */
export async function requestPreview(imageAssetId,targetNodeId,pipelineSnapshot){
  const key=selectionKey(imageAssetId,targetNodeId);
  const body=await request('POST','/preview',{
    image_asset_id:imageAssetId,
    target_node_id:targetNodeId,
    pipeline_snapshot:pipelineSnapshot,
  });

  if(body.request_context_id)latestRequestBySelection.set(key,body.request_context_id);

  if(body.state==='stale')return{
    state:'stale',
    error:body.error,
    failingNodeId:body.failing_node_id,
    lastSuccessfulArtifactRef:body.last_successful_artifact_ref??null,
  };
  return{
    state:'ready',
    artifactRef:body.artifact_ref,
    reused:body.reused,
    requestContextId:body.request_context_id,
  };
}

/**
 * Subscribes to `preview_ready`/`preview_failed` Event Bus notifications,
 * invoking `onEvent({type, imageAssetId, targetNodeId, ...payload})` only
 * when the event's `request_context_id` still matches the latest request
 * issued for that `(image_asset_id, target_node_id)` selection — any other
 * event is a late arrival for a superseded selection and is silently
 * dropped, never treated as an error (event-bus.md, FR-017). Returns an
 * unsubscribe function; only one subscription is active at a time.
 */
export function watchPreviewEvents(onEvent){
  unsubscribePreviewEvents?.();
  unsubscribePreviewEvents=subscribeEvents((event)=>{
    if(event.type!=='preview_ready'&&event.type!=='preview_failed')return;
    const{image_asset_id:imageAssetId,target_node_id:targetNodeId,request_context_id:requestContextId}=event.payload;
    const key=selectionKey(imageAssetId,targetNodeId);
    if(latestRequestBySelection.get(key)!==requestContextId)return;
    onEvent({type:event.type,imageAssetId,targetNodeId,requestContextId,payload:event.payload});
  });
  return unsubscribePreviewEvents;
}

