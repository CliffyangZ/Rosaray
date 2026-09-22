# Introduction

Rosaray 是一個以瀏覽器為介面的本機優先（local-first）口內影像研究工作站，協助研究者匯入口內照片、組合影像處理流程、觀察分割結果，並記錄可重現的實驗指標。

目前專案聚焦於「影像分析研究工作流」的前端原型，尚未連接正式後端、雲端儲存或臨床診斷服務。

::: danger 重要聲明
Rosaray 僅供研究與展示使用，不能取代醫師判讀或作為醫療診斷依據。
:::

## 目標

Rosaray 的目標是把口內影像分析所需的工作集中在同一個研究介面中：

- 以影像檢視器查看原始影像、處理後影像與 mask overlay。
- 以可視化 pipeline 組合前處理、分割與量化步驟。
- 以 metrics、run history 與 validation 面板檢查結果及資料集狀態。
- 保留資料集 fingerprint、pipeline graph、參數與執行時間，方便比較不同實驗設定。

目前內建的範例是由程式產生的灰階合成口內影像與牙齒 reference mask；也可以匯入 PNG 或 JPEG 影像進行測試。
