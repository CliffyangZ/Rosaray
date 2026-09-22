# Image Viewer

> 所屬層級：[Presentation Layer](./presentation-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

渲染原始影像與 pipeline 輸出影像的基礎檢視器，提供縮放、平移等操作。

## 資料流

- 輸入：
  - [Presentation Engine](./presentation-engine.md) → 渲染指令（含影像資料）

## 需求

### 功能需求

- 支援影像分頁、縮放、平移、Fit to window 與 1:1 檢視（對應 README「Image workspace」）。
- 支援切換原始影像、selected node 的各 Input port、Output，以及 Before／After 對照。
- Preview 尚未完成時可保留上一個成功畫面，但必須顯示 loading／stale／failed 狀態，不得將舊 artifact 誤標為目前結果。

### 非功能需求

- 大尺寸影像需支援漸進式載入或降採樣預覽，避免縮放操作卡頓。

## 相依模組

- 上游：[Presentation Engine](./presentation-engine.md)

## 待確認

- 是否需要支援多影像並排比較（例如 before／after）？
