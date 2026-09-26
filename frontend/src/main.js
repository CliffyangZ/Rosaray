import './styles.css';
// Rosaray research workspace shell (read-only): case explorer, 病理推斷 / Evidence /
// Knowledge base and an agent summary panel. No view can author, run or persist anything.
import { $, esc } from './util.js';
import * as store from './store.js';
import { drawEvidence, initEvidence } from './evidence_view.js';
import { drawExplorer, initExplorer } from './explorer_view.js';
import { drawInference } from './inference_view.js';
import { drawAgent } from './agent_panel.js';
import { initKb } from './kb_view.js';

const app = $('#app');
const VIEWS = {
  inference: { crumb: 'INFERENCE', kicker: 'INFERENCE', title: '病理推斷視圖', chip: '待人工審閱', sub: '從量化證據追蹤推斷路徑；所有結論均待人工審閱。' },
  evidence: { crumb: 'EVIDENCE', kicker: 'EVIDENCE', title: 'Evidence / 量化證據', chip: 'CRR · 已載入', sub: '目前顯示 CRR 量測；此區可擴充更多量化資訊來源。' },
  explorer: { crumb: 'EXPLORER', kicker: 'EXPLORER', title: '檔案', chip: '', sub: '' },
};
let view = 'explorer';
let stateNow;

function drawShell(st) {
  const msg = $('#ex-msg');
  msg.hidden = !st.errors;
  msg.textContent = st.errors;
  drawExplorer(st);
}

function render() {
  const it = store.currentItem();
  const r = store.currentReport();
  const label = it ? it.name : '—';
  const v = VIEWS[view];
  app.setAttribute('data-view', view);
  for (const b of document.querySelectorAll('[data-view].rail-btn,[data-view].nav')) {
    b.classList.toggle('on', b.getAttribute('data-view') === view && (!b.getAttribute('data-sub') || view === 'evidence'));
  }
  const { n, i } = store.totals();
  $('#crumb-case').textContent = label;
  $('#crumb-view').textContent = v.crumb;
  $('#chip-case').textContent = it ? `影像 ${label}` : '尚未選取影像';
  $('#page-kicker').textContent = `CASE ${label} / ${v.kicker}`;
  $('#page-title').textContent = v.title;
  $('#page-chip').textContent = v.chip;
  $('#page-sub').textContent = v.sub;
  $('#status-case').textContent = `${label}  ·  ${n ? i + 1 : 0} / ${n} IMAGE`;
  if (view === 'inference') drawInference(() => go('evidence'));
  if (view === 'evidence') drawEvidence();
  drawAgent(view);
}

function go(next) { view = next; render(); }
function setAgent(open) {
  app.setAttribute('data-agent', open ? 'open' : 'closed');
  $('#agent-toggle').setAttribute('aria-pressed', String(open));
}

for (const b of document.querySelectorAll('[data-view]')) b.addEventListener('click', () => go(b.getAttribute('data-view')));
$('#agent-toggle').addEventListener('click', () => setAgent(app.getAttribute('data-agent') !== 'open'));
$('#agent-close').addEventListener('click', () => setAgent(false));
initExplorer(() => go('evidence'), () => drawShell(stateNow));
initEvidence(render);
initKb();
const kb = $('#kb-dialog');
const openKb = () => { if (kb.open) return; kb.showModal(); $('#kb-q').focus(); };
const closeKb = () => kb.close();
$('#kb-open').addEventListener('click', openKb);
$('#kb-close').addEventListener('click', closeKb);
kb.addEventListener('click', ev => { if (ev.target === kb) closeKb(); }); // backdrop click
document.addEventListener('keydown', ev => {
  if ((ev.metaKey || ev.ctrlKey) && !ev.altKey && !ev.shiftKey && ev.key.toLowerCase() === 'k') {
    ev.preventDefault();
    kb.open ? closeKb() : openKb();
  }
});
store.subscribe(st => { stateNow = st; drawShell(st); render(); });
setAgent(window.innerWidth >= 1500);
