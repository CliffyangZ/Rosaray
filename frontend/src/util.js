export const $ = (s, e = document) => e.querySelector(s);
export const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
export const pretty = v => String(v ?? '').replace(/_/g, ' ');
export const shortHash = h => (h ? String(h).replace(/^b3:/, '').slice(0, 12) : '—');
