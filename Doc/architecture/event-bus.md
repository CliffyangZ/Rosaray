# Event Bus

> 所屬層級：[Core](./core.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

作為系統唯一的跨層事件樞紐，接收各層發布的事件並轉發給訂閱者，讓 [Algorithm Layer](./algorithm-layer.md)、[Execution Layer](./execution-layer.md)、[Presentation Layer](./presentation-layer.md)、[Workstation GUI](./workstation-gui.md) 彼此不需直接依賴。

## 資料流

- 輸入（訂閱／接收發布）：
  - [Algorithm Designer](./algorithm-designer.md) → `PreviewRequested`
  - [Execution Engine](./execution-engine.md) → `Progress / Result`
  - [Execution Engine](./execution-engine.md) → `PreviewStarted / PreviewReady / PreviewFailed / PreviewCancelled`
  - [Presentation Engine](./presentation-engine.md) → `UI Events`
- 輸出（廣播）：
  - → [Algorithm Designer](./algorithm-designer.md)：`Events`
  - → [Workstation GUI](./workstation-gui.md)：`Events`
  - → [Presentation Engine](./presentation-engine.md)：`Events`

## 需求

### 功能需求

- 提供 topic／channel 概念，區分 `Progress / Result`、`UI Events`、一般 `Events`。
- Preview lifecycle 至少包含：
  - `PreviewRequested`：`request_id`、`pipeline_revision`、`image_id`、`target_node_id`、immutable pipeline snapshot、pipeline graph hash 與 seed。
  - `PreviewStarted`：request context 與實際開始時間。
  - `PreviewReady`：request context、依 Input port ID 索引的 Input artifact references、Output artifact reference、執行時間與 cache hit／miss。
  - `PreviewFailed`：request context、失敗 node ID、錯誤碼與安全的錯誤訊息。
  - `PreviewCancelled`：request context 與取消原因。
- Preview 事件只傳控制資料與 artifact reference，不在 bus 上傳遞影像像素或大型 tensor。
- Preview 訂閱者必須以 `request_id` 與 `pipeline_revision` 過濾過期事件；同一 context 採 latest-wins。
- 支援一對多廣播（一個發布者、多個訂閱者）。
- 支援訂閱者動態註冊／取消註冊（例如 [Presentation Engine](./presentation-engine.md) 尚未初始化前不應遺失事件，或需提供事件緩衝）。

### 非功能需求

- 傳遞延遲需低到不影響 UI 互動流暢度（[Workstation GUI](./workstation-gui.md) 與 [Presentation Engine](./presentation-engine.md) 都依賴此匯流排更新畫面）。
- 執行緒安全：[Execution Engine](./execution-engine.md) 可能在背景執行緒發布事件，UI 端在主執行緒訂閱。

## 相依模組

- 上游（發布事件者）：[Algorithm Designer](./algorithm-designer.md)、[Execution Engine](./execution-engine.md)、[Presentation Engine](./presentation-engine.md)
- 下游（訂閱事件者）：[Algorithm Designer](./algorithm-designer.md)、[Workstation GUI](./workstation-gui.md)、[Presentation Engine](./presentation-engine.md)

## 待確認

- 是否需要事件優先順序（例如錯誤事件優先於進度事件）？
