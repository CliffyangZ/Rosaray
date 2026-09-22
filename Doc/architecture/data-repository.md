# Data Repository

> 所屬層級：[Data Layer](./data-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

作為資料存取的統一入口，對上提供讀寫介面，對下分流到 [Memory Cache](./memory-cache.md)（熱資料）與 [Persistent Storage](./persistent-storage.md)（持久化）。

## 資料流

- 輸入：
  - [Data Engine](./data-engine.md) → 寫入影像／結果
  - [Presentation Engine](./presentation-engine.md) → `Read Image / Result`（讀取請求）
  - [Execution Engine](./execution-engine.md) → 輸入 Image／artifact 讀取請求，以及 Preview／正式 Run artifact 寫入
- 輸出：
  - → [Memory Cache](./memory-cache.md)：快取熱資料
  - → [Persistent Storage](./persistent-storage.md)：持久化資料
  - → [Presentation Engine](./presentation-engine.md)：查詢結果
  - → [Execution Engine](./execution-engine.md)：輸入 Image、cache lookup 與 artifact 查詢結果

## 需求

### 功能需求

- 提供以 patient／split／run ID 為索引的查詢介面。
- 提供以 artifact reference／content cache key 讀寫中繼結果的介面，讓 [Execution Engine](./execution-engine.md) 不直接耦合儲存實作。
- 決定資料要進 [Memory Cache](./memory-cache.md) 或直接讀寫 [Persistent Storage](./persistent-storage.md)（例如近期存取的影像優先快取）。
- Preview artifact 為易失性資料，只寫入 [Memory Cache](./memory-cache.md)；正式 Run 結果依 run policy 同時建立可持久化記錄。
- 維護 patient 是否跨 split、reference mask 是否配對等一致性檢查（對應 README「Validation 面板」）。

### 非功能需求

- 讀取延遲需滿足 [Presentation Engine](./presentation-engine.md) 互動式渲染需求（不可阻塞 UI）。
- 需保證同一 run 的輸入影像、pipeline graph hash、結果三者可追溯對應。

## 相依模組

- 上游：[Data Engine](./data-engine.md)、[Presentation Engine](./presentation-engine.md)、[Execution Engine](./execution-engine.md)
- 下游：[Memory Cache](./memory-cache.md)、[Persistent Storage](./persistent-storage.md)、[Presentation Engine](./presentation-engine.md)、[Execution Engine](./execution-engine.md)

## 待確認

- System Overview canvas 尚未補上 [Execution Engine](./execution-engine.md) 的 artifact I/O 邊線。
