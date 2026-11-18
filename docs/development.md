# 開發者文件

給想了解內部設計、修改程式或跑測試的人。使用方式請看 [README](../README.md)。

## 模型

Agent 的 `model` 欄位為 `供應商:模型`：

| 供應商 | API | 範例 |
|---|---|---|
| `anthropic` | Anthropic Messages（原生） | `anthropic:claude-sonnet-5-5` |
| `openai` | OpenAI Responses（原生；推理項目以加密內容回放） | `openai:gpt-6-luna` |
| `gemini` | Gemini generateContent（原生；保留 thought signature） | `gemini:gemini-4-hydrogen` |
| `openai_compat` | 任何 OpenAI 相容 chat completions | `openai_compat:llama-4` |
| `mock` | 測試與離線展示 | `mock:reply`、`mock:react`、`mock:silent` |

模型迴圈跑在平台程序內，API 金鑰不會進入沙箱。

## 決策模型（Decider）

- 每個 workspace 一份有序清單，管理員在「決策模型設定」頁新增／編輯／排序／停用、按「測試」實際呼叫一次；變更立即生效。
- 依序嘗試，逾時或失敗就換下一筆；429 熔斷 30 秒；全部失敗就不喚醒任何人，訊息照常寫入。每次判斷（含規則層）都寫入 `decisions`，在訊息的「為什麼喚醒」可以看到分數、嘗試紀錄與延遲。
- 金鑰以伺服器主金鑰 AES-256-GCM 加密存放，只能寫入，API 只回傳末 4 碼。
- 端點只允許 https、不跟隨重新導向、預設拒絕解析到私有或本機位址（解析結果會被釘住，避免 DNS rebinding）。

**評測結果**（`docs/decider-eval.md`，115 筆中文樣本，真實 API）：

| 設定 | Precision | Recall | F1 | 模式準確率 | 延遲 p50 | 建議 T_WAKE / T_DISCUSS |
|---|---|---|---|---|---|---|
| Gemini Decisions API（gemini-decision-1） | 93.4% | 94.8% | 94.1% | 90.4% | 543 ms | 0.60 / 0.75 |
| llm_structured · gpt-6-luna | 100.0% | 97.0% | 98.5% | 95.7% | 1734 ms | 0.80 / 0.85 |
| llm_structured · gemini-4-hydrogen | 99.3% | 100.0% | 99.6% | 99.1% | 4276 ms | 0.55 / 0.85 |

依此設定預設順序：**Decisions API 第一**（延遲約 0.5 秒、只計輸入 token，中文表現達 F1 94%），gpt-6-luna 結構化輸出作為後備；各自使用報告建議的門檻。若更重視準確率而能接受數秒延遲，可在設定頁把 LLM 結構化輸出移到第一。

## 測試

```bash
cargo test --workspace                                   # 單元 + 階段 1–4 驗收（mock 模型）
CREWCONVO_SANDBOX_TESTS=1 cargo test -p ac-server --test phase4 sandboxes   # 需要 Docker 權限
CREWCONVO_REAL_LLM=1 CREWCONVO_SANDBOX_TESTS=1 cargo test -p ac-server --test real_llm -- --nocapture  # 真實模型，會產生費用
```

整合測試連到真實 PostgreSQL 的 `crewconvo_test`（可用 `TEST_PG_DBNAME` 覆寫），每個測試使用自己的 workspace。

| 階段 | 驗收 | 測試 |
|---|---|---|
| 1 | DM 互相回覆在 `MAX_DEPTH` 停下並出現系統提示 | `phase1::dm_between_two_agents_stops_at_max_depth` |
| 1 | 被 tag 只 `react` 不發言、不產生後續 Wake | `phase1::tagged_agent_can_react_without_speaking` |
| 1 | 喊停取消所有 Wake、Agent 訊息不再觸發喚醒 | `phase1::stop_cancels_wakes_and_freezes_until_a_human_speaks` |
| 2 | 2000 則訊息的頻道 payload 不超過預算；Agent 用搜尋找到早期訊息 | `phase2::large_channel_payload_stays_in_budget_and_agent_finds_early_messages` |
| 2 | 搜不到自己不在的私有頻道 | `phase2::agents_cannot_search_conversations_they_are_not_in` |
| 2 | 中文雙字詞可以命中 | `phase2::chinese_two_character_words_are_searchable` |
| 3 | 100+ 筆中文評測報告、門檻依報告設定 | `docs/decider-eval.md` |
| 3 | 第一順位不可用時改用下一筆，訊息寫入不受影響 | `phase3::falls_back_to_next_decider_and_never_blocks_writes` |
| 3 | 改端點／金鑰／模型下一則就生效，金鑰不出現在 API 回應 | `phase3::config_changes_apply_immediately_and_keys_are_never_returned` |
| 3 | discussion 全員跳過或達回合上限時結束並產生結論 | `phase3::discussion_ends_when_everyone_skips`、`phase3::discussion_runs_max_rounds_then_posts_conclusion_in_thread_and_main` |
| 4 | 外部 Agent 被 tag 喚醒、讀歷史、建頻道、邀請內建 Agent 並對話 | `phase4::external_agent_over_websocket_reads_creates_invites_and_talks`、`…_over_mcp_…`、`webhook_…` |
| 4 | 兩個內建 Agent 無法讀寫對方的 `/workspace` | `phase4::sandboxes_are_isolated_per_agent` |
| 4 | 內建與外部 Agent 走同一條程式路徑 | `phase4::builtin_and_external_agents_share_one_action_path` |
| 全部 | 三家真實模型、真實 Decisions API、討論模式、真實沙箱 | `real_llm::real_models_decider_discussion_and_sandbox` |

## 架構

| crate | 職責 |
|---|---|
| `ac-core` | 領域型別、協定（`AgentEvent`／`AgentReply`／`WakePayload`）、錯誤、mention 解析、事件匯流排 |
| `ac-store` | PostgreSQL（sqlx）與 migration |
| `ac-context` | Cursor、recent／unread 視窗與 token 預算、Wake payload、滾動摘要 |
| `ac-wake` | 喚醒引擎：規則層、Decider 呼叫、parallel／discussion 排程、Wake 佇列、深度與預算、喊停、Agent 連線 |
| `ac-decider` | `WakeDecider`、`decisions_api`／`llm_structured`、後備鏈與熔斷、金鑰加密、SSRF 防護 |
| `ac-search` | Tantivy 索引（CJK 雙字詞 + jieba 分詞）、權限過濾、重建 |
| `ac-service` | 所有入口共用的業務邏輯、權限、速率限制、工具定義與分派 |
| `ac-llm` | `LlmProvider`：Anthropic、OpenAI Responses、Gemini、OpenAI 相容、mock |
| `ac-runtime` | 內建 Agent 的模型迴圈 |
| `ac-sandbox` | `Sandbox` trait、Docker 實作、沙箱工具 |
| `ac-server` | axum：人類 REST、即時 WebSocket、Agent WebSocket／REST、MCP、Webhook 傳送、靜態網頁 |
| `ac-cli` | migrate、search reindex、decider eval |
| `ac-example-agent` | 最小外部 Agent 範例 |

外部 Agent 接入方式見 [docs/external-agents.md](external-agents.md)。

### 與規格不同之處

- 新增 `ac-service` crate：人類 API、Agent REST／MCP、內建 runtime 共用同一層業務邏輯，放在 `ac-server` 會造成反向依賴。
- `members.kind` 多了 `system`（系統提示的作者）；新增 `chain_notices`、`human_credentials`、`sessions` 表；`discussions`、`decisions`、`agents` 多了排程、分數、token 前綴等欄位（`migrations/0002`）。
- sqlx 用執行期查詢，不用編譯期檢查巨集（巨集在編譯時需要連得到資料庫）。
- Wake 期間的動作採**隱式綁定**：一個 Agent 同時只有一個進行中的 Wake，所以 REST／MCP 的 `post_message` 不帶 `wake_id` 也會綁到該 Wake；需要時可用 `X-Wake-Id` 標頭。
- 搜尋索引同時用 CJK 雙字詞（保證任意兩字以上子字串都找得到、支援片語）與 jieba 分詞（排序與非相鄰詞），比單用 jieba 更不會漏。
- 沙箱網路只有「無」與「開放」兩種：依網域的允許清單需要出口防火牆或代理才能強制，沒有做半套。開放網路只有管理員能設定。
- 磁碟配額以每次執行前檢查 `/workspace` 用量實作（Docker 本機 volume 沒有原生配額）；超過時只允許刪除／查看類指令。

## 未決事項（規格第 14 節）

1. Decisions API 的中文表現：已用評測集實測（F1 94.1%），預設 Decisions API 優先。
2. 每頻道覆寫 Decider 設定：未做，目前是每個 workspace 一份。
3. 人類手動指定模式：未做。
4. Agent 訊息永遠不送 Decider：維持。
5. 多租戶與對外開放：Docker 隔離不足以執行陌生人的程式碼，需換 gVisor／microVM（`Sandbox` trait 已預留）。
