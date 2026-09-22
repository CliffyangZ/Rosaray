# System Overview

> 情境來源：Thinking、MCV WorkFlow

Rosaray 是一個「積木式演算法 Pipeline」的醫學影像研究工作站。使用者在 [Workstation GUI](./workstation-gui.md) 中設計演算法 pipeline、載入影像資料、觀察運算結果。本文件是 System Overview canvas 的文字對應版本，作為未來 Rust 專案的規格依據（本階段僅整理需求文件，不實作程式碼）。

## 分層總覽

| 層級 | 職責 | 主要元件 |
|---|---|---|
| [Core](./core.md) | 跨層事件溝通樞紐 | [Event Bus](./event-bus.md) |
| [Algorithm Layer](./algorithm-layer.md) | 使用者設計演算法積木與 pipeline | [Algorithm Designer](./algorithm-designer.md)、[Pipeline Definition](./pipeline-definition.md) |
| [Execution Layer](./execution-layer.md) | 執行 pipeline、串接運算後端 | [Execution Engine](./execution-engine.md)、[Runtime Adapter](./runtime-adapter.md) |
| [Algorithm Runtime](./algorithm-runtime.md) | 實際運算硬體／推論後端 | [CPU](./cpu.md)、[GPU / CUDA](./gpu-cuda.md)、[ONNX / Runtime](./onnx-runtime.md) |
| [Data Layer](./data-layer.md) | 影像與結果的存取、快取、持久化 | [Data Engine](./data-engine.md)、[Data Repository](./data-repository.md)、[Memory Cache](./memory-cache.md)、[Persistent Storage](./persistent-storage.md) |
| [Presentation Layer](./presentation-layer.md) | 影像檢視與結果視覺化 | [Presentation Engine](./presentation-engine.md)、[Image Viewer](./image-viewer.md)、[Overlay Renderer](./overlay-renderer.md)、[Annotation Renderer](./annotation-renderer.md)、[Measurement Renderer](./measurement-renderer.md)、[3D Renderer](./3d-renderer.md) |
| (獨立節點) | 使用者操作介面殼層 | [Workstation GUI](./workstation-gui.md) |

## 主要資料流（依 canvas 邊線整理）

1. 使用者在 [Workstation GUI](./workstation-gui.md) 透過 [Algorithm Designer](./algorithm-designer.md) 設計積木，組成 [Pipeline Definition](./pipeline-definition.md)。
2. [Pipeline Definition](./pipeline-definition.md) 交給 [Execution Engine](./execution-engine.md) 執行；[Execution Engine](./execution-engine.md) 透過 [Runtime Adapter](./runtime-adapter.md) 分派到 [Algorithm Runtime](./algorithm-runtime.md)（[CPU](./cpu.md) / [GPU / CUDA](./gpu-cuda.md) / [ONNX / Runtime](./onnx-runtime.md)）。
3. [Execution Engine](./execution-engine.md) 經 [Data Repository](./data-repository.md) 取得輸入 Image、讀寫中繼 artifact 與正式結果；Preview artifact 路由至 [Memory Cache](./memory-cache.md)，正式 Run 結果依 policy 路由至 [Persistent Storage](./persistent-storage.md)。
4. [Execution Engine](./execution-engine.md) 將進度與結果（Progress / Result），以及 Preview lifecycle events 發佈到 [Event Bus](./event-bus.md)。
5. [Event Bus](./event-bus.md) 廣播事件（Events）給 [Algorithm Designer](./algorithm-designer.md)、[Workstation GUI](./workstation-gui.md)、[Presentation Engine](./presentation-engine.md)，形成回饋迴圈。
6. [Presentation Engine](./presentation-engine.md) 從 [Data Repository](./data-repository.md) 讀取影像／結果（Read Image / Result），並分派給 [Image Viewer](./image-viewer.md)、[Overlay Renderer](./overlay-renderer.md)、[Annotation Renderer](./annotation-renderer.md)、[Measurement Renderer](./measurement-renderer.md)、[3D Renderer](./3d-renderer.md) 進行渲染。
7. [Presentation Engine](./presentation-engine.md) 也會發出 UI Events 回 [Event Bus](./event-bus.md)，並直接與 [Workstation GUI](./workstation-gui.md) 互動。
8. [Data Engine](./data-engine.md) 負責把資料寫入 [Data Repository](./data-repository.md)，再分流至 [Memory Cache](./memory-cache.md)（易失／快速存取）與 [Persistent Storage](./persistent-storage.md)（持久化）。

## 文件狀態

- 目前僅完成需求文件與反向連結標註，尚未建立 Rust 專案骨架。
- 每個元件文件包含：職責、資料流、功能／非功能需求、相依模組、未來 Rust 對應與待確認事項。
- `System Overview.canvas` 上的每個節點都已加上對應的 wikilink，可在 Obsidian 的反向連結／關聯面板中互相追溯。
