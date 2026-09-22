# 專案結構

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
