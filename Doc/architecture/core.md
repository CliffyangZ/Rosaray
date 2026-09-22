# Core

> 總覽：[System Overview](./overview.md)

## 定位

Core 是整個系統唯一橫跨其他所有層級的模組，提供跨層之間鬆耦合的事件溝通機制，避免 [Algorithm Layer](./algorithm-layer.md)、[Execution Layer](./execution-layer.md)、[Presentation Layer](./presentation-layer.md) 與 [Workstation GUI](./workstation-gui.md) 互相直接依賴。

## 包含元件

- [Event Bus](./event-bus.md)

## 跨層責任邊界

- 只負責事件的發佈／訂閱與傳遞，不持有業務邏輯，不直接存取 [Data Layer](./data-layer.md)。
- 所有跨層通訊（Events、Progress / Result、UI Events）都必須經過此層，其他層之間不應互相直接呼叫。

## 需求

### 功能需求

- 提供發佈／訂閱（pub/sub）介面，支援多個訂閱者。
- 支援至少三類事件：Pipeline 執行進度／結果、UI 事件、一般系統事件。

### 非功能需求

- 事件傳遞需為非阻塞（non-blocking），避免拖慢 [Execution Engine](./execution-engine.md) 或 [Presentation Engine](./presentation-engine.md)。
- 需可在單一行程（in-process）情境下運作；未來若切分成服務，需保留可替換的傳輸層。

## 待確認

- 事件是否需要保證順序（ordering）與至少一次送達（at-least-once delivery）？
- 是否需要事件重播（replay）以支援 run history／可重現性需求（見 README「Metrics and experiment tracking」）？
