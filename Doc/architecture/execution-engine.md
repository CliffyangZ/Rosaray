# Execution Engine

> 所屬層級：[Execution Layer](./execution-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

依 [Pipeline Definition](./pipeline-definition.md) 的節點與連線順序執行正式 Run，或依 [Algorithm Designer](./algorithm-designer.md) 的 Preview context 執行截至 target node 的必要子圖，並將進度／結果回報給 [Event Bus](./event-bus.md)。

## 資料流

- 輸入：
  - [Pipeline Definition](./pipeline-definition.md) → 可執行的 pipeline 定義
  - [Event Bus](./event-bus.md) → `PreviewRequested`（request ID、graph revision、Image、target node）
  - [Data Repository](./data-repository.md) → 輸入 Image、cache lookup 與 artifact 讀取結果
- 輸出：
  - → [Runtime Adapter](./runtime-adapter.md)：單一節點的運算請求
  - → [Data Repository](./data-repository.md)：Preview 中繼 artifact 或正式 Run 結果
  - → [Event Bus](./event-bus.md)：正式 Run 的 `Progress / Result`，以及 Preview lifecycle events

## 需求

### 功能需求

- 每次執行前重新驗證 typed DAG，拒絕 cycle、型別不相容、必要 Input 缺失與不合法 target node。
- 正式 Run 依拓樸排序執行完整 graph；Preview 只執行 Image source 到 target node 的 ancestor subgraph，不執行無關分支。
- 每個節點完成後產生中繼 artifact；Preview artifact 經 [Data Repository](./data-repository.md) 路由到 [Memory Cache](./memory-cache.md)，事件只發布 artifact reference。
- Preview cache key 由節點實作版本、canonical parameters、Input artifact hashes 與 seed 組成；cache hit 時跳過該節點運算。
- 同一 Image／pipeline context 的 Preview 採 latest-wins：接受新 revision 後取消舊請求，且不得發布舊 revision 為目前結果。
- 記錄每步執行時間、seed、輸入輸出雜湊，供 run history 使用（對應 README「Run history 保存...pipeline graph hash、seed 與執行時間」）。
- 正式 Run 失敗時中止並回報錯誤；Preview 失敗時發布失敗節點與錯誤，但不刪除上一個成功 artifact。

### 非功能需求

- 執行需可重現（deterministic，給定相同輸入與 seed）。
- 需支援長時間執行的進度回報，避免 UI 端無回應。
- Preview 與正式 Run 必須使用相同的 [Runtime Adapter](./runtime-adapter.md) 與節點實作，避免兩種路徑產生不同結果。
- 必須安全處理取消、cache eviction 與 out-of-order completion；舊 request 不得覆蓋新 revision。

## 相依模組

- 上游：[Pipeline Definition](./pipeline-definition.md)、[Event Bus](./event-bus.md)、[Data Repository](./data-repository.md)
- 下游：[Runtime Adapter](./runtime-adapter.md)、[Data Repository](./data-repository.md)、[Event Bus](./event-bus.md)

## 待確認

- System Overview canvas 尚未畫出本節點與 [Data Repository](./data-repository.md) 的 artifact I/O 邊線，需在下一次 Canvas 架構更新時補上。
