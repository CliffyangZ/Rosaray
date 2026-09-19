# Rosaray

<p align="center">
  <a href=".github/icon.png"><img src=".github/icon.png" alt="Rosaray icon" width="160"></a>
</p>

Rosaray 是一個以瀏覽器為介面的本機優先（local-first）口內影像研究工作站，協助研究者匯入口內照片、組合影像處理流程、觀察分割結果，並記錄可重現的實驗指標。

目前專案聚焦於「影像分析研究工作流」的前端原型，尚未連接正式後端、雲端儲存或臨床診斷服務。Rosaray 僅供研究與展示使用，不能取代醫師判讀或作為醫療診斷依據。

## Introduction

Rosaray 的目標是把口內影像分析所需的工作集中在同一個研究介面中：

- 以影像檢視器查看原始影像、處理後影像與 mask overlay。
- 以可視化 pipeline 組合前處理、分割與量化步驟。
- 以 metrics、run history 與 validation 面板檢查結果及資料集狀態。
- 保留資料集 fingerprint、pipeline graph、參數與執行時間，方便比較不同實驗設定。

目前內建的範例是由程式產生的灰階合成口內影像與牙齒 reference mask；也可以匯入 PNG 或 JPEG 影像進行測試。

## Getting Started

### Requirements

- Node.js 20.19+（Vite 7 的執行需求）
- npm

### Install and run

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

1. 開啟工作站後，從左側 Explorer 選擇一張內建範例影像。
2. 在右側 Pipeline 檢查預設流程：`Image source → Normalize → Gaussian blur → Threshold → Morphology → Area measurement`。
3. 在 Inspector 調整節點參數，或從 Algorithms 拖曳新的節點到 pipeline。
4. 按下 **Run pipeline**（`⌘/Ctrl + Enter`）。
5. 在中央檢視器切換 **Input**、**Output** 與 **Overlay**，並在下方查看 Metrics、Run history 與 Validation。

若要測試自己的影像，可使用 **File → Import Image…**。匯入影像會先轉為灰階；新匯入的影像沒有 reference mask，因此無法計算 Dice，且 patient ID 與 dataset split 需要後續確認。

## Features

### Image workspace

- 內建多張合成口內影像，包含不同病患、視角與 train / validation / test split。
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

## Project structure

```text
frontend/
├── index.html          # 工作站入口
├── package.json        # Vite scripts 與依賴
└── src/
    ├── main.js         # UI、影像檢視器、pipeline 與 run state
    ├── registry.js     # 節點定義、參數與執行邏輯
    ├── algo.js         # blur、Otsu、morphology、components、Dice
    ├── samples.js      # 合成範例影像與 reference mask
    ├── styles.css      # 工作站介面樣式
    └── util.js         # 共用 UI 與工具函式
```

## Current status

目前版本是可在瀏覽器執行的前端研究原型：

- 演算法執行在瀏覽器端，尚無正式後端 API。
- **Import Model**、**Save Project**、**Export Bundle** 與 Evidence 搜尋目前是介面預留功能。
- ONNX segmentation 節點需要模型資產與後端／載入流程，現階段不會實際執行。
- 內建影像與 reference mask 是合成資料，不代表真實臨床資料表現。

## License

License 尚未設定。
