# Data Engine

> 所屬層級：[Data Layer](./data-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

負責資料的匯入與寫入邏輯，將外部影像（PNG／JPEG）與執行結果正規化後寫入 [Data Repository](./data-repository.md)。

## 資料流

- 輸出：
  - → [Data Repository](./data-repository.md)：正規化後的影像／結果資料

## 需求

### 功能需求

- 支援 PNG、JPEG 匯入，匯入時轉為灰階（對應 README「匯入影像會先轉為灰階」）。
- 為匯入影像產生／要求 patient ID、split 資訊；缺少時需標記提醒（對應 README「Validation 面板...對缺少 patient ID 的匯入影像顯示提醒」）。
- 計算並附加 dataset fingerprint。

### 非功能需求

- 匯入流程需為本機操作（local-first），不上傳雲端。
- 匯入失敗（格式不支援、檔案損毀）需明確回報，不可讓 [Data Repository](./data-repository.md) 存入不完整資料。

## 相依模組

- 下游：[Data Repository](./data-repository.md)

## 待確認

- 匯入影像目前無 reference mask，是否需要提供手動標註流程以支援 Dice 計算？
