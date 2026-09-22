# CPU

> 所屬層級：[Algorithm Runtime](./algorithm-runtime.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

提供不依賴特殊硬體的預設運算後端，確保系統在任何環境下都能執行 pipeline（fallback 後端）。

## 資料流

- 輸入：
  - [Runtime Adapter](./runtime-adapter.md) → 運算請求
- 輸出：
  - → [Runtime Adapter](./runtime-adapter.md)：運算結果

## 需求

### 功能需求

- 實作 README 既有的影像處理演算法：Normalize、Gaussian blur、Threshold（manual／Otsu）、Morphology（open／close／erode／dilate）、Area measurement、Dice 計算。

### 非功能需求

- 作為所有其他後端不可用時的保底選項，需保證正確性優先於效能。
- 需與 [GPU / CUDA](./gpu-cuda.md)、[ONNX / Runtime](./onnx-runtime.md) 共用相同的運算契約（輸入輸出格式一致）。

## 相依模組

- 上游：[Runtime Adapter](./runtime-adapter.md)

## 待確認

- 是否需要 SIMD／多執行緒優化以縮短大影像的處理時間？
