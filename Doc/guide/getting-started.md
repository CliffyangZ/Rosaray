# 快速開始

## 需求

- Node.js 20.19+（Vite 7 的執行需求）
- npm

## 安裝與執行

在專案根目錄執行：

```bash
cd frontend
npm install
npm run dev
```

接著開啟終端機輸出的本機網址，通常是 `http://localhost:5173`。

## 建置與預覽

```bash
cd frontend
npm run build
npm run preview
```

## 第一次實驗

1. 開啟工作站後，從左側 Explorer 選擇一張內建範例影像。
2. 在右側 Pipeline 檢查預設流程：`Image source → Normalize → Gaussian blur → Threshold → Morphology → Area measurement`。
3. 在 Inspector 調整節點參數，或從 Algorithms 拖曳新的節點到 pipeline。
4. 按下 **Run pipeline**（`⌘/Ctrl + Enter`）。
5. 在中央檢視器切換 **Input**、**Output** 與 **Overlay**，並在下方查看 Metrics、Run history 與 Validation。

若要測試自己的影像，可使用 **File → Import Image…**。匯入影像會先轉為灰階；新匯入的影像沒有 reference mask，因此無法計算 Dice，且 patient ID 與 dataset split 需要後續確認。
