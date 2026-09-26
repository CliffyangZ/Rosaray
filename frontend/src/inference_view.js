// 病理推斷 tab: traces the loaded CRR evidence into a reasoning map.
// Only the evidence nodes and trace use report values; hypotheses, checklist and
// gate text are fixed illustrative content (labelled 示意), never computed conclusions.
import { $, esc } from './util.js';
import { currentReport as current, fmt } from './store.js';

export function drawInference(goEvidence) {
  const pane = $('#v-inference');
  const r = current();
  if (!r) { pane.innerHTML = '<div class="empty big">尚未載入 CRR 報告。請先到「檔案」開啟資料夾，並選取有 CRR 報告的影像。</div>'; return; }
  const m = r.metrics || {};
  const ok = r.status === 'ok';
  const crr = ok ? `冠根比 ${fmt(m.ratio, 2)}` : '冠根比 —';
  const crrSub = ok ? `牙冠 ${fmt(m.crown_length_px, 2)} px · 牙根 ${fmt(m.root_length_px, 2)} px` : esc(r.message || '尚無有效量測');
  pane.innerHTML = `
    <div class="inference">
      <section class="panel graph" aria-label="推斷圖">
        <div class="cap">REASONING MAP &nbsp;/&nbsp; EVIDENCE → HYPOTHESIS</div>
        <div class="dim small">可追蹤節點 · 示意資料</div>
        <div class="graph-body">
          <div class="col">
            <button type="button" class="gnode blue" data-go="evidence"><span class="tg">EVIDENCE / CRR</span><b>${crr}</b><span class="dim small">${crrSub}</span></button>
            <div class="gnode static lite"><span class="tg">IMAGE / OBSERVATION</span><b>影像特徵</b><span class="dim small">輪廓與長軸標記待確認</span></div>
          </div>
          <div class="links"><i class="ln blue"></i><i class="ln lite"></i></div>
          <div class="gnode static yellow wide"><span class="tg">AGENT / SYNTHESIS</span><b>交叉檢查</b><span class="dim small">量測、影像與適用條件逐項核對。產生可回溯的候選解釋。</span></div>
          <div class="out"><i class="ln yellow"></i><span class="arrow">→</span><span class="dim small">待審閱</span></div>
        </div>
        <div class="gate"><span class="gate-ic">◎</span><div><b>人工審閱關卡</b><div class="dim small">AI 提供推斷路徑與證據來源；正式判讀須由專業人員完成。</div></div></div>
      </section>
      <aside class="panel hyp" aria-label="候選解釋">
        <div class="cap">HYPOTHESES</div>
        <h3>候選解釋 <span class="tag-demo">示意</span></h3>
        <div class="cand"><span class="tg">01 / 待核對</span><b>牙周支持組織變化</b><span class="dim small">依據：CRR 與影像觀察</span><span class="yellow small">需補充：臨床與影像審閱</span></div>
        <div class="cand alt"><span class="tg">02 / 替代解釋</span><b>成像角度或遮蔽效應</b><span class="dim small">量測結果可能受影像品質影響</span></div>
        <hr>
        <div class="sub mono">REVIEW CHECKLIST</div>
        <ul class="check"><li>○ 確認輪廓與頸部位置</li><li>○ 核對原始影像品質</li><li>○ 紀錄人工結論</li></ul>
      </aside>
    </div>
    <section class="panel trace">
      <div class="yellow small">推斷追蹤 / TRACE</div>
      <div class="steps">
        <div><b>01&nbsp; 量測</b><span class="mono dim">${ok ? `CRR ${fmt(m.ratio, 2)}` : '無有效量測'}</span></div>
        <div><b>02&nbsp; 觀察</b><span class="dim">影像標記</span></div>
        <div><b>03&nbsp; 推斷</b><span class="dim">候選解釋</span></div>
        <div><b>04&nbsp; 審閱</b><span class="dim">待人工確認</span></div>
      </div>
    </section>
    <div class="notice">研究用途 · 這是方法與證據的可視化示意，不構成診斷或臨床有效性證明。</div>`;
  pane.querySelector('[data-go="evidence"]').addEventListener('click', goEvidence);
}
