# 功能

## 影像工作區

- 內建多張合成口內影像，包含不同病患、視角與 train / validation / test split。
- 支援 PNG、JPEG 匯入。
- 支援影像分頁、縮放、平移、Fit to window 與 1:1 檢視。
- 可切換原始影像、pipeline 輸出與預測 mask overlay。

## 視覺化 Pipeline

- 以節點方式建立影像處理流程，並透過 typed ports 連接節點。
- 支援 pipeline 驗證：檢查影像型別是否相容、是否存在 cycle，以及是否有唯一的 Image source。
- 可調整下列節點：
  - **Normalize**：依百分位數拉伸影像強度。
  - **Gaussian blur**：降低影像雜訊。
  - **Threshold**：使用 manual 或 Otsu threshold 產生 mask。
  - **Morphology**：執行 open、close、erode 或 dilate。
  - **Area measurement**：計算前景像素、面積與連通元件數。
  - **ONNX segmentation**：保留模型節點介面，模型載入目前仍未完成。

## Metrics 與實驗追蹤

- 計算 Dice（僅在影像具有 reference mask 時）。
- 顯示估算面積（mm²）、前景像素數、連通元件數與各步驟執行時間。
- Run history 保存 run ID、影像、Dice、面積、dataset fingerprint、pipeline graph hash、seed 與執行時間。
- 每次執行會產生一筆可比較的 run snapshot。

## 資料集與驗證

- Dataset 面板顯示影像的 patient、split 與 reference mask 配對狀態。
- 以 dataset fingerprint 辨識資料集設定變更。
- Validation 面板檢查 patient 是否跨 split、reference mask 是否配對，以及 pipeline 是否有效。
- 對缺少 patient ID 的匯入影像顯示提醒。
