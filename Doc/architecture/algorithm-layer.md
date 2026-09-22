# Algorithm Layer

> 總覽：[System Overview](./overview.md)

## 定位

對應 Thinking 中「積木式演算法 Pipeline」與「將論文的邏輯轉換成演算法積木」的核心需求層，讓使用者以節點（積木）方式設計演算法流程。

## 包含元件

- [Algorithm Designer](./algorithm-designer.md)
- [Pipeline Definition](./pipeline-definition.md)

## 跨層責任邊界

- 只負責 pipeline 的「設計」與「定義」，不負責實際執行（執行交給 [Execution Layer](./execution-layer.md)）。
- 透過 [Event Bus](./event-bus.md) 接收系統事件（例如執行結果回饋），不直接與 [Execution Layer](./execution-layer.md) 以外的下游耦合。

## 需求

### 功能需求

- 提供積木（節點）目錄：對應 README 中 Normalize、Gaussian blur、Threshold、Morphology、Area measurement、ONNX segmentation 等節點類型。
- 支援節點間 typed ports 連接與 pipeline 驗證（型別相容性、cycle 偵測、唯一 Image source）。
- 將設計結果序列化為 [Pipeline Definition](./pipeline-definition.md)，可儲存／載入。

### 非功能需求

- pipeline 定義需為與執行環境無關的中立格式（serializable），以便未來跨後端（[CPU](./cpu.md) / [GPU / CUDA](./gpu-cuda.md) / [ONNX / Runtime](./onnx-runtime.md)）重用。

## 待確認

- Pipeline 版本相容性（新增／移除節點類型後舊 pipeline 如何相容）尚未定義。
