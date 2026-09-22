# Measurement Renderer

> 所屬層級：[Presentation Layer](./presentation-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

將量化結果（面積、像素數、連通元件數、Dice 等）以視覺化方式呈現在檢視器上，對應 README「Metrics and experiment tracking」。

## 資料流

- 輸入：
  - [Presentation Engine](./presentation-engine.md) → 渲染指令（含量測資料）

## 需求

### 功能需求

- 顯示估算面積（mm²）、前景像素數、連通元件數（對應 README「Area measurement」節點輸出）。
- 顯示 Dice 分數（僅在影像具有 reference mask 時，對應 README 限制）。

### 非功能需求

- 數值顯示需與目前檢視的影像／run 嚴格對應，避免切換影像時顯示舊數據。

## 相依模組

- 上游：[Presentation Engine](./presentation-engine.md)

## 待確認

- 是否需要支援量測結果的圖表化呈現（例如跨 run 趨勢圖）？
