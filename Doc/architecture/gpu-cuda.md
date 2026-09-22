# GPU / CUDA

> 所屬層級：[Algorithm Runtime](./algorithm-runtime.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

提供 GPU 加速運算後端，用於大影像或高頻率運算節點，縮短 pipeline 執行時間。

## 資料流

- 輸入：
  - [Runtime Adapter](./runtime-adapter.md) → 運算請求
- 輸出：
  - → [Runtime Adapter](./runtime-adapter.md)：運算結果

## 需求

### 功能需求

- 與 [CPU](./cpu.md) 提供相同的運算契約，讓 [Runtime Adapter](./runtime-adapter.md) 可無感切換。
- 需能回報自身是否可用（裝置偵測），供 [Runtime Adapter](./runtime-adapter.md) 決策使用。

### 非功能需求

- 不可用時需優雅降級到 [CPU](./cpu.md)，不可讓整個 pipeline 執行失敗。
- 需回報記憶體用量，避免 OOM 造成執行中斷。

## 相依模組

- 上游：[Runtime Adapter](./runtime-adapter.md)

## 待確認

- 目標支援的 CUDA 版本範圍、跨平台（例如 Apple Silicon 上無 CUDA）替代方案尚未定義。
