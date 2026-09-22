# Algorithm Runtime

> 總覽：[System Overview](./overview.md)

## 定位

實際執行運算的硬體／推論後端集合，被 [Execution Layer](./execution-layer.md) 透過 [Runtime Adapter](./runtime-adapter.md) 間接呼叫，對上層隱藏後端差異。

## 包含元件

- [CPU](./cpu.md)
- [GPU / CUDA](./gpu-cuda.md)
- [ONNX / Runtime](./onnx-runtime.md)

## 跨層責任邊界

- 只提供運算能力，不知道 pipeline 語意、不直接存取 [Data Layer](./data-layer.md) 或 [Event Bus](./event-bus.md)。
- 由 [Runtime Adapter](./runtime-adapter.md) 統一決定要用哪個後端執行（依硬體可用性、節點需求）。

## 需求

### 功能需求

- 三種後端須提供一致的抽象介面（相同輸入輸出契約），讓 [Runtime Adapter](./runtime-adapter.md) 可無感切換。
- [ONNX / Runtime](./onnx-runtime.md) 需支援模型載入（對應 README 中「ONNX segmentation 節點介面保留、模型載入未完成」的既有缺口）。

### 非功能需求

- 需能偵測執行環境是否具備 [GPU / CUDA](./gpu-cuda.md)，並在缺少時自動降級為 [CPU](./cpu.md)。
- 效能量測需可回報給 [Execution Engine](./execution-engine.md)（單步執行時間，對應 README「各步驟執行時間」）。

## 待確認

- GPU 記憶體不足時的降級／排隊策略尚未定義。
