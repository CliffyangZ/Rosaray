# Feature Specification: Rosaray Data Layer

**Feature Branch**: `N/A — no branch hook configured`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "幫我設計 Data Layer，從 specs/Rosaray 理解整體架構"

## Clarifications

### Session 2026-09-22

- Q: 匯入影像應複製進專案、僅連結外部檔案，或讓使用者選擇？ → A: 僅連結外部檔案。
- Q: 正式 Run 是否保存輸入快照？ → A: 每次正式 Run 保存實際送入 pipeline 的灰階輸入 artifact。
- Q: 正式 Run 後如何處理資料集變更？ → A: 變更建立新的 immutable Dataset Version，舊版本保持不可變。
- Q: 本機研究資料的靜態加密範圍為何？ → A: Rosaray 管理的持久資料預設靜態加密，外部來源檔案不由 Rosaray 加密。
- Q: Export Bundle 應如何保護？ → A: Export Bundle 一律加密，匯入時需要獨立憑證。
- Q: 前端載入資料的最小可用範圍為何？ → A: 顯示 Dataset／影像清單、選取影像與基本 metadata，並可切換 reference mask overlay。
- Q: 瀏覽器前端如何持續存取本機資料？ → A: 瀏覽器前端連接本機 Rosaray service，由該 service 提供資料存取。
- Q: 前端要支援哪種資料載入入口？ → A: 同時支援單檔、多檔與資料夾掃描確認。
- Q: 批次匯入時如何取得 patient、split 與 reference mask 配對？ → A: 支援選配 metadata manifest，未匹配項目標示 incomplete。
- Q: Explorer 是否顯示影像縮圖？ → A: 僅對目前可見的清單項目延遲載入縮圖。
- Q: Rosaray 應採用哪種執行與部署邊界？ → A: 瀏覽器前端連接本機 Rosaray service；Core、Algorithm、Execution 與 Data 層均由 service 承載，Event Bus 僅用於 service 內部通訊。
- Q: service 內跨層通訊應如何區分直接 API 與 Event Bus？ → A: 需要即時結果的 Command／Query 使用 typed API；非同步狀態與 lifecycle 通知使用 Event Bus。
- Q: Data Engine 與 Data Repository 的責任如何劃分？ → A: Data Engine 負責資料應用流程與規則；Data Repository 僅提供查詢與持久化抽象，Execution Engine 透過專用 repository API 寫入 artifact 與 Run。
- Q: Presentation Engine 與 Renderers 應在哪個行程執行？ → A: 全部位於瀏覽器前端，只透過本機 Rosaray service API 取得 display descriptor、影像與 artifact。
- Q: Event Bus 是否需要持久化與事件重播？ → A: 不持久化，只保證同一 Run／Preview context 內有序；重啟或重新連線後由 Repository 查詢目前狀態。

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 建立可信任的研究資料集 (Priority: P1)

醫學影像研究者可將口內 PNG／JPEG 影像匯入本機專案，補充去識別化的受試者識別碼與資料切分，配對 reference mask，並在執行實驗前看見資料完整性與資料洩漏風險。

**Why this priority**: 所有 Preview、正式 Run 與評估結果都依賴正確且可識別版本的輸入資料；資料集若不可信，後續結果即使可運算也沒有研究價值。

**Independent Test**: 以一組包含正常影像、重複影像、缺少受試者識別碼、跨 split 受試者與不相容 mask 的測試資料匯入，即可獨立驗證資料正規化、配對、fingerprint 與 Validation 結果。

**Acceptance Scenarios**:

1. **Given** 一批可讀取的 PNG／JPEG 影像，**When** 研究者匯入資料，**Then** 每張影像都有穩定識別、外部來源連結、灰階研究用表示與明確的 patient／split／reference mask 狀態，且專案不複製原始影像。
2. **Given** 同一受試者的影像被分配到不同 split，**When** 資料集驗證完成，**Then** 系統指出所有衝突影像與受影響的 split，且不將資料集標示為可用於正式評估。
3. **Given** reference mask 缺失、無法配對或尺寸不相容，**When** 研究者檢視 Validation，**Then** 系統分別顯示「不可計算 reference-based metric」或「配對無效」，且不產生誤導性 Dice 結果。
4. **Given** 資料集內容或會影響研究的 metadata 發生變更，**When** 研究者確認變更，**Then** 系統建立具有新 fingerprint 的 immutable Dataset Version，且舊版本與其 Run 關聯保持不變；只有顯示名稱等非研究內容變更時不建立新版本。
5. **Given** 研究者從前端選擇單一檔案、多個檔案或資料夾，**When** 掃描完成，**Then** 系統先顯示可匯入影像、略過項目、重複內容、錯誤與預計建立的資料項目數量，尚未改變 Dataset Version。
6. **Given** 研究者檢視批次掃描結果，**When** 明確確認匯入，**Then** 系統只納入確認清單中的支援影像並建立新的 Dataset Version；取消時不得建立任何資料項目或版本。
7. **Given** Import Batch 包含 metadata manifest，**When** 掃描預覽完成，**Then** 每個候選影像都顯示 manifest 配對的 patient ID、split 與 reference mask，並明確列出未知影像、重複 mapping、缺少檔案與不相容 mask。
8. **Given** Import Batch 未提供 manifest 或有候選影像未匹配，**When** 研究者確認匯入，**Then** 未匹配影像以 incomplete metadata 狀態建立，且系統不得從檔名或資料夾名稱猜測 patient、split 或 mask 關係。

---

### User Story 2 - 在前端瀏覽研究資料 (Priority: P2)

研究者開啟 Rosaray 後，可在 Workstation GUI 的 Explorer 瀏覽 Dataset Versions 與影像清單；選取可用影像後，中央檢視器顯示灰階原始影像、基本 metadata，並在有有效 reference mask 時切換 overlay。

**Why this priority**: 這是 Data Layer 與 Presentation Layer 的第一條完整使用者可見資料流，也是開始設計 pipeline、Preview 或正式 Run 前的必要能力。

**Independent Test**: 載入一個同時包含正常影像、缺少 mask、無效 mask 與遺失外部來源的 Dataset Version，驗證清單、選取、影像顯示、metadata、overlay 與錯誤狀態，即可獨立交付此故事。

**Acceptance Scenarios**:

1. **Given** 專案含有可讀取的 Dataset Versions，**When** 研究者開啟 Explorer，**Then** 前端列出 Dataset Version 與其 Image Assets，並顯示每張影像的可用性及 validation 摘要，而不必先載入完整像素資料。
2. **Given** 研究者選取狀態為 `available` 的 Image Asset，**When** 影像載入完成，**Then** 中央檢視器顯示與選取 identity 相符的灰階影像，以及 patient ID、split、尺寸與 reference mask 狀態。
3. **Given** 選取影像具有有效且尺寸相容的 reference mask，**When** 研究者開啟 overlay，**Then** mask 與目前影像正確對齊顯示，且關閉 overlay 後回到未覆蓋的影像。
4. **Given** 選取影像缺少或具有無效 reference mask，**When** 研究者查看 overlay 控制，**Then** 前端停用該控制並顯示原因，不產生虛假的空 mask。
5. **Given** 研究者在上一張影像尚未載入完成前選取另一張影像，**When** 較舊請求晚到，**Then** 中央檢視器仍只顯示最新選取的 Image Asset。
6. **Given** 本機 Rosaray service 未啟動、連線中斷或拒絕存取，**When** 前端嘗試列出或讀取資料，**Then** 前端顯示明確的 service unavailable 狀態與重試方式，不顯示過期內容為目前資料。
7. **Given** Explorer 顯示的影像清單超過目前可見範圍，**When** 研究者捲動清單，**Then** 前端只為目前可見及鄰近項目請求縮圖，未進入可見範圍的項目不觸發完整影像讀取。

---

### User Story 3 - 低延遲檢視 Pipeline 中繼結果 (Priority: P3)

研究者調整 pipeline 節點後，可在中央檢視器快速查看指定節點的 Input、Output 或 Before／After；系統重用仍有效的中繼結果，也能在快取失效時安全地重新運算。

**Why this priority**: 即時視覺回饋是積木式演算法設計的核心體驗，但 Preview 不應污染正式實驗紀錄或被誤認為持久資料。

**Independent Test**: 對同一影像與 pipeline 重複調整、復原參數並強制淘汰快取，可獨立驗證 artifact reference、內容等價重用、cache miss 與過期結果隔離。

**Acceptance Scenarios**:

1. **Given** 某節點的輸入、參數、實作版本與 seed 均未改變，**When** 研究者再次請求相同內容的 Preview，**Then** 系統回傳同一內容的 artifact，且不建立正式 Run 紀錄。
2. **Given** Preview artifact 已被淘汰，**When** 使用者再次查看該節點，**Then** 系統將其視為可恢復的 cache miss，要求重新運算，並且不宣稱發生持久資料遺失。
3. **Given** 使用者快速連續調整參數，**When** 舊請求晚於新請求完成，**Then** 只能顯示目前 image、pipeline revision 與 target node 所對應的結果。
4. **Given** Preview 失敗，**When** 中央檢視器收到失敗狀態，**Then** 上一個成功結果可保留但必須明確標示 stale／failed，並指出失敗節點。

---

### User Story 4 - 保存可重現的正式實驗 (Priority: P4)

研究者執行完整 pipeline 後，可保存並比較正式 Run；每筆結果都能回溯到確切輸入資料集、影像、pipeline 內容、參數、seed、產出、指標與執行狀態。

**Why this priority**: 正式 Run 是研究比較、錯誤分析與日後重現的證據單位，必須與易失性的 Preview 清楚分離。

**Independent Test**: 以固定資料集與 pipeline 執行兩次正式 Run，重新啟動工作站後比較兩筆紀錄，即可驗證完整保存、關聯追蹤與可重現性。

**Acceptance Scenarios**:

1. **Given** 有效的 Dataset Version 與 pipeline，**When** 正式 Run 成功，**Then** run history 包含唯一 run ID、不可變 Dataset Version 與 fingerprint、image identity、實際運算的灰階輸入 artifact、pipeline graph identity、seed、節點版本、時間資訊、最終 artifacts、metrics 與完整狀態。
2. **Given** 正式 Run 在保存期間被中斷，**When** 專案再次開啟，**Then** 不完整資料不會被標示為成功，已存在的完整 Run 仍可讀取。
3. **Given** 任一已保存的最終結果或 metric，**When** 研究者查看其來源，**Then** 可回溯到產生它的 Run、pipeline、輸入 artifact 與資料集版本。
4. **Given** 正式 Run 失敗，**When** 研究者查看 run history，**Then** 可看到失敗狀態、失敗階段與安全的錯誤摘要，但不會出現假冒成功的最終結果。
5. **Given** 未經授權的程序直接讀取 Rosaray 管理的專案資料，**When** 它檢視 Dataset Versions、Run Input Artifacts、masks、results 或 metrics，**Then** 無法取得可理解的研究內容。

---

### User Story 5 - 攜出與恢復研究專案 (Priority: P5)

研究者可將選定資料集、pipeline 與正式 Run 匯出成自足且可驗證的研究 Bundle，並在另一個本機工作環境中確認內容完整後恢復；Preview 與暫存狀態不被當作研究成果匯出。

**Why this priority**: local-first 工作流仍需要可控的備份、交接與封存方式，但此能力建立在前三個故事的資料契約之上。

**Independent Test**: 匯出含影像、mask、pipeline 與多筆 Run 的專案，再於空白工作環境匯入並比對 fingerprint、關聯與結果，即可獨立驗證。

**Acceptance Scenarios**:

1. **Given** 研究者選定要分享的資料集與正式 Run，並提供獨立匯出憑證，**When** 建立 Export Bundle，**Then** Bundle 以加密形式包含內容清單、資料來源與版本關聯、完整性證明及所有必要的持久資料，但不包含 Preview cache 或可直接解密自身的憑證。
2. **Given** Bundle 內容完整、版本可支援且研究者提供正確的獨立憑證，**When** 匯入至空白工作環境，**Then** 恢復後的 dataset fingerprint、pipeline identity、run identity、metrics 與 artifact 內容皆與來源一致。
3. **Given** Bundle 已損毀、內容被竄改或版本不受支援，**When** 嘗試匯入，**Then** 系統在改變既有專案前拒絕匯入並指出可理解的原因。

### Edge Cases

- 檔案格式副檔名正確但內容損毀、加密、截斷或無法解碼時，匯入必須失敗且不得留下可被誤用的半成品。
- 同一影像內容以不同檔名重複匯入時，系統必須辨識內容重複並讓研究者決定是否保留獨立資料項目；不得靜默重複計入資料集。
- 資料夾掃描期間遇到無權限子目錄、連結循環、隱藏檔案或非 PNG／JPEG 檔案時，必須停止該路徑或略過該項目並在確認畫面列出，不得無限掃描或靜默改變 Dataset。
- Metadata manifest 指向不存在或不在 Import Batch 中的檔案、重複指定同一影像、使用不允許的 split，或將一個 mask 配給多張不相容影像時，必須在預覽中逐項指出，且不得套用有歧義的 mapping。
- 影像沒有 patient ID 或 split 時，可保留為待整理資料，但正式評估前 Validation 必須阻擋或警示對應用途。
- Reference mask 與影像尺寸、資料型別或對應關係不相容時，不得計算 Dice 或其他 reference-based metric。
- 正式資料的 artifact reference 找不到、內容不符或已損毀時，讀取必須失敗並指出受影響的 Run；不得退回不相干資料。
- Preview cache 在記憶體壓力、工作站重啟或手動清除後消失時，正式影像、Run 與結果不得受影響。
- 儲存空間不足、權限被撤銷或保存過程中程式終止時，不完整寫入不得取代上一個有效狀態。
- 同時有多個讀取者與一個正式 Run 寫入結果時，讀取者只能看到上一個完整狀態或新的完整狀態，不得看到部分更新。
- 舊專案或 Export Bundle 的資料契約版本不受支援時，必須先判斷可否安全遷移；不可理解的內容不得直接載入。
- 使用者移動、重新命名或刪除外部來源檔案時，對應 Image Asset 必須進入 `source_missing` 狀態並阻擋以該外部來源啟動新的 Preview／正式 Run；既有正式 Run 仍可讀取，並可使用其灰階輸入 artifact 重現運算。
- Rosaray 無法取得或驗證解密所需憑證時，必須拒絕開啟受保護內容並提供恢復指引，不得以空白資料覆寫專案或顯示部分解密內容。
- Export Bundle 的獨立憑證缺失或錯誤時，必須在揭露 manifest、metadata 或影像內容前拒絕匯入；憑證不得存放在同一 Bundle 中。
- 縮圖缺失、產生失敗或已過期時，Explorer 必須顯示不洩漏影像內容的 placeholder 與狀態；不得以其他影像的縮圖代替，也不得阻止使用者嘗試載入該影像的中央檢視內容。

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 系統 MUST 以 Data Layer 作為 Execution Layer 與 Presentation Layer 讀取影像、mask、artifact、metric 與 Run 資料的唯一一致來源；上層不得依賴資料實際存放位置。
- **FR-002**: 系統 MUST 支援以外部檔案連結匯入 PNG 與 JPEG 影像，不將原始影像複製進專案；系統可建立可重建的灰階研究表示，但外部來源仍是該 Image Asset 的權威內容。
- **FR-003**: 系統 MUST 為每個匯入項目指派穩定 identity，並記錄外部來源位置、匯入時的內容 identity、尺寸、來源建立時間、匯入時間與資料狀態。
- **FR-004**: 系統 MUST 允許研究者為影像維護去識別化 patient ID、train／validation／test split 與 reference mask 配對；缺漏欄位必須有明確的 incomplete 狀態。
- **FR-005**: 系統 MUST 在匯入前驗證可讀性與支援格式；失敗時不得建立可供執行或顯示的資料項目。
- **FR-006**: 系統 MUST 偵測內容相同的重複影像，避免在沒有明確使用者決定時將其重複計入資料集。
- **FR-007**: 系統 MUST 為每個 immutable Dataset Version 產生穩定 fingerprint；影像內容、patient 對應、split 或 reference mask 等影響研究解讀的內容改變時必須建立 fingerprint 不同的新版本，純顯示資訊改變時不得建立新版本。
- **FR-008**: 系統 MUST 驗證 patient 跨 split、缺少 patient ID、缺少或不相容 reference mask、重複內容與不完整資料，並回報受影響資料項目及嚴重程度。
- **FR-009**: 系統 MUST 支援依 dataset、image、patient、split、run ID、artifact reference 與內容 identity 找到相應資料，並為查無資料、資料損毀與暫時不可用提供可區分結果。
- **FR-010**: 系統 MUST 以 artifact reference 代表大型影像或運算結果，reference 至少可識別內容、資料類型、shape、大小、建立時間、產生來源與持久性等級。
- **FR-011**: 系統 MUST 讓正式資料的 reference 在工作站重啟後仍可解析；Preview-only reference 在內容被淘汰後必須回報可恢復的 cache miss。
- **FR-012**: 系統 MUST 優先提供近期需要的資料，且在暫存內容不存在時，自動改由持久資料取得或明確要求上游重建，不能回傳錯誤內容冒充命中。
- **FR-013**: 系統 MUST 將 Preview artifact 視為易失性資料，不納入正式 run history、專案封存或 Export Bundle。
- **FR-014**: 系統 MUST 以會影響運算內容的節點類型、實作版本、canonical parameters、所有輸入內容 identity 與 seed 判斷 Preview artifact 是否可重用；UI revision、節點位置與選取狀態不得影響內容等價性。
- **FR-015**: 系統 MUST 禁止重用不可重現節點的 artifact，除非該節點提供足以唯一決定輸出的 seed 與外部狀態版本。
- **FR-016**: 系統 MUST 保存 Preview artifact 的 lineage 與診斷資訊，使呼叫者能判斷輸入、輸出、產生節點、執行時間及 cache hit／miss。
- **FR-017**: 系統 MUST 隔離不同 image、pipeline revision、target node 與 request context 的結果，使過期 Preview 無法被誤認為目前畫面資料。
- **FR-018**: 系統 MUST 為每次正式 Run 建立不可混淆的 Run Record，包含 run ID、狀態、不可變 Dataset Version identity 與 fingerprint、image identity、灰階輸入 artifact identity、pipeline graph identity、seed、節點實作版本、開始／結束時間、輸出內容 identity、metrics 與錯誤摘要。
- **FR-019**: 系統 MUST 持久保存正式 Run 實際送入 pipeline 的灰階輸入 artifact、pipeline snapshot、最終 artifacts、metrics 與 lineage；中繼 artifacts 是否保存由明確的 run policy 決定，且該 policy 必須成為 Run Record 的一部分。
- **FR-020**: 系統 MUST 確保正式 Run 的完成狀態與其必要資料共同成立；只要必要資料未完整保存，該 Run 就不得被標示為成功。
- **FR-021**: 系統 MUST 保留失敗與取消的 Run Record，但不得為其建立誤導性的成功結果；錯誤資訊不得暴露原始影像內容或非必要的受試者資訊。
- **FR-022**: 系統 MUST 讓任何正式結果與 metric 可回溯至唯一 Run、pipeline snapshot、輸入 image、dataset fingerprint 與直接上游 artifacts。
- **FR-023**: 系統 MUST 在讀取正式資料時驗證內容完整性；驗證失敗時必須停止使用該內容、標示受影響關聯並保留其他未受影響資料可用。
- **FR-024**: 系統 MUST 以 local-first 為預設，匯入、Preview、正式 Run、讀取與保存均不得要求網路連線，也不得自動將資料傳送至外部服務。
- **FR-025**: 系統 MUST 將 direct identifiers 視為不允許的預設資料；研究者使用的 patient ID 應為去識別化識別碼，且任何自由文字 metadata 都必須清楚提示不得輸入可識別個資。
- **FR-026**: 系統 MUST 支援建立自足且完整加密的 Export Bundle，內容包含所選資料、reference masks、pipeline snapshots、正式 Run Records、metrics、artifacts、資料契約版本、內容清單與完整性證明；Preview cache 不得包含其中。
- **FR-027**: 系統 MUST 要求使用獨立匯出憑證開啟 Export Bundle，再驗證完整性、內容關聯與版本相容性；只有憑證正確且全部必要檢查通過後才可改變目前專案。
- **FR-028**: 系統 MUST 對匯入、讀取、驗證、保存、空間不足、權限、版本不相容與完整性錯誤提供穩定分類及可採取行動的安全訊息。
- **FR-029**: 系統 MUST 支援在不影響正式保存資料的情況下清除全部 Preview cache，並在清除後正確回報需重新運算的內容。
- **FR-030**: 系統 MUST 支援安全移除本機資料集或 Run；移除前必須列出會失效的關聯並取得明確確認，移除後不得留下可被解析為有效正式結果的孤立 reference。
- **FR-031**: 系統 MUST 在每次需要外部 Image Asset 前確認來源存在且內容 identity 與匯入時一致；來源遺失時標示 `source_missing`，內容改變時標示 `source_changed`，兩者皆不得靜默用於新的 Preview 或正式 Run。
- **FR-032**: 系統 MUST 讓 Dataset Version 在建立後不可修改；任何影響 fingerprint 的新增、移除、內容、patient、split 或 reference mask 變更都必須由目前版本衍生新版本，既有 Run 與 Validation Finding 必須繼續指向原版本。
- **FR-033**: 系統 MUST 對所有 Rosaray 管理的持久資料預設提供靜態加密，包括 project metadata、Dataset Versions、reference masks、Pipeline Snapshots、Run Records、Run Input Artifacts、正式 results 與 metrics；Rosaray 不負責加密僅以連結引用的外部來源檔案。
- **FR-034**: 系統 MUST 在無法取得或驗證解密憑證時安全拒絕讀寫受保護內容，且不得洩漏部分內容、建立未加密替代副本或破壞既有資料。
- **FR-035**: 系統 MUST 確保 Export Bundle 的獨立憑證不被嵌入 Bundle、manifest 或同一匯出位置；無法以正確憑證開啟時不得揭露任何研究內容。
- **FR-036**: 系統 MUST 讓 Presentation Layer 取得 Dataset Versions 與 Image Assets 的輕量清單，至少包含 identity、顯示名稱、patient ID、split、尺寸、來源可用性、reference mask 狀態與 validation 摘要，且列出清單時不需載入完整影像像素。
- **FR-037**: 系統 MUST 在前端選取 Image Asset 時，透過 Data Repository 提供對應的灰階影像內容與 metadata；回傳內容的 image identity 必須與目前選取項目一致。
- **FR-038**: 系統 MUST 在 reference mask 有效且與影像尺寸相容時提供可顯示的 mask reference；缺少或無效時必須提供不可用原因，且不得以空白 mask 冒充有效資料。
- **FR-039**: 系統 MUST 為前端影像讀取提供 `loading`、`ready`、`source_missing`、`source_changed`、`invalid_mask` 與 `failed` 等可區分狀態，並保證較舊的讀取結果不能覆蓋較新的使用者選取。
- **FR-040**: 系統 MUST 由本機 Rosaray service 作為瀏覽器前端與 Data Layer 之間的唯一資料存取邊界；前端不得直接開啟 project storage、解密持久資料或以本機檔案路徑繞過 Data Repository。
- **FR-041**: 系統 MUST 讓前端以 Dataset Version identity、Image Asset identity 與 Artifact Reference 請求清單、metadata、影像及 mask；一般顯示流程不得要求前端保存外部來源的絕對檔案路徑。
- **FR-042**: 系統 MUST 讓前端區分本機 service 的 `connecting`、`ready`、`unavailable` 與 `access_denied` 狀態，並在 service 中斷後停止接受新的資料結果，直到重新建立有效連線。
- **FR-043**: 系統 MUST 讓前端透過本機 Rosaray service 選擇單一 PNG／JPEG、多個 PNG／JPEG 或本機資料夾作為 Import Batch；資料夾掃描必須有界並避免重複走訪相同位置。
- **FR-044**: 系統 MUST 在寫入任何資料前提供 Import Batch Preview，列出候選影像、非支援項目、不可讀項目、重複內容、預計 metadata 狀態與項目計數；只有使用者明確確認後才可建立新的 Dataset Version，取消或掃描失敗不得留下部分版本。
- **FR-045**: 系統 MUST 允許 Import Batch 選配一份 metadata manifest，將候選影像明確對應至去識別化 patient ID、train／validation／test split 與 reference mask；manifest 的可交換格式與版本規則在 planning 階段定義。
- **FR-046**: 系統 MUST 在確認匯入前驗證 manifest 的檔案引用、唯一 mapping、split 值與 mask 相容性；未匹配影像可標示為 incomplete 後匯入，但系統不得依檔名或資料夾名稱自動推測研究 metadata。
- **FR-047**: 系統 MUST 讓 Explorer 依 Image Asset identity 個別請求可重建的 Thumbnail Artifact；開啟 Dataset／Image 清單不得自動載入所有完整影像或所有縮圖。
- **FR-048**: 系統 MUST 只為目前可見及鄰近清單項目安排縮圖讀取，並在項目離開相關範圍時允許取消尚未完成的請求；晚到縮圖不得套用到 identity 不同的清單項目。
- **FR-049**: 系統 MUST 使 Thumbnail Artifact 與來源 Image Asset 的內容 identity 綁定；來源進入 `source_changed` 或縮圖驗證失敗時，舊縮圖必須失效並以 placeholder 取代，直到新內容經確認後重建。
- **FR-050**: 本機 Rosaray service 內需要明確回應的 Command／Query（包含資料讀取、artifact 寫入與執行請求）MUST 使用 typed service／repository API；Event Bus MUST 僅承載非同步狀態、進度與 lifecycle 通知，不得作為 request-response 資料存取介面。
- **FR-051**: Data Engine MUST 擁有匯入、Dataset Version 建立、validation 與 export 等資料應用流程及其業務規則；Data Repository MUST 僅提供查詢、交易與持久化抽象。Execution Engine MAY 透過專用 artifact／Run repository API 寫入執行資料，但該 API MUST 強制執行 Data Layer 定義的完整性與生命週期規則。
- **FR-052**: Presentation Engine、Image Viewer 與各 Renderer MUST 在瀏覽器前端執行，並僅能透過本機 Rosaray service API 取得 Image Display Descriptor、影像、mask 與 artifact 內容；它們不得直接依賴 Data Repository、project storage 或本機檔案路徑。
- **FR-053**: Event Bus MUST 為非持久化的 service 內部通知機制，並保證同一 Run／Preview context 的事件依序交付；系統重啟、訂閱者重新連線或事件遺失後，呼叫者 MUST 由 Data Repository 查詢權威目前狀態，而不得依賴事件重播重建正式資料。

### Scope and Boundaries

**In scope**:

- 單機、local-first 的口內研究影像、reference mask、pipeline snapshot、Preview artifact、正式 Run、metrics、validation 與 Export Bundle 生命週期。
- Data Engine 的匯入與正規化責任、Data Repository 的一致存取語意、Memory Cache 的易失性語意，以及 Persistent Storage 的研究紀錄保存語意。
- 與 Execution Engine、Presentation Engine、Pipeline Definition 及 Event Bus 之間的資料責任邊界。
- Workstation GUI 中 Dataset／Image Explorer、中央影像顯示與 reference mask overlay 所需的 Data Layer 讀取行為及狀態契約；具體畫面繪製仍由 Presentation Layer 負責。
- 瀏覽器前端與本機 Rosaray service 之間的資料存取邊界，以及離線、連線中斷與重連時的使用者可見狀態。

**Out of scope for this feature**:

- DICOM、影片、3D volume、串流影像與未在現有 Rosaray 規格中定義的醫療格式。
- 雲端同步、遠端物件儲存、多人共同編輯、跨裝置衝突合併與院內系統整合。
- 將 Rosaray service 部署為遠端或多使用者服務；第一版 service 僅服務同一台工作站上的 Rosaray 前端。
- 影像標註工具、reference standard 審查流程與標註者一致性計算。
- 演算法節點的實際運算、GPU／CPU 排程、畫面渲染與 Event Bus 傳遞機制。
- 臨床診斷、病歷管理、法規認證與直接識別病患的資料管理。

### Key Entities *(include if feature involves data)*

- **Project**: 一個本機研究工作空間，聚合 datasets、pipelines、Run Records 與匯出設定，並界定資料生命週期。
- **Dataset**: 研究影像集合的長期身份與 Dataset Versions 容器；顯示資訊可更新，但不直接承載會影響研究結果的可變內容。
- **Dataset Version**: Dataset 在特定時間的不可變研究快照，由資料項目、patient／split 對應、reference mask 關係、validation 結果與唯一 fingerprint 構成；可由前一版本衍生，正式 Run 必須指向確切版本。
- **Image Asset**: 以外部檔案連結匯入的單張口內影像，具有穩定 identity、外部來源位置、匯入時內容 identity、可重建的灰階研究表示、尺寸及 `available`／`source_missing`／`source_changed` 狀態；專案不擁有原始影像副本。
- **Research Subject**: 由去識別化 patient ID 表示的研究對象，可關聯多張影像，但在同一資料集內不得跨越互斥 split。
- **Split Assignment**: Image Asset／Research Subject 在 train、validation 或 test 中的研究分組，參與 dataset fingerprint 與 leakage validation。
- **Reference Mask**: 與特定 Image Asset 配對的標準答案資料，具有自身內容 identity、shape 與配對有效性。
- **Dataset Fingerprint**: 對會影響研究解讀的資料集內容與關係所建立的穩定身份，用於辨識資料集版本。
- **Pipeline Snapshot**: 正式 Run 使用的不可變 pipeline 內容，包含 graph identity、節點版本與可重現參數，但不包含純 UI 狀態。
- **Run Input Artifact**: 正式 Run 實際送入 pipeline 的不可變灰階影像內容；獨立於外部原始檔生命週期，並由 Run Record 保存其內容 identity 與來源 Image Asset 關聯。
- **Run Record**: 一次正式實驗的權威紀錄，連結輸入資料、pipeline、seed、policy、狀態、時間、metrics、artifacts 與錯誤摘要。
- **Artifact**: 影像、mask、measurement 或其他節點輸出內容，可為 Preview-only 或正式持久資料，並保存內容 identity 與 lineage。
- **Artifact Reference**: 上層用來定位 Artifact 的輕量、不可混淆識別，攜帶足以驗證型別、內容與生命週期的 metadata，而非像素本身。
- **Image Display Descriptor**: Explorer 與中央檢視器使用的輕量讀取描述，關聯 Dataset Version、Image Asset、顯示 metadata、來源狀態、validation 摘要，以及可用的 image／reference mask artifact references；本身不包含完整像素資料。
- **Thumbnail Artifact**: 從特定 Image Asset 內容 identity 衍生、供 Explorer 辨識影像使用的低解析度顯示 artifact；可被淘汰與重建，不可用於正式 Run、measurement 或研究判讀。
- **Local Service Session**: 瀏覽器前端與同一工作站 Rosaray service 之間的存取情境，具有 `connecting`、`ready`、`unavailable` 或 `access_denied` 狀態；所有前端資料讀取都必須隸屬於有效 session。
- **Import Batch**: 一次由單檔、多檔或資料夾掃描形成的候選匯入集合，保存候選項目、略過原因、重複判斷、錯誤與確認狀態；確認前不屬於任何 immutable Dataset Version。
- **Metadata Manifest**: Import Batch 可選的明確 mapping 資料，將候選影像連結到去識別化 patient ID、split 與 reference mask；其自身具有版本，且任何無法唯一解析的 mapping 都不得套用。
- **Metric Set**: 與特定 Run 及其輸出綁定的量化結果，例如 Dice、面積、前景像素、連通元件數與各步驟時間。
- **Validation Finding**: 對資料缺漏、split leakage、mask 配對、重複內容或完整性問題的可追蹤結果，包含嚴重程度與受影響項目。
- **Export Bundle**: 使用者明確選取、自足且完整加密的可攜式研究封存，包含 manifest、版本、正式資料、關聯與完整性證明，不含 Preview cache；必須使用未包含於 Bundle 內的獨立憑證才能開啟。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 在包含 1,000 張、每張最高 50 megapixels 影像與 500 筆 Run 的代表性本機研究專案中，95% 的已快取影像或結果選取可在 250 ms 內呈現可用內容，99% 可在 1 秒內呈現或顯示明確載入狀態。
- **SC-002**: 對已知包含 patient 跨 split、缺少 patient ID、mask 缺失、mask 尺寸不符與重複內容的驗證資料集，100% 的植入問題都被正確分類，且不產生不合法的 reference-based metric。
- **SC-003**: 在 1,000 次包含快速參數變更、取消與亂序完成的 Preview 壓力情境中，0 次過期 artifact 被顯示為目前 revision 的結果。
- **SC-004**: 100% 成功的正式 Run 都能由任一最終結果回溯至唯一的 dataset fingerprint、image、pipeline snapshot、seed、節點版本與直接輸入 artifacts。
- **SC-005**: 在保存過程中模擬中斷的測試裡，0 筆缺少必要資料的 Run 被標示為成功，且先前完整資料的可讀率為 100%。
- **SC-006**: 對 100 組相同輸入、pipeline、節點版本與 seed 的重複正式 Run，可重現節點的內容 identity 一致率為 100%；若不一致，系統必須明確將其分類為不可重現。
- **SC-007**: 完整 Export Bundle 的匯出／匯入往返測試中，dataset fingerprint、pipeline identity、Run identity、metrics 與 artifact 內容 identity 的一致率為 100%；任何單一內容被竄改、憑證錯誤或憑證缺失時，匯入拒絕率為 100%。
- **SC-008**: 在完全離線環境中，研究者可完成匯入、驗證、Preview、正式 Run、重新開啟專案與匯出等全部核心流程，且資料對外傳輸事件為 0。
- **SC-009**: 在可用性測試中，至少 90% 的目標研究者可在沒有開發者協助下完成「匯入資料、修正一個 validation 問題、執行並找到一筆正式 Run」任務，且能正確區分 Preview 與正式結果。
- **SC-010**: 清除 Preview cache 或重新啟動工作站後，100% 的 Run Input Artifacts、reference masks、成功 Run、metrics 與最終 artifacts 仍可讀取，Preview miss 則皆被正確要求重建。
- **SC-011**: 在移動、重新命名、刪除或修改外部來源檔案的測試中，100% 受影響的 Image Assets 都被分類為 `source_missing` 或 `source_changed`，且 0 次以失效來源啟動新的 Preview 或正式 Run。
- **SC-012**: 在 100 次會影響 fingerprint 的資料集變更測試中，100% 建立新的 Dataset Version，0 個既有版本或既有 Run 關聯被覆寫；純顯示資訊變更建立新版本的次數為 0。
- **SC-013**: 對 Rosaray 管理的持久資料進行 100 次未經授權的直接檔案讀取測試時，可被還原為影像、mask、pipeline、metric 或可識別研究 metadata 的內容為 0；合法使用情境的成功解密率為 100%。
- **SC-014**: 對 100 個 Export Bundles 進行未提供憑證的內容檢查時，可辨識的 manifest、影像、mask、pipeline、metric 或研究 metadata 為 0，且 0 個 Bundle 內含可解密自身的憑證。
- **SC-015**: 在包含 1,000 張影像的代表性專案中，Explorer 在 1 秒內顯示 Dataset／Image 清單或明確 loading 狀態；選取影像後，95% 可在 1 秒內顯示正確灰階影像與 metadata，100% 在 1 秒內顯示內容或可採取行動的錯誤狀態。
- **SC-016**: 在 500 次快速切換影像的測試中，0 次將舊選取的影像、metadata 或 reference mask 顯示為目前項目；對所有有效 masks 的 overlay 配對正確率為 100%，缺少或無效 mask 被錯誤啟用的次數為 0。
- **SC-017**: 在本機 Rosaray service 正常運作時，前端於 2 秒內進入 `ready` 或顯示可採取行動的連線錯誤；在 100 次 service 中斷測試中，0 次將中斷後抵達的資料顯示為目前內容，且重新連線後 100% 可恢復清單與目前選取狀態或明確要求重新選取。
- **SC-018**: 對包含 1,000 張支援影像、100 個非支援檔案、20 個重複內容與 10 個不可讀項目的資料夾，Import Batch Preview 必須分類 100% 項目並顯示正確計數；確認前建立的 Image Assets 與 Dataset Versions 數量皆為 0。
- **SC-019**: 在取消匯入、掃描失敗或確認期間中斷的 100 次測試中，0 次產生部分可見的 Dataset Version；成功確認後，已選候選項目的建立率為 100%，未選與略過項目的建立率為 0%。
- **SC-020**: 對包含正確 mapping、未知影像、缺少檔案、重複 mapping、非法 split 與尺寸不符 mask 的 manifest 測試集，100% 問題都在確認前被分類；未匹配項目標示 incomplete 的正確率為 100%，從檔名或資料夾錯誤推測 metadata 的次數為 0。
- **SC-021**: 在含 1,000 張影像的 Explorer 測試中，95% 可見縮圖在進入可見範圍後 1 秒內顯示，100% 在 1 秒內顯示縮圖或明確 placeholder；未進入可見或鄰近範圍的項目觸發完整影像讀取的次數為 0，錯置到其他 Image Asset 的縮圖次數為 0。

## Assumptions

- Rosaray 第一版仍是研究與展示工作站，不用於臨床診斷或取代醫師判讀。
- 工作站為單一使用者、單一行程的本機應用；同一專案不會被多個獨立工作站同時修改。
- Workstation GUI、Presentation Engine 與各 Renderer 位於瀏覽器前端，並透過同一台工作站上的 Rosaray service 存取系統能力；Core、Algorithm、Execution 與 Data 層均由該 service 承載，Event Bus 僅限 service 內部通訊。前端與 service 間使用明確的本機 API，不把 in-process Event Bus 暴露為跨行程協定；核心資料瀏覽不依賴網際網路或遠端服務。
- 第一版輸入限 PNG／JPEG 單張影像；專案僅保留外部來源連結與匯入時內容 identity，原始檔生命週期由研究者管理，灰階研究表示可按需重建。
- Patient ID 是研究用去識別化識別碼，不保存姓名、病歷號、生日或其他 direct identifiers。
- 外部來源影像的磁碟加密、檔案權限與備份由研究者及作業系統負責；Rosaray 的加密責任從其自行持久保存的 project metadata 與 artifacts 開始。
- Train、validation 與 test 為互斥 split，且以 Research Subject 為單位檢查跨 split leakage。
- Dataset 是可持續演進的集合身份；Dataset Version 是不可變研究快照，所有正式 Run、validation 結果與 fingerprint 都以版本為關聯單位。
- 缺少 reference mask 的影像仍可用於不依賴 reference standard 的 Preview 或 Run，但不得計算 Dice 等 reference-based metrics。
- Preview 與正式 Run 使用相同節點語意；兩者差異只在執行範圍、生命週期與保存政策。
- 新的 Preview 與正式 Run 預設從可用的外部 Image Asset 建立輸入；既有正式 Run 可直接使用其已保存的 Run Input Artifact 重現，不依賴外部來源仍然存在。
- 代表性效能驗收工作量為單一專案最多 1,000 張、單張最高 50 megapixels 的影像與 500 筆正式 Run；更大規模留待容量測試後另立規格。
- Pipeline graph identity、artifact content identity 與 dataset fingerprint 的 canonical 定義由後續 planning 階段具體化，但必須遵守本規格描述的內容包含與排除原則。
- 專案資料保留期限由研究者與其機構政策決定；Rosaray 不自動上傳、到期刪除或代替 IRB／資料治理流程。
- Export Bundle 的獨立憑證交付、保管與恢復由研究者及其機構流程負責；Rosaray 不提供繞過遺失憑證的復原方式。
- 依賴既有 [System Overview](../../architecture/overview.md)、[Data Layer](../../architecture/data-layer.md)、[Data Engine](../../architecture/data-engine.md)、[Data Repository](../../architecture/data-repository.md)、[Memory Cache](../../architecture/memory-cache.md)、[Persistent Storage](../../architecture/persistent-storage.md)、[Execution Engine](../../architecture/execution-engine.md)、[Pipeline Definition](../../architecture/pipeline-definition.md) 與 [Presentation Engine](../../architecture/presentation-engine.md) 所定義的跨層角色。
