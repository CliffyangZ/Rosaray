// Evidence: draws one kind of quantitative evidence on the selected real image
// (overlay / segmentation / bounding box …). Each evidence type declares its layers
// and its Measurement Details; only CRR exists today. Draws report data only.
import { $, esc } from './util.js';
import * as store from './store.js';
import { STATUS_TEXT, fmt, itemUrl, num, pt } from './store.js';

const ui = { layers: { seg: true, bbox: true, axis: true, marks: true }, opacity: 0.3, vb: null, key: null };

function bboxOf(r) {
  const b = r.bbox_xyxy;
  if (Array.isArray(b) && b.length === 4 && b.every(v => num(v) !== null)) return b;
  const c = (r.contour || []).map(pt).filter(Boolean);
  if (!c.length) return null;
  const xs = c.map(p => p[0]), ys = c.map(p => p[1]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs) + 1, Math.max(...ys) + 1];
}

// ---- evidence type registry -------------------------------------------------
const CRR = {
  label: 'CRR 量測',
  layers: [
    { id: 'seg', label: 'Segmentation' },
    { id: 'bbox', label: 'Bounding box' },
    { id: 'axis', label: '長軸' },
    { id: 'marks', label: '冠 / 根 / 頸部' },
  ],
  draw(r, L, u) {
    const g = r.geometry || {};
    const contour = (r.contour || []).map(pt).filter(Boolean);
    const [tip, neck, apex] = [pt(g.crown_tip), pt(g.neck), pt(g.root_apex)];
    const nl = Array.isArray(g.neck_line) ? g.neck_line.map(pt) : [];
    const ext = Array.isArray(g.axis_extent) ? g.axis_extent.map(pt) : [];
    const line = (a, b, cls, w) => `<line class="${cls}" x1="${a[0]}" y1="${a[1]}" x2="${b[0]}" y2="${b[1]}" stroke-width="${w * u}" stroke-linecap="round"/>`;
    const text = (x, y, cls, t) => `<text class="lbl ${cls}" x="${x}" y="${y}" font-size="${12 * u}">${esc(t)}</text>`;
    let s = '';
    if (L.seg && contour.length > 2) s += `<polygon class="cont" style="fill-opacity:${ui.opacity}" points="${contour.map(p => p.join(',')).join(' ')}" stroke-width="${1.5 * u}"/>`;
    const b = bboxOf(r);
    if (L.bbox && b) s += `<rect class="bbox" x="${b[0]}" y="${b[1]}" width="${b[2] - b[0]}" height="${b[3] - b[1]}" stroke-width="${1.5 * u}"/>${text(b[0], b[1] - 5 * u, 'bbox', `tooth ${b[2] - b[0]}×${b[3] - b[1]}`)}`;
    if (L.axis && ext.length === 2 && ext[0] && ext[1]) s += line(ext[0], ext[1], 'axis', 1.2);
    if (L.marks && tip && neck && apex) {
      s += line(tip, neck, 'crown', 3) + line(neck, apex, 'root', 3);
      if (nl.length === 2 && nl[0] && nl[1]) s += line(nl[0], nl[1], 'neckline', 2.5);
      s += `<circle class="neckdot" cx="${neck[0]}" cy="${neck[1]}" r="${5 * u}" stroke-width="${1.5 * u}"/>`;
      for (const p of [tip, apex]) s += `<circle class="enddot" cx="${p[0]}" cy="${p[1]}" r="${3.5 * u}"/>`;
      s += text((tip[0] + neck[0]) / 2 + 10 * u, (tip[1] + neck[1]) / 2, 'crown', 'CROWN') + text((neck[0] + apex[0]) / 2 + 10 * u, (neck[1] + apex[1]) / 2, 'root', 'ROOT') + text(neck[0] + 12 * u, neck[1] - 8 * u, 'neck', 'NECK ≈ CEJ');
    }
    return s;
  },
  details(r, it) {
    const m = r.metrics || {};
    const ok = r.status === 'ok';
    const px = v => (num(v) === null ? '—' : `${fmt(v, 2)} px`);
    const spacing = num(m.pixel_spacing_mm) !== null;
    const prov = r.provenance || {};
    const row = (k, v, cls) => `<div class="detail-row"><span>${k}</span><span class="mono ${cls || ''}">${v}</span></div>`;
    return `
      <h3>冠根比量測</h3>
      <div class="state">狀態：${esc(STATUS_TEXT[r.status] || r.status)}</div>
      <hr>
      ${row('CROWN / ROOT', ok ? fmt(m.ratio, 2) : '—', 'big-v accent')}
      ${row('牙冠長度', px(m.crown_length_px))}
      ${row('牙根長度', px(m.root_length_px))}
      ${row('頸部寬度', px(m.neck_width_px), 'warn')}
      <hr>
      ${row('牙冠占比', ok && num(m.crown_fraction) !== null ? `${(m.crown_fraction * 100).toFixed(1)}%` : '—')}
      ${row('牙齒全長', px(m.tooth_length_px))}
      ${row('最大寬度', px(m.max_width_px))}
      ${row('頸部凹陷深度', num(m.neck_prominence) !== null ? `${(m.neck_prominence * 100).toFixed(1)}%` : '—')}
      ${row('長軸角度', num(m.axis_angle_deg) !== null ? `${fmt(m.axis_angle_deg, 1)}°` : '—')}
      ${row('Mask 面積', num(m.mask_area_px) !== null ? `${m.mask_area_px.toLocaleString()} px²` : '—')}
      ${spacing ? row('牙冠 / 牙根 (mm)', `${fmt(m.crown_length_mm, 2)} / ${fmt(m.root_length_mm, 2)}`) : ''}
      <hr>
      ${widthChart(r)}
      <hr>
      <div class="sub">來源與限制</div>
      <p class="dim small">影像 ${esc(it.name)} · ${r.image.width}×${r.image.height} · mask：${esc(prov.mask_source || 'unknown')}${num(prov.sam_score) !== null ? `（SAM ${prov.sam_score}）` : ''}<br>純 mask 幾何近似；${spacing ? `pixel spacing ${m.pixel_spacing_mm} mm。` : '未提供 pixel spacing，數值以像素表示，'}非臨床 CRR。${r.message ? `<br>${esc(r.message)}。` : ''}</p>`;
  },
};
const TYPES = { crr: CRR };

function widthChart(r) {
  const p = r.profile || {};
  const w = (p.width_smooth || []).map(Number);
  const raw = (p.width || []).map(Number);
  if (w.length < 2 || w.some(v => !Number.isFinite(v))) return '';
  const L = 6, R = 6, T = 22, B = 14, VW = 320, VH = 110;
  const n = w.length, maxW = Math.max(...w, ...raw.filter(Number.isFinite), 1);
  const x = i => L + (i / (n - 1)) * (VW - L - R);
  const y = v => T + (1 - v / maxW) * (VH - T - B);
  const poly = a => a.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(' ');
  const ni = num(p.neck_index), ci = num(p.crown_end_index);
  let s = `<div class="sub">沿長軸的寬度輪廓</div><svg class="chart-svg" viewBox="0 0 ${VW} ${VH}" role="img" aria-label="沿長軸的寬度輪廓">`;
  for (const f of [0, 0.5, 1]) s += `<line class="grid" x1="${L}" x2="${VW - R}" y1="${y(maxW * f)}" y2="${y(maxW * f)}"/>`;
  s += `<polyline class="raw" points="${poly(raw)}"/><polyline class="smooth" points="${poly(w)}"/>`;
  if (ni !== null) s += `<line class="neckv" x1="${x(ni)}" x2="${x(ni)}" y1="${T - 4}" y2="${VH - B}"/><text class="neck-tag" x="${x(ni) + 4}" y="14">NECK</text><circle class="neckdot" cx="${x(ni)}" cy="${y(w[ni])}" r="3.5"/>`;
  if (ci !== null) s += `<text class="tick" x="${L}" y="${VH - 2}">${ci === 0 ? 'CROWN' : 'ROOT'}</text><text class="tick" x="${VW - R}" y="${VH - 2}" text-anchor="end">${ci === 0 ? 'ROOT' : 'CROWN'}</text>`;
  return s + '</svg>';
}

// ---- rendering --------------------------------------------------------------
function stageSvg(r, it, type) {
  const { width: W, height: H } = r.image;
  const u = Math.max(W, H) / 400;
  const url = itemUrl(it);
  const key = `${it.path}|${W}x${H}`;
  if (ui.key !== key) { ui.key = key; ui.vb = { x: 0, y: 0, w: W, h: H }; }
  const v = ui.vb;
  return `<svg class="stage-svg" viewBox="${v.x} ${v.y} ${v.w} ${v.h}" role="img" aria-label="${esc(type.label)}疊圖"><rect width="${W}" height="${H}" class="bg"/>${url ? `<image href="${esc(url)}" width="${W}" height="${H}" preserveAspectRatio="none"/>` : ''}${type.draw(r, ui.layers, u)}</svg>`;
}

export function drawEvidence() {
  const pane = $('#v-evidence');
  const it = store.currentItem();
  const c = store.activeCollection();
  if (!it) { pane.innerHTML = '<div class="empty big">尚未選取影像。請先到「檔案」開啟資料夾並選取一張影像。</div>'; return; }
  const type = TYPES.crr;
  const r = it.report;
  const { n, i } = store.totals();
  const nav = `<div class="imgnav"><button type="button" class="btn sm" data-step="-1" ${i <= 0 ? 'disabled' : ''} aria-label="上一張">‹</button><span class="mono small">${esc(it.name)} <span class="dim">· ${i + 1} / ${n}</span></span><button type="button" class="btn sm" data-step="1" ${i >= n - 1 ? 'disabled' : ''} aria-label="下一張">›</button></div>`;
  if (!r) {
    const url = itemUrl(it);
    pane.innerHTML = `<div class="ev-grid"><section class="panel stage-panel">${nav}<div class="stage-body">${url ? `<img class="plain-img" src="${esc(url)}" alt="${esc(it.name)}">` : ''}</div></section>
      <aside class="panel details"><div class="cap">MEASUREMENT DETAILS</div><h3>${esc(type.label)}</h3><div class="state warn">此影像尚無 CRR 報告</div><p class="dim small">前端為唯讀，不執行量測。請用 <span class="mono">python -m crr measure --image ${esc(it.name)} --out ${esc(it.name.replace(/\.[^.]+$/, ''))}.crr.json</span> 產生報告，再與影像放在同一資料夾開啟。</p></aside></div>`;
    return;
  }
  pane.innerHTML = `<div class="ev-grid">
    <section class="panel stage-panel" aria-label="${esc(type.label)}疊圖">
      <div class="ev-toolbar">
        <select id="ev-type" aria-label="Evidence 類型"><option>${esc(type.label)}</option></select>
        <div class="layers" role="group" aria-label="圖層">${type.layers.map(l => `<button type="button" class="lyr${ui.layers[l.id] ? ' on' : ''}" data-layer="${l.id}" aria-pressed="${ui.layers[l.id]}">${l.label}</button>`).join('')}</div>
        <label class="opac small dim">不透明度<input id="ev-opacity" type="range" min="0" max="1" step="0.05" value="${ui.opacity}"></label>
        <span class="grow"></span>
        <button type="button" class="btn sm" data-kb aria-haspopup="dialog">Knowledge base <kbd class="kbd">⌘K</kbd></button>
        <div class="zoomctl"><button type="button" class="btn sm" data-zoom="out" aria-label="縮小">−</button><button type="button" class="btn sm" data-zoom="fit">Fit</button><button type="button" class="btn sm" data-zoom="in" aria-label="放大">＋</button></div>
      </div>
      <div class="stage-body" id="ev-stage">${stageSvg(r, it, type)}</div>
      ${nav}
    </section>
    <aside class="panel details" aria-label="Measurement Details"><div class="cap">MEASUREMENT DETAILS</div>${type.details(r, it)}</aside>
  </div>`;
}

// ---- interaction (zoom / pan / layers) ---------------------------------------
function zoomAt(svg, factor, cx, cy) {
  const v = ui.vb; if (!v) return;
  const r = store.currentReport(); if (!r) return;
  const { width: W, height: H } = r.image;
  const nw = Math.min(W * 1.5, Math.max(W / 20, v.w * factor));
  const k = nw / v.w;
  const fx = cx ?? v.x + v.w / 2, fy = cy ?? v.y + v.h / 2;
  ui.vb = { x: fx - (fx - v.x) * k, y: fy - (fy - v.y) * k, w: nw, h: v.h * k };
  svg.setAttribute('viewBox', `${ui.vb.x} ${ui.vb.y} ${ui.vb.w} ${ui.vb.h}`);
}

export function initEvidence(redraw) {
  const pane = $('#v-evidence');
  pane.addEventListener('click', ev => {
    const l = ev.target.closest('[data-layer]');
    if (l) { ui.layers[l.getAttribute('data-layer')] = !ui.layers[l.getAttribute('data-layer')]; redraw(); return; }
    if (ev.target.closest('[data-kb]')) { $('#kb-open').click(); return; }
    const s = ev.target.closest('[data-step]');
    if (s) { store.step(Number(s.getAttribute('data-step'))); return; }
    const z = ev.target.closest('[data-zoom]');
    if (z) {
      const svg = pane.querySelector('.stage-svg'); const r = store.currentReport();
      if (!svg || !r) return;
      const m = z.getAttribute('data-zoom');
      if (m === 'fit') { ui.vb = { x: 0, y: 0, w: r.image.width, h: r.image.height }; svg.setAttribute('viewBox', `0 0 ${r.image.width} ${r.image.height}`); }
      else zoomAt(svg, m === 'in' ? 0.7 : 1.4);
    }
  });
  pane.addEventListener('input', ev => {
    if (ev.target.id === 'ev-opacity') { ui.opacity = Number(ev.target.value); const c = pane.querySelector('.cont'); if (c) c.style.fillOpacity = ui.opacity; }
  });
  const toUser = (svg, x, y) => { const p = svg.createSVGPoint(); p.x = x; p.y = y; return p.matrixTransform(svg.getScreenCTM().inverse()); };
  pane.addEventListener('wheel', ev => {
    const svg = ev.target.closest('.stage-svg'); if (!svg) return;
    ev.preventDefault();
    const p = toUser(svg, ev.clientX, ev.clientY);
    zoomAt(svg, ev.deltaY < 0 ? 0.85 : 1.18, p.x, p.y);
  }, { passive: false });
  let drag = null;
  pane.addEventListener('pointerdown', ev => {
    const svg = ev.target.closest('.stage-svg'); if (!svg || !ui.vb) return;
    drag = { svg, x: ev.clientX, y: ev.clientY, vb: { ...ui.vb } };
    svg.setPointerCapture(ev.pointerId);
    svg.classList.add('grab');
  });
  pane.addEventListener('pointermove', ev => {
    if (!drag) return;
    const rect = drag.svg.getBoundingClientRect();
    const scale = Math.max(drag.vb.w / rect.width, drag.vb.h / rect.height);
    ui.vb = { ...drag.vb, x: drag.vb.x - (ev.clientX - drag.x) * scale, y: drag.vb.y - (ev.clientY - drag.y) * scale };
    drag.svg.setAttribute('viewBox', `${ui.vb.x} ${ui.vb.y} ${ui.vb.w} ${ui.vb.h}`);
  });
  const end = () => { if (drag) { drag.svg.classList.remove('grab'); drag = null; } };
  pane.addEventListener('pointerup', end);
  pane.addEventListener('pointercancel', end);
}
