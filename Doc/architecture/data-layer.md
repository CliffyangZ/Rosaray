# Data Layer

> 總覽：[System Overview](./overview.md)

## 定位

管理影像、reference mask、pipeline 結果與 run history 等資料的讀寫、快取與持久化，是唯一被 [Presentation Layer](./presentation-layer.md) 讀取資料的來源（對應 canvas 上「Read Image / Result」邊線）。

## 包含元件

- [Data Engine](./data-engine.md)
- [Data Repository](./data-repository.md)
- [Memory Cache](./memory-cache.md)
- [Persistent Storage](./persistent-storage.md)

## 跨層責任邊界

- 不主動發起跨層呼叫；被動提供讀寫介面給 [Presentation Engine](./presentation-engine.md)（讀）、[Data Engine](./data-engine.md)（寫）與 [Execution Engine](./execution-engine.md)（輸入 Image、Preview／正式 Run artifact I/O）。
- [Execution Engine](./execution-engine.md) 一律透過 [Data Repository](./data-repository.md) 存取本層，不直接耦合 [Memory Cache](./memory-cache.md) 或 [Persistent Storage](./persistent-storage.md)。

## 需求

### 功能需求

- 管理影像的 patient／split／reference mask 配對狀態（對應 README「Dataset and validation」）。
- 提供 dataset fingerprint 計算，偵測資料集設定變更。
- 分流資料至 [Memory Cache](./memory-cache.md)（熱資料、低延遲）與 [Persistent Storage](./persistent-storage.md)（冷資料、持久化）。
- Preview artifact 只進 [Memory Cache](./memory-cache.md)；正式 Run 結果依 run policy 持久化，兩者使用相同 artifact reference 讀取介面。

### 非功能需求

- 需支援 local-first（本機優先）儲存，不依賴雲端服務（對應 README 專案定位）。
- 匯入影像（PNG／JPEG）需正規化為統一內部格式後再進入 [Data Repository](./data-repository.md)。

## 待確認

- 需在 System Overview canvas 補上 [Execution Engine](./execution-engine.md) ↔ [Data Repository](./data-repository.md) 的 artifact I/O 邊線，使圖面與本規格一致。
