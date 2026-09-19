import {boxBlur,otsu,morph,components} from './algo.js';

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

