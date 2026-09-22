# Annotation Renderer

> 所屬層級：[Presentation Layer](./presentation-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

繪製與影像相關的標註資訊（例如標記點、reference mask 邊界、病灶標籤），輔助研究者比對結果。

## 資料流

- 輸入：
  - [Presentation Engine](./presentation-engine.md) → 渲染指令（含標註資料）

## 需求

### 功能需求

- 顯示 reference standard／人工標註（對應 MCV WorkFlow 中「標註與 reference standard 建立」）。
- 支援標註圖層獨立開關，不影響 [Overlay Renderer](./overlay-renderer.md) 顯示的模型結果。

### 非功能需求

- 標註繪製需與底圖座標系一致，並支援縮放時同步縮放。

## 相依模組

- 上游：[Presentation Engine](./presentation-engine.md)

## 待確認

- 是否需要支援使用者在檢視器上互動新增／編輯標註（而不僅是唯讀顯示）？
