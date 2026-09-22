# Persistent Storage

> 所屬層級：[Data Layer](./data-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

提供資料的持久化儲存，是影像、reference mask、pipeline 定義與 run history 的最終存放位置（local-first，對應 README 專案定位）。

## 資料流

- 輸入：
  - [Data Repository](./data-repository.md) → 待持久化資料
- 輸出：
  - → [Data Repository](./data-repository.md)：讀取持久化資料

## 需求

### 功能需求

- 持久化：影像原始檔、reference mask、pipeline 定義（[Pipeline Definition](./pipeline-definition.md)）、run history（run ID、Dice、面積、fingerprint、graph hash、seed、執行時間）。
- 支援 Export Bundle（對應 README 目前列為介面預留功能）將資料打包匯出。

### 非功能需求

- 需為本機檔案系統儲存（local-first），不依賴雲端儲存服務。
- 資料寫入需具備一定程度的原子性，避免程式中斷造成資料損毀。

## 相依模組

- 上游：[Data Repository](./data-repository.md)

## 待確認

- 儲存格式（例如自訂目錄結構 vs. 嵌入式資料庫）尚未決定。
