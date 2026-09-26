// Read-only agent panel. There is no chat backend: the summary below is generated
// by fixed rules from the loaded report, and the composer is disabled.
import { $, esc } from './util.js';
import { STATUS_TEXT, currentItem, currentReport as current, fmt, itemLabel, num } from './store.js';

const VIEW_NAME = { inference: '病理推斷', evidence: 'CRR 量測' };

function summary(r) {
  if (!r) return '尚未載入報告。載入後，這裡會列出量測摘要與建議核對項目。';
  const m = r.metrics || {};
  if (r.status === 'ok') return `目前 CRR 為 ${fmt(m.ratio, 2)}。此值來自 mask 幾何量測${num(m.neck_prominence) !== null ? `（頸部凹陷深度 ${(m.neck_prominence * 100).toFixed(1)}%）` : ''}，應先核對頸部位置與影像品質。`;
  return `${STATUS_TEXT[r.status] || r.status}：${r.message || '未取得有效 CRR'}。不提供 ratio，請人工檢視 mask。`;
}

export function drawAgent(view) {
  const r = current();
  $('#agent-body').innerHTML = `
    <div class="cap">CASE CONTEXT &nbsp;/&nbsp; ${esc(itemLabel(currentItem()))}</div>
    <div class="card"><b>正在檢視：${VIEW_NAME[view] || ''}</b><div class="dim small">可引用本頁可見的研究證據；回覆須可追溯。</div></div>
    <div class="card msg"><span class="tg">◈ 規則摘要 · 由報告數值產生</span><div>${esc(summary(r))}</div></div>
    <div class="card msg"><span class="tg">◈ 規則摘要</span><b>優先檢查三項：</b>
      <div class="list">1&nbsp; 頸部與長軸標記<br>2&nbsp; 原始影像品質<br>3&nbsp; 替代解釋與適用條件</div>
      <span class="yellow small">引用：Evidence / CRR · 影像觀察</span></div>`;
}
