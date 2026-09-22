# Algorithm Designer

> 所屬層級：[Algorithm Layer](./algorithm-layer.md)　|　所屬 Canvas：[System Overview](./overview.md)

## 職責

提供使用者以節點（積木）方式設計演算法 pipeline 的互動邏輯，對應 Thinking 中「將論文的邏輯轉換成演算法積木」需求。除編輯與驗證外，也負責發出單張影像 Preview 請求、呈現節點 Input／Output 狀態，以及協調編輯回復與運算狀態回溯；實際運算仍由 [Execution Engine](./execution-engine.md) 負責。

## Pipeline 圖模型

Pipeline 是 typed directed acyclic graph（DAG）$G=(V,E)$：

- $V$ 是演算法節點集合。每個節點具有穩定 `node_id`、`node_type`、`implementation_version`、canonical parameters，以及具型別的 Input／Output ports。
- $E$ 是連線集合。每條 edge 必須由一個 Output port 指向型別相容的 Input port，並保存兩端的 node／port ID。
- Graph 必須只有一個 Image source，且不得包含 cycle、型別不相容連線、未連接的必要 Input 或重複佔用的單值 Input port。
- Cycle detection 與型別檢查在編輯端同步完成；[Execution Engine](./execution-engine.md) 執行前仍需重新驗證，不能信任 UI 傳入的 graph。
- Topological sort 決定執行順序；ancestors 查詢決定 Preview 所需子圖，descendants 查詢決定 graph 修改後需要失效的輸出。

## 狀態模型

### PipelineDraftState

描述尚在編輯中的 pipeline：

- `graph`：目前 nodes、edges 與參數。
- `revision`：每次會影響運算結果的編輯後單調遞增，用來辨識過期 Preview 回應。
- `selected_node_id`：目前 Inspector 與 Preview 的觀察節點；只改變選取不增加 revision。
- `undo_stack`／`redo_stack`：保存新增／移除節點、連線、參數變更等可逆 command。

Undo／Redo 回復的是 graph 編輯狀態，而非嘗試反轉演算法本身。Threshold、量化或 resize 等不可逆運算，只能透過歷史 graph 與中繼輸出快取還原。

### NodePreviewState

描述「某個 pipeline revision 對某張 Image、截至某個 target node」的運算狀態：

- 識別欄位：`request_id`、`pipeline_revision`、`image_id`、`target_node_id`。
- Artifact 欄位：依 Input port ID 索引的 `input_artifact_refs`，以及 `output_artifact_ref`；事件中只傳 reference，不傳影像像素。
- 狀態：`idle`、`stale`、`queued`、`running`、`ready`、`failed`、`cancelled`。
- 診斷欄位：節點執行時間、cache hit／miss、錯誤節點與錯誤訊息。

Time-travel 是選取任一節點並查看該節點當時的 Input／Output artifact，不會修改 graph，也不會產生新的 Undo command。

## 單張影像增量 Preview

1. 使用者選定一張 Image 與一個 target node；預設 target 是目前選取的節點。
2. 節點、連線或參數變更時，先增加 `revision` 並同步驗證 graph。有效 graph 以 300 ms debounce 發出 `PreviewRequested`；無效 graph 不送出請求，且目前結果保持 `stale`。
3. [Execution Engine](./execution-engine.md) 只執行 Image source 到 target node 的 ancestor subgraph，不執行無關分支。
4. 修改某節點時，該節點及其 descendants 的 Preview 狀態變為 `stale`；未受影響的 ancestors 可沿用 [Memory Cache](./memory-cache.md) 中的 artifact。
5. 同一 Preview context 採 latest-wins：新 revision 會取消或取代舊請求；即使舊結果較晚抵達，revision 不符時也不可更新畫面。
6. `PreviewReady` 後，中央檢視器可切換 selected node 的 Input、Output 或 Before／After。切換觀察節點時若已有快取，不需重新運算。
7. `PreviewFailed` 時保留上一個成功畫面，但必須標示 `stale`／`failed`、失敗節點與錯誤訊息，不可讓舊畫面看似最新結果。

Preview 與正式 Run 使用相同的 [Runtime Adapter](./runtime-adapter.md) 與節點實作，避免預覽結果與正式結果不一致；Preview artifact 為易失性資料，只進 [Memory Cache](./memory-cache.md)，不寫入 run history 或 [Persistent Storage](./persistent-storage.md)。

## 回朔與快取

- Undo／Redo 回到既有 graph 狀態後，先用內容式 cache key 查找中繼輸出；命中則直接恢復 Preview，未命中才重新執行。
- Cache key 由 `node_type`、`implementation_version`、canonical parameters、所有 Input artifact hashes 與 seed 組成，不使用 UI `revision`，使相同內容在不同 revision 間可以重用。
- 非 deterministic 節點必須明確宣告不可快取，或提供足以重現輸出的 seed／外部狀態版本；否則不得把舊 artifact 當成有效結果。
- Undo／Redo history 預設僅存在目前編輯 session；儲存專案只保存當前 [Pipeline Definition](./pipeline-definition.md)，不保存整份 command history。

## 資料流

- 輸入：
  - [Event Bus](./event-bus.md) → `PreviewStarted / PreviewReady / PreviewFailed / PreviewCancelled` 與正式 Run 事件。
- 輸出：
  - → [Pipeline Definition](./pipeline-definition.md)：目前可序列化的 typed DAG。
  - → [Event Bus](./event-bus.md)：`PreviewRequested`，包含 Image、revision 與 target node context。

## 需求

### 功能需求

- 提供積木目錄（節點型錄），包含 Normalize、Gaussian blur、Threshold、Morphology、Area measurement、ONNX segmentation。
- 支援節點拖曳、typed ports 連線、參數調整，以及 command-based Undo／Redo。
- 即時驗證型別相容性、cycle、唯一 Image source 與必要 Input 完整性。
- 支援選取任一節點查看 Input／Output、Preview 狀態、執行時間與 cache hit／miss。
- 在 graph 有效且已選擇 Image 時，自動觸發增量 Preview。

### 非功能需求

- 編輯驗證與狀態切換需立即回饋，不等待後端運算完成。
- [Algorithm Designer](./algorithm-designer.md) 不直接執行影像演算法；所有 Preview 與正式 Run 均委派給 [Execution Engine](./execution-engine.md)。
- Preview 必須處理取消、out-of-order response 與 cache eviction，不得顯示錯誤 revision 的結果。
- Preview graph 第一版限定為 DAG，不支援 loop、feedback edge 或具有未宣告 mutable state 的節點。

## 驗收情境

- 建立 cycle、連接不相容 port、缺少必要 Input 或加入第二個 Image source 時，graph 驗證失敗且不送出 Preview。
- Undo／Redo 可精確還原 nodes、edges、parameters 與相應 revision 的 graph 內容。
- 在分支 graph 中修改一個節點時，只有該節點及其 descendants 變為 `stale`；其他分支的 artifact 仍可重用。
- Undo／Redo 回到曾運算過的參數狀態時，內容式 cache key 命中且不重跑節點。
- 快速連續修改參數時，只有最新 `pipeline_revision` 的結果可以更新中央檢視器。
- Preview 執行失敗時顯示失敗節點與錯誤，並保留但明確標示上一個成功畫面為 stale。
- 選取不同節點可切換各 Input port 與 Output；已有 artifact 時不重跑無關分支。
- Preview 不建立正式 run history；正式 Run 仍保存 pipeline graph hash、seed、輸入輸出雜湊與執行時間。

## 相依模組

- 上游：[Event Bus](./event-bus.md)（接收 Preview 與正式 Run 狀態事件）
- 下游：[Pipeline Definition](./pipeline-definition.md)、[Event Bus](./event-bus.md)（輸出 graph 與 Preview 請求）
- 間接相依：[Execution Engine](./execution-engine.md)、[Memory Cache](./memory-cache.md)、[Presentation Engine](./presentation-engine.md)

## 待確認

- 自訂／第三方節點如何註冊到節點目錄，並宣告 deterministic、cacheability 與 Input／Output port schema？
