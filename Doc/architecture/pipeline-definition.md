# Pipeline Definition

> 所屬層級：[Algorithm Layer](./algorithm-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

儲存 pipeline typed DAG 的結構化定義，作為 [Algorithm Designer](./algorithm-designer.md) 與 [Execution Engine](./execution-engine.md) 之間可序列化、可驗證且可版本化的資料契約。

## 資料流

- 輸入：
  - [Algorithm Designer](./algorithm-designer.md) → 節點圖（設計結果）
- 輸出：
  - → [Execution Engine](./execution-engine.md)：可執行的 pipeline 定義

## 需求

### 功能需求

- 以可序列化格式（例如 JSON／自訂 schema）描述：
  - `schema_version` 與單調遞增的 `graph_revision`。
  - Nodes：穩定 `node_id`、`node_type`、`implementation_version`、canonical parameters、typed Input／Output ports。
  - Edges：穩定 edge ID，以及來源／目的 node ID 與 port ID。
- Graph 必須是 directed acyclic graph（DAG），並可透過 cycle detection、typed-port validation 與 topological sort 驗證。
- 提供 pipeline graph hash，支援 README 中「pipeline graph hash」用於 run 比較的需求。Hash 只納入會影響運算的 canonical graph 內容，不納入 UI layout、selected node 或 `graph_revision`。
- 穩定 node／port ID 不可因畫布移動、重新排序或儲存／載入而改變。
- 支援儲存／載入（對應 README 目前列為介面預留功能的 Save Project）。
- Undo／Redo command history 屬於 [Algorithm Designer](./algorithm-designer.md) 的 session state；預設不包含在儲存的 pipeline 定義內。

### 非功能需求

- 格式需與執行後端無關（不綁定 [CPU](./cpu.md) / [GPU / CUDA](./gpu-cuda.md) / [ONNX / Runtime](./onnx-runtime.md)）。
- 需可版本化，方便未來 schema 演進時偵測相容性。
- Canonical serialization 必須穩定，確保相同 graph 內容產生相同 graph hash 與 Preview cache key 材料。

## 相依模組

- 上游：[Algorithm Designer](./algorithm-designer.md)
- 下游：[Execution Engine](./execution-engine.md)

## 待確認

- Pipeline 定義的 schema 版本控制與遷移策略尚未制定。
