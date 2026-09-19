import {rng} from './util.js';

// Synthetic intraoral placeholders (grayscale) until real photos are imported.
// `view` follows the backend diagram: L / M / R frontal photos per patient.
export const samples=[
  {id:'s1',name:'P001_M.png',patient:'P001',view:'M',split:'train',seed:11,spacing:0.05},
  {id:'s2',name:'P001_L.png',patient:'P001',view:'L',split:'train',seed:12,spacing:0.05},
  {id:'s3',name:'P002_M.png',patient:'P002',view:'M',split:'validation',seed:23,spacing:0.05},
  {id:'s4',name:'P003_M.png',patient:'P003',view:'M',split:'test',seed:37,spacing:0.05}
];
const cache={};
export const getImg=s=>cache[s.id]||(cache[s.id]=s.data||phantom(s));

function phantom(s){
  const W=480,H=320,r=rng(s.seed),d=new Float32Array(W*H),gt=new Uint8Array(W*H);
  const shift=s.view==='L'?-40:s.view==='R'?40:0,cx=W/2+shift,cy=H*.52,n=6,tw=52+r()*8,th=78+r()*10,curve=70+r()*30;
  const teeth=[];
  for(let i=-n/2;i<n/2;i++){const x=cx+(i+.5)*(tw+4),y=cy+curve*((x-cx)/W)**2*4;teeth.push({x,y,w:tw*(1-Math.abs(i+.5)*.05),h:th*(1-Math.abs(i+.5)*.06)})}
  for(let y=0;y<H;y++)for(let x=0;x<W;x++){
    let v=18+8*Math.sin(x/37+y/29);                          // oral cavity
    const mx=(x-W/2)/(W*.46),my=(y-H*.5)/(H*.46);
    if(mx*mx+my*my>1)v=36+22*Math.random();                  // lips / cheeks
    for(const t of teeth){
      const u=(x-t.x)/(t.w/2),w=(y-t.y)/(t.h/2);
      if(w<-.9&&w>-1.9&&Math.abs(u)<1.15)v=Math.max(v,128+14*Math.sin(x/9));   // gingiva above crown
      if(Math.abs(u)**3+Math.abs(w)**3<1){gt[y*W+x]=1;v=196+30*(1-Math.abs(w))-12*Math.abs(u)}
    }
    d[y*W+x]=Math.max(0,Math.min(255,v*(1+(r()*2-1)*.07)));
  }
  return{w:W,h:H,d,gt};
}
