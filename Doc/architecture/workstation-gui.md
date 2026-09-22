# Workstation GUI

> 備註：canvas 上未歸屬於任一分層群組，作為整體操作介面殼層

## 職責

使用者操作的最外層介面殼層，承載 [Algorithm Designer](./algorithm-designer.md) 的設計畫布與 [Presentation Engine](./presentation-engine.md) 的檢視結果，是唯一與 [Presentation Engine](./presentation-engine.md) 有直接邊線（非透過 [Event Bus](./event-bus.md)）的元件。

## 資料流

- 輸入：
  - [Event Bus](./event-bus.md) → `Events`
- 輸出：
  - → [Presentation Engine](./presentation-engine.md)：直接介面呼叫（檢視模式切換、使用者操作）

## 需求

### 功能需求

- 整合 Explorer（範例／匯入影像清單）、Pipeline 編輯區、Inspector（節點參數）、中央檢視器、Metrics／Run history／Validation 面板（對應 README 完整工作流程）。
- 提供 Import Image、File → Import Image 等操作入口。

### 非功能需求

- 需支援鍵盤快捷鍵（例如 README 提到的 `⌘/Ctrl + Enter` 執行 pipeline）。
- 作為 local-first 應用殼層，啟動與操作不應依賴網路連線。

## 相依模組

- 上游：[Event Bus](./event-bus.md)
- 下游：[Presentation Engine](./presentation-engine.md)（直接呼叫）、間接透過 [Event Bus](./event-bus.md) 影響 [Algorithm Designer](./algorithm-designer.md)

## 待確認

- 本節點未被劃入任何 canvas 分層群組，是否應歸類為獨立的「Shell / Application」層？在未來 Rust 專案中會影響 crate 劃分。
