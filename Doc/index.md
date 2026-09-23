---
layout: home

hero:
  name: "Rosaray"
  text: "本機運行的醫療影像研究工作站"
  tagline: 匯入資料、組合影像處理流程、觀察分割結果，並記錄可重現的實驗指標。
  actions:
    - theme: brand
      text: 快速開始
      link: /guide/getting-started
    - theme: alt
      text: 系統架構
      link: /architecture/overview

features:
  - title: 本機運行、離線可用
    details: 所有演算法在瀏覽器端執行，匯入、預覽、正式執行與儲存都不需要網路連線，也不自動上傳資料。
  - title: 積木式 Pipeline 演算法設計
    details: 以節點組合前處理、分割與量化步驟，透過 typed ports 連接，並支援 cycle、型別與來源唯一性驗證。
  - title: 可重現的實驗紀錄
    details: 每次正式 Run 保存 run ID、dataset fingerprint、pipeline graph hash、seed 與執行時間，方便比較不同實驗設定。
  - title: 資料完整性與驗證
    details: Dataset 面板顯示 patient、split 與 reference mask 配對狀態，並以 fingerprint 偵測資料集設定變更。
---

::: warning 研究與展示用途
Rosaray 僅供研究與展示使用，不能取代醫師判讀或作為醫療診斷依據。
:::

## 關於本文件

本文件站整合了 Rosaray 專案的完整規格：

- **指南** — 從 README 拆解的使用說明、功能清單與目前狀態。
- **架構** — 對應 `specs/Rosaray/system_architecture.canvas` 的文字規格，涵蓋 Core、Algorithm、Execution、Data、Presentation 各層元件。
- **Data Layer 規格** — 目前進行中的功能設計（`specs/001-data-layer-design`），包含需求、研究決策、資料模型與實作計畫。

查看原始碼：[:github: CliffyangZ/Rosaray](https://github.com/CliffyangZ/Rosaray)
