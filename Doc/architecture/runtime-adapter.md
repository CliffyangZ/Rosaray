# Runtime Adapter

> 所屬層級：[Execution Layer](./execution-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

對 [Execution Engine](./execution-engine.md) 隱藏底層運算後端差異，將單一節點的運算請求分派到 [Algorithm Runtime](./algorithm-runtime.md) 中合適的後端（[CPU](./cpu.md) / [GPU / CUDA](./gpu-cuda.md) / [ONNX / Runtime](./onnx-runtime.md)）。

## 資料流

- 輸入：
  - [Execution Engine](./execution-engine.md) → 節點運算請求
- 輸出：
  - → [Algorithm Runtime](./algorithm-runtime.md)（[CPU](./cpu.md) / [GPU / CUDA](./gpu-cuda.md) / [ONNX / Runtime](./onnx-runtime.md)）：實際運算呼叫

## 需求

### 功能需求

- 依節點型別與硬體可用性選擇後端（例如 ONNX segmentation 節點優先用 [ONNX / Runtime](./onnx-runtime.md)，一般影像處理節點用 [CPU](./cpu.md) 或 [GPU / CUDA](./gpu-cuda.md)）。
- 提供統一的錯誤與逾時處理，向 [Execution Engine](./execution-engine.md) 回報一致格式的結果。

### 非功能需求

- 後端選擇邏輯需可設定／覆寫（例如使用者強制指定 CPU-only 模式）。
- 需能偵測硬體可用性變化（例如 GPU 不可用時自動降級）。

## 相依模組

- 上游：[Execution Engine](./execution-engine.md)
- 下游：[CPU](./cpu.md)、[GPU / CUDA](./gpu-cuda.md)、[ONNX / Runtime](./onnx-runtime.md)（皆屬 [Algorithm Runtime](./algorithm-runtime.md)）

## 待確認

- 多節點併發請求時的後端資源排程策略尚未定義。
