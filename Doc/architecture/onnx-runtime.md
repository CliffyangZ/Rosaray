# ONNX / Runtime

> 所屬層級：[Algorithm Runtime](./algorithm-runtime.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

提供模型推論後端，支撐 README 中「ONNX segmentation」節點（目前為保留介面、模型載入尚未完成）。

## 資料流

- 輸入：
  - [Runtime Adapter](./runtime-adapter.md) → 模型推論請求（含模型路徑／輸入影像）
- 輸出：
  - → [Runtime Adapter](./runtime-adapter.md)：推論結果（例如分割 mask）

## 需求

### 功能需求

- 支援模型載入（對應 README 明列的既有缺口：「Import Model」目前是介面預留功能）。
- 支援與 [CPU](./cpu.md) / [GPU / CUDA](./gpu-cuda.md) 一致的輸入輸出契約，讓分割節點結果可與其他節點串接。

### 非功能需求

- 模型載入失敗需有明確錯誤訊息回報至 [Execution Engine](./execution-engine.md)，而非靜默失敗。
- 需支援模型資產的版本標記，方便 run history 記錄使用的模型版本。

## 相依模組

- 上游：[Runtime Adapter](./runtime-adapter.md)

## 待確認

- 模型資產從何處取得／如何驗證（對應 README「模型資產與後端／載入流程」尚未完成）？
