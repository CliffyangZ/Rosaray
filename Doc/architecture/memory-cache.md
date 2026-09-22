# Memory Cache

> 所屬層級：[Data Layer](./data-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

提供低延遲的記憶體內快取，存放近期存取的影像／結果與單張影像 Preview 的中繼 artifact，減少重複讀取 [Persistent Storage](./persistent-storage.md) 或重複執行未變更節點的成本。

## 資料流

- 輸入：
  - [Data Repository](./data-repository.md) → 待快取資料
- 輸出：
  - → [Data Repository](./data-repository.md)：快取命中資料

## 需求

### 功能需求

- 提供 key-value 快取介面（以影像 ID／run ID 為 key）。
- Preview artifact 使用內容式 key：`node_type`、`implementation_version`、canonical parameters、Input artifact hashes 與 seed；UI graph revision 不屬於 key。
- 保存 artifact metadata：輸出型別／shape、建立時間、大小、產生節點、Input hashes 與 deterministic／cacheable 標記。
- 支援依 artifact reference 讀取，讓 [Event Bus](./event-bus.md) 只需傳 reference，不傳大型影像資料。
- 支援快取淘汰策略（例如 LRU），避免大影像資料無限制佔用記憶體。

### 非功能需求

- 讀取延遲需達到互動式渲染等級（毫秒級），支撐 [Presentation Engine](./presentation-engine.md) 的縮放／平移即時操作。
- 快取為易失性，不可作為持久資料的唯一來源；行程重啟後，正式資料由 [Persistent Storage](./persistent-storage.md) 重建，Preview artifact 則按需重新運算。
- Preview artifact 被淘汰後屬正常 cache miss，由 [Data Repository](./data-repository.md) 回報 [Execution Engine](./execution-engine.md) 重新執行，不視為資料遺失。

## 相依模組

- 上游／下游：[Data Repository](./data-repository.md)（雙向：寫入快取、讀出快取）

## 待確認

- 快取容量上限與淘汰策略的具體參數尚未定義。
