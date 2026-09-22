# 目前狀態

目前版本是可在瀏覽器執行的前端研究原型：

- 演算法執行在瀏覽器端，尚無正式後端 API。
- **Import Model**、**Save Project**、**Export Bundle** 與 Evidence 搜尋目前是介面預留功能。
- ONNX segmentation 節點需要模型資產與後端／載入流程，現階段不會實際執行。
- 內建影像與 reference mask 是合成資料，不代表真實臨床資料表現。

## 進行中的設計

目前正在進行 [Data Layer 規格](/features/data-layer/spec) 的設計，規劃以 Rust 本機服務承載 Core、Algorithm、Execution 與 Data 層，詳見 [實作計畫](/features/data-layer/plan) 與 [研究決策](/features/data-layer/research)。

## License

License 尚未設定。
