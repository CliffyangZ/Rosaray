// Explorer: browse opened local collections (folders of images + CRR reports) as a
// list or a thumbnail grid. Selecting an item makes it the working image; opening
// it shows its evidence. Read-only: no rename, delete-from-disk or upload.
import { $, esc } from './util.js';
import * as store from './store.js';

const ui = { q: '', filter: 'all', sort: 'name', size: 150 };

const ratioOf = it => (it.report && it.report.status === 'ok' ? it.report.metrics.ratio : null);
const badge = it => {
  if (!it.report) return '<span class="bd none">無報告</span>';
  if (it.report.status === 'ok') return `<span class="bd ok">ratio ${store.fmt(ratioOf(it), 2)}</span>`;
  return `<span class="bd warn">${esc(store.STATUS_TEXT[it.report.status] || it.report.status)}</span>`;
};
const kb = n => (n == null ? '—' : n > 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);

function visible(c) {
  const q = ui.q.trim().toLowerCase();
  const rows = c.items.map((it, idx) => ({ it, idx })).filter(({ it }) =>
    (!q || it.name.toLowerCase().includes(q)) &&
    (ui.filter === 'all' || (ui.filter === 'reported' ? it.report : !it.report)));
  if (ui.sort === 'ratio') rows.sort((a, b) => (ratioOf(b.it) ?? -1) - (ratioOf(a.it) ?? -1));
  return rows;
}

export function drawExplorer(st) {
  const c = store.activeCollection();
  $('#ex-tree').innerHTML = st.collections.length
    ? st.collections.map(x => `<div class="tree-row${x.id === st.active ? ' on' : ''}" data-cid="${x.id}"><button type="button" class="tree-btn" data-cid="${x.id}"><span class="fold">▸</span><span class="nm">${esc(x.name)}</span><span class="dim mono small">${x.items.length}</span></button><button type="button" class="tree-x" data-close="${x.id}" aria-label="關閉 ${esc(x.name)}">×</button></div>`).join('')
    : '<div class="empty small">尚未開啟資料集。</div>';
  for (const b of document.querySelectorAll('#ex-mode [data-mode]')) b.classList.toggle('on', b.getAttribute('data-mode') === st.mode);
  $('#ex-size-wrap').hidden = st.mode !== 'grid';
  const body = $('#ex-items');
  const foot = $('#ex-foot');
  if (!c) { body.innerHTML = '<div class="empty big">開啟含影像與 CRR 報告（*.crr.json）的資料夾，或載入範例。</div>'; foot.textContent = ''; return; }
  const rows = visible(c);
  const reported = c.items.filter(i => i.report).length;
  foot.textContent = `${c.name} · 顯示 ${rows.length} / ${c.items.length} 項 · ${reported} 項有 CRR 報告${st.selected >= 0 ? ` · 已選：${c.items[st.selected].name}` : ''}`;
  if (!rows.length) { body.innerHTML = '<div class="empty big">沒有符合條件的項目。</div>'; return; }
  if (st.mode === 'grid') {
    body.className = 'ex-items grid';
    body.style.setProperty('--tile', `${ui.size}px`);
    body.innerHTML = rows.map(({ it, idx }) => {
      const url = store.itemUrl(it);
      return `<button type="button" class="tile${idx === st.selected ? ' on' : ''}" data-i="${idx}" title="${esc(it.path)}"><span class="thumb">${url ? `<img src="${esc(url)}" alt="" loading="lazy" decoding="async">` : '<span class="noimg dim small">無影像檔</span>'}</span><span class="cap-row"><span class="nm">${esc(it.name)}</span>${badge(it)}</span></button>`;
    }).join('');
  } else {
    body.className = 'ex-items list';
    body.style.removeProperty('--tile');
    body.innerHTML = `<table class="ex-table"><thead><tr><th></th><th>名稱</th><th>CRR 狀態</th><th>ratio</th><th>牙冠 / 牙根 px</th><th>大小</th></tr></thead><tbody>${rows.map(({ it, idx }) => {
      const url = store.itemUrl(it);
      const m = (it.report && it.report.metrics) || {};
      return `<tr class="${idx === st.selected ? 'on' : ''}" data-i="${idx}" tabindex="0"><td class="th">${url ? `<img src="${esc(url)}" alt="" loading="lazy" decoding="async">` : ''}</td><td class="nm">${esc(it.name)}</td><td>${badge(it)}</td><td class="mono">${store.fmt(ratioOf(it), 2)}</td><td class="mono">${it.report && it.report.status === 'ok' ? `${store.fmt(m.crown_length_px, 1)} / ${store.fmt(m.root_length_px, 1)}` : '—'}</td><td class="mono dim">${kb(it.size)}</td></tr>`;
    }).join('')}</tbody></table>`;
  }
}

export function initExplorer(openEvidence, redraw) {
  const pick = ev => { const idx = ev.target.closest('[data-i]'); return idx ? Number(idx.getAttribute('data-i')) : null; };
  const body = $('#ex-items');
  body.addEventListener('click', ev => { const i = pick(ev); if (i !== null) store.select(i); });
  body.addEventListener('dblclick', ev => { const i = pick(ev); if (i !== null) { store.select(i); openEvidence(); } });
  body.addEventListener('keydown', ev => { if (ev.key === 'Enter') { const i = pick(ev); if (i !== null) { store.select(i); openEvidence(); } } });
  $('#ex-tree').addEventListener('click', ev => {
    const x = ev.target.closest('[data-close]');
    if (x) { store.closeCollection(Number(x.getAttribute('data-close'))); return; }
    const b = ev.target.closest('[data-cid]');
    if (b) store.setActive(Number(b.getAttribute('data-cid')));
  });
  for (const b of document.querySelectorAll('#ex-mode [data-mode]')) b.addEventListener('click', () => store.setMode(b.getAttribute('data-mode')));
  $('#ex-q').addEventListener('input', ev => { ui.q = ev.target.value; redraw(); });
  $('#ex-filter').addEventListener('change', ev => { ui.filter = ev.target.value; redraw(); });
  $('#ex-sort').addEventListener('change', ev => { ui.sort = ev.target.value; redraw(); });
  $('#ex-size').addEventListener('input', ev => { ui.size = Number(ev.target.value); $('#ex-items').style.setProperty('--tile', `${ui.size}px`); });
  $('#ex-open-ev').addEventListener('click', openEvidence);
  $('#ex-dir').addEventListener('change', ev => { store.loadFiles([...ev.target.files]); ev.target.value = ''; });
  $('#ex-files').addEventListener('change', ev => { store.loadFiles([...ev.target.files]); ev.target.value = ''; });
  $('#ex-sample').addEventListener('click', store.loadSample);
  const area = $('#v-explorer');
  area.addEventListener('dragover', ev => ev.preventDefault());
  area.addEventListener('drop', ev => { ev.preventDefault(); store.loadFiles([...ev.dataTransfer.files]); });
}
