# Execution Layer

> 總覽：[System Overview](./overview.md)

## 定位

負責把 [Algorithm Layer](./algorithm-layer.md) 定義好的 pipeline 實際跑起來，並將實際運算工作分派給 [Algorithm Runtime](./algorithm-runtime.md) 中的硬體／推論後端。

## 包含元件

- [Execution Engine](./execution-engine.md)
- [Runtime Adapter](./runtime-adapter.md)

## 跨層責任邊界

- 只以 [Pipeline Definition](./pipeline-definition.md) 的 immutable graph snapshot 作為可執行內容；Preview command 經 [Event Bus](./event-bus.md) 傳入，但本層不參與 pipeline 的設計邏輯。
- 執行進度與結果一律透過 [Event Bus](./event-bus.md) 發佈（Progress / Result），不直接呼叫 [Presentation Layer](./presentation-layer.md)。
- 實際運算能力委派給 [Runtime Adapter](./runtime-adapter.md) → [Algorithm Runtime](./algorithm-runtime.md)，本層不直接實作矩陣／影像運算核心。

## 需求

### 功能需求

- 依 [Pipeline Definition](./pipeline-definition.md) 的節點與連線順序（topological order）逐步執行。
- 每個節點執行後回報進度與中間結果（供 run history 使用，見 README「Metrics and experiment tracking」）。
- 支援單張 Image Preview：只執行 Image source 到 target node 的 ancestor subgraph，重用未失效 artifact，並採 latest-wins 取消過期 revision。
- 支援執行失敗時的錯誤回報與中止。

### 非功能需求

- 執行需可重現：相同 pipeline + 相同輸入 + 相同 seed 需產生相同結果（對應 README run snapshot 需求）。
- 需能在無 GPU 環境下降級為 [CPU](./cpu.md) 執行。

## 待確認

- 是否支援節點級別的平行執行（同一 pipeline 內無相依節點平行跑）？
