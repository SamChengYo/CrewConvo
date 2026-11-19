# 設定參考

CrewConvo 的設定全部來自環境變數。最簡單的做法是在伺服器的工作目錄放一個 `.env` 檔（範本：[`.env.example`](../.env.example)），啟動時會自動讀取。用 Docker 時，compose 檔會把 `.env` 傳進容器。

改完設定要重新啟動伺服器才會生效。

## 最少要設定什麼

| 版本 | 必填 | 建議 |
|---|---|---|
| Personal | 沒有。什麼都不設定就能啟動 | 要用內建 Agent 才需要模型的 API key |
| Enterprise | `CREWCONVO_EDITION=enterprise`、資料庫連線 | `PUBLIC_BASE_URL`、`COOKIE_SECURE=true`、`CREWCONVO_MASTER_KEY` |

## 版本

| 變數 | 預設 | 說明 |
|---|---|---|
| `CREWCONVO_EDITION` | `personal` | `personal` 或 `enterprise`。差異見下表 |
| `REGISTRATION` | `open` | 只在 Enterprise 有效。`open`：任何人都能自己註冊；`closed`：只有第一個帳號。Personal 永遠只有一個帳號 |

| | Personal | Enterprise |
|---|---|---|
| 帳號 | 一個（第一次開啟網頁時建立） | 多個；第一個是管理員 |
| 資料庫 | 內建，自動管理；也可以指定外部 PostgreSQL | 必須指定外部 PostgreSQL |
| 預設監聽位址 | `127.0.0.1:8090`（只有本機） | `0.0.0.0:8090` |
| 加密金鑰 | 自動產生並保存 | 自動產生，或用 `CREWCONVO_MASTER_KEY` 指定 |

## 資料庫

| 變數 | 說明 |
|---|---|
| `DATABASE_URL` | 完整連線字串，例如 `postgres://user:pass@host:5432/crewconvo?sslmode=require` |
| `PG_HOST`、`PG_PORT`、`PG_DBNAME`、`PG_USER`、`PG_PASSWORD`、`PG_SSL` | 分開寫的版本（密碼有特殊字元時比較方便）。`PG_PORT` 預設 5432，`PG_SSL=true` 會強制加密連線 |
| `DB_MAX_CONNECTIONS` | 連線池上限，預設 20 |
| `EMBEDDED_DB_PORT` | 內建資料庫固定使用的連接埠；預設每次啟動自動挑一個 |

- 設定了 `DATABASE_URL` 或 `PG_HOST` 就會使用外部資料庫，否則 Personal 版使用內建資料庫。
- **內建資料庫**是一個由 CrewConvo 自己下載並管理的 PostgreSQL：第一次啟動時下載（需要網路，解開後約 120 MB），之後資料放在 `DATA_DIR/postgres`。伺服器關閉時它會一起關閉。備份時，先停止伺服器再複製整個 `DATA_DIR` 即可。
- **Enterprise 可用的資料庫**：PostgreSQL 與相容的服務，例如自架 PostgreSQL、AWS RDS / Aurora PostgreSQL、Google Cloud SQL、Azure Database for PostgreSQL、Supabase、Neon。實際測試過的是 PostgreSQL 17（AWS RDS 與內建資料庫）。**不支援** MySQL、SQL Server、SQLite。
- 資料表會在啟動時自動建立與升級，不需要手動執行任何 SQL。資料庫帳號需要建立資料表的權限。

## 網路

| 變數 | 預設 | 說明 |
|---|---|---|
| `BIND_ADDR` | 依版本 | 監聽位址與連接埠。Personal 想讓區域網路的其他電腦連進來，改成 `0.0.0.0:8090` |
| `PUBLIC_BASE_URL` | 從連線推算 | 其他電腦連到這台伺服器用的網址，例如 `https://crewconvo.example.com`。`crewconvo setup` 顯示的核准連結用的就是它 |
| `COOKIE_SECURE` | `false` | 網站走 HTTPS 時設為 `true` |

放在反向代理後面時，請讓代理帶上 `X-Forwarded-Proto` 與 `Host`，並關閉回應緩衝（範例：[`deploy/nginx.conf`](../deploy/nginx.conf)）。

## 資料與金鑰

| 變數 | 預設 | 說明 |
|---|---|---|
| `DATA_DIR` | `data` | 內建資料庫、搜尋索引、自動產生的金鑰都在這裡 |
| `CREWCONVO_MASTER_KEY` | 自動產生 | 加密儲存的機密（決策模型的 API key、平台連線的 token）。32 個隨機位元組的 base64。留白時自動產生並存到 `DATA_DIR/master.key`。**弄丟這把金鑰，已儲存的機密就解不開** |
| `SEARCH_INDEX_DIR` | `DATA_DIR/search-index` | 全文搜尋索引。刪掉後會在下次啟動時從資料庫重建 |
| `WEB_DIST` | 自動 | 網頁檔案的位置 |
| `LOCAL_AGENTS` | 依情況 | 是否在網頁提供「這台電腦的 Coding Agent」：由伺服器直接偵測、安裝並喚醒它所在電腦上的 Coding Agent。預設只在 Personal 版且 `BIND_ADDR` 是本機位址時開啟。連線資訊與每個 Agent 的權限存在 `DATA_DIR/local-agents.json`；對話綁定的專案資料夾在 `DATA_DIR/local-projects.json`，尚未合併的工作成果（git worktree）在 `DATA_DIR/local-work.json` |
| `LOCAL_SESSIONS` | `reuse` | 這台電腦的 Coding Agent 被叫起來時要不要接續先前的工作階段（session）。`reuse`：CrewConvo 記下它開過的工作階段和各自的用途（`DATA_DIR/local-sessions.json`），每次喚醒挑一個合適的接續——同一個討論串接續同一個；其他情況由決策模型依用途挑選，決策模型無法回答時接續同一個頻道最近用過的；沒有合適的就開新的。超過 7 天沒用或已用 30 次的不再接續，30 天沒動過的紀錄會刪除；工作階段在 Coding Agent 那邊被刪掉時，紀錄會跟著刪除並改開新的。`new`：每次都開新的，不記錄。只有 Claude Code 和 Codex 支援接續，其他一律開新的 |
| `REQUIRE_DECIDER` | `true` | 工作區沒有可用的決策模型（至少一筆啟用、而且需要金鑰的協定已填金鑰）時，網頁只顯示決策模型的設定畫面，人類發訊息會收到 409「請先設定決策模型」。`false` 取消這項要求 |

Windows 桌面應用程式（`CrewConvo.exe`）把 `DATA_DIR` 預設成 `%LOCALAPPDATA%\CrewConvo\data`，並讀取 `%LOCALAPPDATA%\CrewConvo\.env`；記錄寫在同一個資料夾的 `logs\crewconvo.log`。

## 模型（內建 Agent 用，選填）

外部接入的 Agent（Claude Code、Codex 等）用它們自己的模型，不需要這一段。

| 變數 | 說明 |
|---|---|
| `ANTHROPIC_API_KEY`、`OPENAI_API_KEY`、`GEMINI_API_KEY` | 有設定的供應商才能用。建立內建 Agent 時模型寫成 `anthropic:…`、`openai:…`、`gemini:…` |
| `OPENAI_COMPAT_BASE_URL`、`OPENAI_COMPAT_API_KEY` | 任何 OpenAI 相容的端點；模型寫成 `openai_compat:<model>` |
| `SUMMARY_MODEL` | 長對話的滾動摘要要用的模型（建議便宜的）。留白就不做摘要 |
| `DECIDER_DEFAULT_T_WAKE`、`DECIDER_DEFAULT_T_DISCUSS` | 新工作區的決策門檻，預設 0.60 與 0.75 |

決策模型（決定「沒被 @ 的訊息該叫醒誰」）在網頁的「決策模型設定」管理；第一次建立工作區時，如果有 `GEMINI_API_KEY` 或 `OPENAI_API_KEY` 會自動帶入預設設定。

## 平台連線（A2A）

| 變數 | 說明 |
|---|---|
| `A2A_CONNECTIONS` | JSON 陣列，每個平台一筆 `{"url", "token", "name"}` |

```ini
A2A_CONNECTIONS=[{"url":"https://platform.example.com/crewconvo/catalog","token":"…","name":"Agatha"}]
```

- `url`：平台的目錄網址，或單一 A2A Agent Card 的網址。`token`、`name` 可省略。
- **有設定這個變數時，它就是唯一的來源**：啟動時，清單裡的連線會自動建立並同步，不在清單裡的會被移除（連同它的 Agent 與群組）。之後每 10 分鐘重新同步一次。
- 完全不設定時，不會動現有的連線。
- 連線屬於第一個管理員帳號；還沒有任何帳號時會等到帳號建立後才套用。
- 平台那邊要做什麼：[a2a-provider.md](a2a-provider.md)。

## 對外連線的限制

CrewConvo 會主動連到你設定的網址：平台連線、決策模型端點、webhook。

| 變數 | 預設 | 說明 |
|---|---|---|
| `EGRESS_ALLOW_PRIVATE` | `false` | 允許連到內網或本機位址（例如同一台機器上的平台或自架模型） |
| `EGRESS_ALLOW_HTTP` | `false` | 允許沒有加密的 `http://` |

預設兩者都關閉，避免有人把連線指向內部服務。只在你信任所有能設定這些網址的人時才打開。

## 沙箱（選填）

讓內建 Agent 在隔離的 Docker 容器裡執行程式。伺服器連不到 Docker 時會自動停用，不影響其他功能。

| 變數 | 預設 | 說明 |
|---|---|---|
| `SANDBOX_ENABLED` | `true` | 關閉請設 `false` |
| `SANDBOX_IMAGE` | `python:3.12-slim` | 容器映像檔 |
| `SANDBOX_IDLE_STOP_SECS` | `900` | 閒置多久自動停止容器 |
| `SANDBOX_ALLOW_NETWORK` | `true` | 設為 `false` 時，任何 Agent 的容器都不能連網 |

## 喚醒的限制

用來避免 Agent 互相喚醒而失控。一般不需要改。

| 變數 | 預設 | 說明 |
|---|---|---|
| `CC_MAX_DEPTH` | 6 | Agent 互相喚醒的最大連鎖深度 |
| `CC_CHAIN_MAX_WAKES` | 25 | 一條對話鏈最多自動喚醒幾次 |
| `CC_CHAIN_MAX_TOKENS` | 0 | 一條對話鏈的 token 上限，0 表示不限 |
| `CC_DEFAULT_WAKE_SECONDS` | 300 | 單次喚醒的預設時限 |
| `CC_MAX_WAKE_TOTAL_SECONDS` | 1800 | 單次喚醒含延長的總時限 |
| `CC_PROACTIVE_POSTS_PER_DAY` | 50 | 每個 Agent 每天可以主動發言幾次 |
| `CC_STICKY_WINDOW_SECONDS` | 300 | 人回覆 Agent 之後，幾秒內的下一句仍算在跟它說話 |
| `CC_DISCUSSION_MAX_ROUNDS` | 3 | 多個 Agent 討論的最多輪數 |

## 記錄

| 變數 | 預設 | 說明 |
|---|---|---|
| `RUST_LOG` | `info,sqlx=warn,tower_http=info` | 記錄的詳細程度，例如 `debug` |

## `crewconvo` 指令的設定

`crewconvo`（裝在要接入的電腦上）不讀 `.env`。它的設定在 `~/.crewconvo/config.json`，由 `crewconvo setup` 寫入。可用的環境變數：

| 變數 | 說明 |
|---|---|
| `CREWCONVO_HOME` | 設定與記錄的位置，預設 `~/.crewconvo` |
| `CREWCONVO_NO_LINGER` | 設為 `1` 時，Linux 上不會自動開啟 linger |
| `CURSOR_API_KEY`、`ANTHROPIC_API_KEY`、`OPENAI_API_KEY` | 如果你的 coding agent 靠這些環境變數登入，安裝背景服務時會把它們帶進服務的環境 |
