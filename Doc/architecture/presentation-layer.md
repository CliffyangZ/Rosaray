# Presentation Layer

> 總覽：[System Overview](./overview.md)

## 定位

負責影像與結果的視覺化呈現，是使用者透過 [Workstation GUI](./workstation-gui.md) 觀察 pipeline 輸出的主要層級（對應 Thinking「圖片演算結果」與情境「可以顯示」）。

## 包含元件

- [Presentation Engine](./presentation-engine.md)
- [Image Viewer](./image-viewer.md)
- [Overlay Renderer](./overlay-renderer.md)
- [Annotation Renderer](./annotation-renderer.md)
- [Measurement Renderer](./measurement-renderer.md)
- [3D Renderer](./3d-renderer.md)

## 跨層責任邊界

- 只讀取 [Data Repository](./data-repository.md)，不寫入資料（寫入是 [Data Engine](./data-engine.md) 的責任）。
- 透過 [Event Bus](./event-bus.md) 接收 Events、發送 UI Events；同時與 [Workstation GUI](./workstation-gui.md) 有直接介面呼叫（canvas 上唯一一條跨層直接邊線）。

## 需求

### 功能需求

- [Presentation Engine](./presentation-engine.md) 依當前檢視模式（Input／Output／Overlay，見 README）分派渲染工作給對應的 renderer 元件。
- 支援影像分頁、縮放、平移、Fit to window 與 1:1 檢視（對應 README「Image workspace」）。

### 非功能需求

- 渲染效能需支援互動式操作（縮放／平移即時回應），不可阻塞主執行緒。
- 各 renderer（Overlay／Annotation／Measurement／3D）需可獨立開關，互不影響彼此渲染結果。

## 待確認

- [3D Renderer](./3d-renderer.md) 的資料來源（是否需要體積資料／多切面）目前尚未在 README 或 canvas 中定義。
