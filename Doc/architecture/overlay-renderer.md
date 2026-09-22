# Overlay Renderer

> 所屬層級：[Presentation Layer](./presentation-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

將預測 mask 等結果以疊圖（overlay）方式繪製在 [Image Viewer](./image-viewer.md) 之上，對應 README「切換原始影像、pipeline 輸出與預測 mask overlay」。

## 資料流

- 輸入：
  - [Presentation Engine](./presentation-engine.md) → 渲染指令（含 mask／結果資料）

## 需求

### 功能需求

- 支援 overlay 透明度調整與開關切換。
- 支援多種 overlay 來源（pipeline 中間結果、最終 mask）。

### 非功能需求

- Overlay 繪製需與底圖（[Image Viewer](./image-viewer.md)）座標系一致，不可有偏移。

## 相依模組

- 上游：[Presentation Engine](./presentation-engine.md)

## 待確認

- 多個 overlay 同時顯示時的疊圖順序與混色規則尚未定義。
