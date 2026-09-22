# Presentation Engine

> 所屬層級：[Presentation Layer](./presentation-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

協調影像與結果的視覺化呈現，依當前檢視模式從 [Data Repository](./data-repository.md) 讀取資料，並分派渲染工作給各 renderer 元件。是 canvas 上唯一同時連接 [Event Bus](./event-bus.md)、[Workstation GUI](./workstation-gui.md)、[Data Repository](./data-repository.md) 與所有 renderer 的節點。

## 資料流

- 輸入：
  - [Event Bus](./event-bus.md) → `Events`（含 `PreviewStarted / PreviewReady / PreviewFailed / PreviewCancelled`）
  - [Workstation GUI](./workstation-gui.md) → 使用者操作（檢視模式切換等）
  - [Data Repository](./data-repository.md) → 影像／結果資料（`Read Image / Result`）
- 輸出：
  - → [Event Bus](./event-bus.md)：`UI Events`
  - → [Image Viewer](./image-viewer.md)、[Overlay Renderer](./overlay-renderer.md)、[Annotation Renderer](./annotation-renderer.md)、[Measurement Renderer](./measurement-renderer.md)、[3D Renderer](./3d-renderer.md)：渲染指令

## 需求

### 功能需求

- 支援 Input／Output／Overlay 檢視模式切換（對應 README「中央檢視器切換」）。
- Preview 模式依 selected node 顯示各 Input port、Output 或 Before／After；實際影像由 `artifact_ref` 經 [Data Repository](./data-repository.md) 讀取。
- 只接受符合目前 `request_id` 與 `pipeline_revision` 的 Preview 結果；舊 revision 不得更新畫面。
- Preview 執行中顯示 loading／stale 狀態；失敗時保留上一個成功畫面並明確標示 failed 與失敗節點。
- 依使用者操作（縮放、平移、分頁）更新所有子 renderer 的顯示狀態，保持同步。

### 非功能需求

- 需將重運算（例如大影像解碼）與渲染分離，避免阻塞互動操作。
- 對 [Data Repository](./data-repository.md) 的讀取需具備快取意識，避免每次操作都觸發完整重新讀取。

## 相依模組

- 上游：[Event Bus](./event-bus.md)、[Workstation GUI](./workstation-gui.md)、[Data Repository](./data-repository.md)
- 下游：[Image Viewer](./image-viewer.md)、[Overlay Renderer](./overlay-renderer.md)、[Annotation Renderer](./annotation-renderer.md)、[Measurement Renderer](./measurement-renderer.md)、[3D Renderer](./3d-renderer.md)、[Event Bus](./event-bus.md)

## 待確認

- 各 renderer 是否需要獨立的渲染時脈（frame budget），或統一由本模組排程？
