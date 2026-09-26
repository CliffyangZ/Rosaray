// In-memory workspace state: opened local collections (folders of images and
// CRR reports) and the selected item. Read-only: files are only read in the
// browser; nothing is computed, uploaded, written or persisted.
import sample from './crr_sample.json';

export const REPORT_SCHEMA = 'rosaray.crr.report/1';
export const STATUS_TEXT = { ok: '已找到頸部', neck_not_found: '找不到明顯頸部', invalid_mask: 'mask 無效' };
const IMAGE_EXT = /\.(png|jpe?g|bmp|tiff?|webp)$/i;

export const num = v => (typeof v === 'number' && Number.isFinite(v) ? v : null);
export const pt = p => (Array.isArray(p) && p.length === 2 && num(p[0]) !== null && num(p[1]) !== null ? [p[0], p[1]] : null);
export const fmt = (v, nd = 2) => (num(v) === null ? '—' : v.toFixed(nd));

const state = { collections: [], active: null, selected: -1, mode: 'grid', errors: '' };
const listeners = new Set();
const emit = () => listeners.forEach(fn => fn(state));
let nextId = 1;

export const subscribe = fn => { listeners.add(fn); fn(state); };
export const activeCollection = () => state.collections.find(c => c.id === state.active) || null;
export const currentItem = () => { const c = activeCollection(); return (c && c.items[state.selected]) || null; };
export const currentReport = () => { const it = currentItem(); return it ? it.report : null; };
export const itemLabel = it => (it ? it.name : '—');
export const totals = () => { const c = activeCollection(); return { n: c ? c.items.length : 0, i: c ? state.selected : -1 }; };

// Image bytes are exposed through object URLs, created on first use.
export function itemUrl(it) {
  if (!it || !it.file) return null;
  if (!it.url) it.url = URL.createObjectURL(it.file);
  return it.url;
}

function validate(r) {
  if (!r || r.report_schema !== REPORT_SCHEMA) throw new Error('不是 CRR 報告（report_schema 不符）。');
  const img = r.image || {};
  if (!(num(img.width) > 0 && num(img.height) > 0)) throw new Error('報告缺少影像尺寸。');
  if (typeof img.file_name !== 'string' || !img.file_name) throw new Error('報告缺少 image.file_name。');
  return r;
}

const natural = (a, b) => a.name.localeCompare(b.name, 'zh-Hant', { numeric: true });

export function select(idx) {
  const c = activeCollection();
  if (c && idx >= 0 && idx < c.items.length) { state.selected = idx; emit(); }
}
export function step(delta) { select(state.selected + delta); }
export function setActive(id) { if (state.collections.some(c => c.id === id)) { state.active = id; state.selected = -1; emit(); } }
export function setMode(mode) { state.mode = mode; emit(); }

export function closeCollection(id) {
  const c = state.collections.find(x => x.id === id);
  if (!c) return;
  for (const it of c.items) if (it.url) URL.revokeObjectURL(it.url);
  state.collections = state.collections.filter(x => x !== c);
  if (state.active === id) { state.active = state.collections.length ? state.collections[0].id : null; state.selected = -1; }
  emit();
}

function addCollection(name, items) {
  items.sort(natural);
  const c = { id: nextId++, name, items };
  state.collections.push(c);
  state.active = c.id;
  state.selected = items.findIndex(i => i.report) >= 0 ? items.findIndex(i => i.report) : (items.length ? 0 : -1);
}

export function loadSample() {
  const r = validate(sample);
  const existing = state.collections.find(c => c.sample);
  if (existing) { state.active = existing.id; state.selected = 0; emit(); return; }
  addCollection('範例（合成資料）', [{ name: r.image.file_name, path: r.image.file_name, file: null, url: null, size: null, report: r }]);
  state.collections[state.collections.length - 1].sample = true;
  state.errors = '';
  emit();
}

// Groups files by top-level folder (folder picker) or into one loose collection.
export async function loadFiles(files) {
  const groups = new Map();
  for (const f of files) {
    const rel = f.webkitRelativePath || '';
    const top = rel.includes('/') ? rel.split('/')[0] : '選取的檔案';
    if (!groups.has(top)) groups.set(top, []);
    groups.get(top).push(f);
  }
  const errors = [];
  for (const [name, list] of groups) {
    const items = new Map();
    for (const f of list) if (IMAGE_EXT.test(f.name) || f.type.startsWith('image/')) {
      items.set(f.name, { name: f.name, path: f.webkitRelativePath || f.name, file: f, url: null, size: f.size, report: null });
    }
    for (const f of list) {
      if (!/\.json$/i.test(f.name)) continue;
      let data;
      try { data = JSON.parse(await f.text()); } catch (err) { errors.push(`${f.name}：不是有效的 JSON`); continue; }
      for (const raw of Array.isArray(data) ? data : [data]) {
        try {
          const r = validate(raw);
          const it = items.get(r.image.file_name) || { name: r.image.file_name, path: r.image.file_name, file: null, url: null, size: null, report: null };
          it.report = r;
          items.set(it.name, it);
        } catch (err) { if (!/\.coco|annotations/i.test(f.name)) errors.push(`${f.name}：${err.message}`); }
      }
    }
    if (items.size) addCollection(name, [...items.values()]);
  }
  state.errors = errors.slice(0, 5).join('\n');
  emit();
}
