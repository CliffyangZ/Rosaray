export function boxBlur(d,w,h,r){
  if(r<1)return d;const t=new Float32Array(d.length),o=new Float32Array(d.length);
  for(let y=0;y<h;y++){let s=0;for(let x=-r;x<=r;x++)s+=d[y*w+Math.min(w-1,Math.max(0,x))];
    for(let x=0;x<w;x++){t[y*w+x]=s/(2*r+1);s+=d[y*w+Math.min(w-1,x+r+1)]-d[y*w+Math.max(0,x-r)]}}
  for(let x=0;x<w;x++){let s=0;for(let y=-r;y<=r;y++)s+=t[Math.min(h-1,Math.max(0,y))*w+x];
    for(let y=0;y<h;y++){o[y*w+x]=s/(2*r+1);s+=t[Math.min(h-1,y+r+1)*w+x]-t[Math.max(0,y-r)*w+x]}}
  return o;
}
export function otsu(d){const hist=new Float64Array(256);for(const v of d)hist[Math.min(255,Math.max(0,v|0))]++;
  const tot=d.length;let sum=0;for(let i=0;i<256;i++)sum+=i*hist[i];let sb=0,wb=0,best=0,th=0;
  for(let i=0;i<256;i++){wb+=hist[i];if(!wb)continue;const wf=tot-wb;if(!wf)break;sb+=i*hist[i];
    const mb=sb/wb,mf=(sum-sb)/wf,v=wb*wf*(mb-mf)**2;if(v>best){best=v;th=i}}return th}
export function morph(m,w,h,op,r){
  const pass=(src,fn)=>{const t=new Uint8Array(src.length),o=new Uint8Array(src.length);
    for(let y=0;y<h;y++)for(let x=0;x<w;x++){let a=fn===0?1:0;for(let k=-r;k<=r;k++){const xx=Math.min(w-1,Math.max(0,x+k)),v=src[y*w+xx];a=fn===0?(a&v):(a|v)}t[y*w+x]=a}
    for(let y=0;y<h;y++)for(let x=0;x<w;x++){let a=fn===0?1:0;for(let k=-r;k<=r;k++){const yy=Math.min(h-1,Math.max(0,y+k)),v=t[yy*w+x];a=fn===0?(a&v):(a|v)}o[y*w+x]=a}return o};
  const er=s=>pass(s,0),di=s=>pass(s,1);
  return op==='erode'?er(m):op==='dilate'?di(m):op==='open'?di(er(m)):er(di(m));
}
export function components(m,w,h){const seen=new Uint8Array(m.length);let n=0,stack=[];
  for(let i=0;i<m.length;i++){if(!m[i]||seen[i])continue;n++;stack.push(i);seen[i]=1;
    while(stack.length){const p=stack.pop(),x=p%w,y=(p/w)|0;
      for(const q of [x>0&&p-1,x<w-1&&p+1,y>0&&p-w,y<h-1&&p+w]){if(q!==false&&m[q]&&!seen[q]){seen[q]=1;stack.push(q)}}}}return n}
export function dice(a,b){let i=0,s=0;for(let k=0;k<a.length;k++){i+=a[k]&b[k];s+=a[k]+b[k]}return s?2*i/s:1}

