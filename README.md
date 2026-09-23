# Rosaray

<p align="center">
  <a href=".github/icon.png"><img src=".github/icon.png" alt="Rosaray icon" width="160"></a>
</p>

<p align="center">
  <a href="https://cliffyangz.github.io/Rosaray/"><img src="https://img.shields.io/badge/docs-Rosaray-ea7233?logo=readthedocs&logoColor=white&label=docs" alt="Rosaray docs"></a>
</p>

Rosaray 是一個以瀏覽器為介面的本機優先（local-first）口內影像研究工作站，協助研究者匯入口內照片、組合影像處理流程、觀察分割結果，並記錄可重現的實驗指標。

目前專案聚焦於「影像分析研究工作流」的前端原型，尚未連接正式後端、雲端儲存或臨床診斷服務。Rosaray 僅供研究與展示使用，不能取代醫師判讀或作為醫療診斷依據。

## Current status

目前以 Rust 開發後端的 Data Layer 與前端的交互運作。

## Introduction

Rosaray 的目標是把醫療影像分析所需的工作集中在同一個研究介面中：

- 以影像檢視器查看原始影像、處理後影像與 mask overlay。
- 以可視化 pipeline 組合前處理、分割與量化步驟。
- 以 metrics、run history 與 validation 面板檢查結果及資料集狀態。
- 保留資料集 fingerprint、pipeline graph、參數與執行時間，方便比較不同實驗設定。

工作站預設不載入任何影像。可從已連線的本機服務選擇資料集影像，或匯入 PNG、JPEG 影像進行測試。

## Getting Started

### Requirements

- Node.js 20.19+（Vite 7 的執行需求）
- npm

### Install and run

可直接從專案根目錄執行啟動腳本；它會在首次使用時安裝相依套件、啟動本機後端服務並開啟已連線的瀏覽器工作站：

```bash
./scripts/launch.sh
```

腳本會要求設定 Rosaray 專案密碼，用於加密本機儲存的資料；請在每次開啟同一個專案時輸入相同密碼。若要在非互動環境啟動，請先設定 `ROSARAY_PASSPHRASE`。

或手動啟動：

在專案根目錄執行：

```bash
cd frontend
npm install
npm run dev
```

接著開啟終端機輸出的本機網址，通常是 `http://localhost:5173`。

### Build and preview

```bash
cd frontend
npm run build
npm run preview
```

### First experiment

1. 開啟工作站後，從左側 Explorer 匯入影像，或從 Dataset 選擇本機服務中的影像。
2. 在右側 Pipeline 檢查預設流程：`Image source → Normalize → Gaussian blur → Threshold → Morphology → Area measurement`。
3. 在 Inspector 調整節點參數，或從 Algorithms 拖曳新的節點到 pipeline。
4. 按下 **Run pipeline**（`⌘/Ctrl + Enter`）。
5. 在中央檢視器切換 **Input**、**Output** 與 **Overlay**，並在下方查看 Metrics、Run history 與 Validation。

若要測試自己的影像，可使用 **File → Import Image…**。匯入影像會先轉為灰階；新匯入的影像沒有 reference mask，因此無法計算 Dice，且 patient ID 與 dataset split 需要後續確認。

## Features

### Image workspace

- 預設為空白工作區；可載入本機服務中的資料集影像或匯入 PNG、JPEG。
- 支援 PNG、JPEG 匯入。
- 支援影像分頁、縮放、平移、Fit to window 與 1:1 檢視。
- 可切換原始影像、pipeline 輸出與預測 mask overlay。

### Visual pipeline

- 以節點方式建立影像處理流程，並透過 typed ports 連接節點。
- 支援 pipeline 驗證：檢查影像型別是否相容、是否存在 cycle，以及是否有唯一的 Image source。
- 可調整下列節點：
  - **Normalize**：依百分位數拉伸影像強度。
  - **Gaussian blur**：降低影像雜訊。
  - **Threshold**：使用 manual 或 Otsu threshold 產生 mask。
  - **Morphology**：執行 open、close、erode 或 dilate。
  - **Area measurement**：計算前景像素、面積與連通元件數。
  - **ONNX segmentation**：保留模型節點介面，模型載入目前仍未完成。

### Metrics and experiment tracking

- 計算 Dice（僅在影像具有 reference mask 時）。
- 顯示估算面積（mm²）、前景像素數、連通元件數與各步驟執行時間。
- Run history 保存 run ID、影像、Dice、面積、dataset fingerprint、pipeline graph hash、seed 與執行時間。
- 每次執行會產生一筆可比較的 run snapshot。

### Dataset and validation

- Dataset 面板顯示影像的 patient、split 與 reference mask 配對狀態。
- 以 dataset fingerprint 辨識資料集設定變更。
- Validation 面板檢查 patient 是否跨 split、reference mask 是否配對，以及 pipeline 是否有效。
- 對缺少 patient ID 的匯入影像顯示提醒。

## License

License 尚未設定。
