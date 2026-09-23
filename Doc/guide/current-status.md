# 目前狀態

目前版本已從「純前端原型」演進為「瀏覽器前端 + Rust 本機服務」的雙層架構：

- Rust 本機服務（`backend/`）承載 Core、Data、Algorithm 與 Execution 層，僅繫結 `127.0.0.1`，透過本機 HTTP + WebSocket API 提供資料。
- 瀏覽器前端（`frontend/`）改由 `service_client.js` 連線本機服務，不再直接讀取本機檔案路徑。
- 所有 Rosaray 管理的持久化資料預設加密（Argon2id 主鑰 + AES-256-GCM envelope encryption），並以內容定址（BLAKE3）儲存至加密 blob store。
- **Import Model**、**Save Project** 與 Evidence 搜尋目前仍是介面預留功能。
- ONNX segmentation 節點需要模型資產與後端／載入流程，現階段不會實際執行。
- 內建影像與 reference mask 是合成資料，不代表真實臨床資料表現。

## Data Layer 實作進度

Data Layer 規格（[Spec](/features/data-layer/spec)）分為五個使用者故事，目前已完成前四項（對應任務 T001–T058）：

| 優先序 | 使用者故事 | 狀態 |
|---|---|---|
| P1 | US1 — 建立可信任的研究資料集：匯入、去識別化、leakage/mask 驗證、Dataset Version fingerprint | ✅ 完成 |
| P2 | US2 — 在前端瀏覽研究資料：Explorer、Image Display Descriptor、縮圖、連線狀態 | ✅ 完成 |
| P3 | US3 — 低延遲檢視 Pipeline 中繼結果：Preview 快取、內容等價鍵、stale 回應保護 | ✅ 完成 |
| P4 | US4 — 保存可重現的正式實驗：Run 持久化、可追溯性鏈、可重現性檢查 | ✅ 完成 |
| P5 | US5 — 攜出與恢復研究專案：Export / Import Bundle（加密、完整性驗證） | ⏳ 尚未開始 |
| — | 收尾：Preview/Thumbnail cache 清除、刪除端點、錯誤分類一致性檢查、效能驗證 | ⏳ 尚未開始 |

對應的實作檔案與任務清單見 [實作計畫](/features/data-layer/plan) 與 `specs/001-data-layer-design/tasks.md`。

## 進行中的設計

目前正在進行 [Data Layer 規格](/features/data-layer/spec) 的實作，以 Rust 本機服務承載 Core、Algorithm、Execution 與 Data 層，詳見 [實作計畫](/features/data-layer/plan) 與 [研究決策](/features/data-layer/research)。

## License

License 尚未設定。
