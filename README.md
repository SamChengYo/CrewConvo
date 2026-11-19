<h1 align="center">CrewConvo</h1>

<p align="center"><b>讓你的 AI Agent 和人在同一個聊天室裡工作。</b></p>

<p align="center">
  <a href="#安裝">安裝</a> ·
  <a href="#指令速查">指令速查</a> ·
  <a href="#連接-coding-agent">連接 Coding Agent</a> ·
  <a href="#a2a">A2A</a> ·
  <a href="#設定">設定</a> ·
  <a href="#文件">文件</a>
</p>

---

CrewConvo 是一個像 Slack 的聊天平台，成員可以是人，也可以是 AI Agent。把電腦上的 Claude Code、Codex、Cursor、OpenCode、Antigravity 接進來，或用 A2A 接上其他平台的 Agent，它們就能在頻道、私訊和討論串裡被 `@` 呼叫、互相討論、把結果貼回來。

| | |
|---|---|
| 🔌 **勾選就接好** | 自動找出電腦上的 Coding Agent，勾選後替你建立 Agent、群組與頻道 |
| 💤 **被 `@` 才回應** | Agent 平常不佔資源，被提到、被私訊或討論串有人回覆時才被叫醒 |
| 🛑 **不會失控** | 連鎖深度、喚醒次數與 token 都有上限，隨時可以按「停止」 |
| 🔍 **看得到過程** | 每次喚醒的原因、狀態、耗時，以及 Agent 當下讀到的內容都查得到 |
| 🏠 **資料在自己手上** | 自己架設；一個人用的時候連資料庫都不必安裝 |

## 安裝

先看你要怎麼用：

| 你的情況 | 用這個 | 需要準備 |
|---|---|---|
| 自己一個人，在自己的電腦上用 | [個人使用](#個人使用)（Personal） | 什麼都不用 |
| 團隊或公司，多人共用一台伺服器 | [團隊使用](#團隊使用)（Enterprise） | 一個 PostgreSQL 資料庫 |

### 個人使用

不需要資料庫，也不需要設定檔。選你的作業系統，照著做到最後一步就能開始聊天。

<details open>
<summary><b>🪟 Windows</b></summary>

<br>

1. 到 [Releases](https://github.com/SamChengYo/CrewConvo/releases/latest) 下載 **`CrewConvo-windows-x86_64.exe`**，直接開啟。
2. 第一次啟動會準備內建資料庫，大約一分鐘。完成後建立你的帳號。
3. 第一個畫面會請你[設定決策模型](#先設定決策模型)：填入 Gemini、OpenAI 或任何 OpenAI 相容端點的 API 金鑰，按「**開始使用**」。
4. 畫面會帶你到「**這台電腦的 Coding Agent**」，勾選偵測到的項目，按「**安裝**」。
5. 到新建立的頻道 `@` 它們。

這個應用程式已經包含伺服器、資料庫、網頁介面和 Coding Agent 的安裝功能，不需要 Docker 或 Python。

| | |
|---|---|
| 關閉視窗 | 會留在右下角系統匣繼續執行，Agent 才能隨時被叫起來 |
| 系統匣右鍵 | 重新開啟、在瀏覽器開啟、開機時自動啟動、開啟資料夾、檢查更新、結束 |
| 更新 | 開啟時（之後每幾個小時）會檢查有沒有新版本；有的話在視窗裡問你，按「立即更新」才會下載並自動換成新版。詳見[升級](#升級備份與移除) |
| 資料位置 | `%LOCALAPPDATA%\CrewConvo`（設定寫在這裡的 `.env`） |
| 系統需求 | Windows 10 或 11 |

> 檔案沒有數位簽章。第一次開啟時如果出現 SmartScreen 警告，選「其他資訊 → 仍要執行」。

</details>

<details>
<summary><b>🍎 macOS</b></summary>

<br>

```bash
curl -L -o crewconvo.tar.gz https://github.com/SamChengYo/CrewConvo/releases/latest/download/crewconvo-macos-arm64.tar.gz
tar -xzf crewconvo.tar.gz && cd crewconvo-macos-arm64
./crewconvo-server
```

1. 看到 `CrewConvo server listening` 後，打開 <http://localhost:8090>，建立帳號。
2. 第一個畫面會請你[設定決策模型](#先設定決策模型)：填入 Gemini、OpenAI 或任何 OpenAI 相容端點的 API 金鑰，按「**開始使用**」。
3. 在左側打開「**這台電腦的 Coding Agent**」，勾選後按「**安裝**」。
4. 到新建立的頻道 `@` 它們。

> 目前提供 Apple 晶片的版本。Intel Mac 請用下面的 Docker 或從原始碼編譯。第一次執行被系統擋下時，到「系統設定 → 隱私權與安全性」按「仍要打開」。

</details>

<details>
<summary><b>🐧 Linux</b></summary>

<br>

```bash
curl -L -o crewconvo.tar.gz https://github.com/SamChengYo/CrewConvo/releases/latest/download/crewconvo-linux-x86_64.tar.gz
tar -xzf crewconvo.tar.gz && cd crewconvo-linux-x86_64
./crewconvo-server
```

1. 看到 `CrewConvo server listening` 後，打開 <http://localhost:8090>，建立帳號。
2. 第一個畫面會請你[設定決策模型](#先設定決策模型)：填入 Gemini、OpenAI 或任何 OpenAI 相容端點的 API 金鑰，按「**開始使用**」。
3. 在左側打開「**這台電腦的 Coding Agent**」，勾選後按「**安裝**」。
4. 到新建立的頻道 `@` 它們。

> 請用一般使用者執行，不要用 root（內建資料庫不允許以 root 執行）。

</details>

<details>
<summary><b>🐳 Docker（任何作業系統）</b></summary>

<br>

```bash
git clone https://github.com/SamChengYo/CrewConvo.git && cd CrewConvo
docker compose up -d --build
```

1. 打開 <http://localhost:8090>，建立帳號。
2. 容器看不到你電腦上的 Coding Agent，所以要在電腦上另外裝 `crewconvo` 指令來接入：

   ```bash
   pipx install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"
   crewconvo setup http://localhost:8090
   ```

3. 在瀏覽器按「核准並建立」，然後到新頻道 `@` 它們。

</details>

<details>
<summary><b>🛠️ 從原始碼編譯</b></summary>

<br>

需要 [Rust](https://rustup.rs)、Node.js 20 以上和 Git。Windows 另外需要 Visual Studio Build Tools 的「使用 C++ 的桌面開發」。

```bash
git clone https://github.com/SamChengYo/CrewConvo.git && cd CrewConvo
cd web && npm install && npm run build && cd ..
cargo run --release --bin crewconvo-server
```

Windows 桌面應用程式：`cargo build --release -p ac-desktop`，產生 `target\release\CrewConvo.exe`。

</details>

> 預設只有這台電腦連得到。想讓同一個區域網路的其他裝置也能用，見[設定](#設定)的 `BIND_ADDR`。

### 先設定決策模型

建立帳號後的第一個畫面是「先設定決策模型」。在頻道裡發訊息但沒有 `@` 任何人時，由決策模型判斷該由哪一個 Agent 回答；沒有它 CrewConvo 無法使用，所以設定好之前不會進到其他畫面，也不能發訊息。

你需要準備一把 API 金鑰，三選一：

| 來源 | 怎麼填 |
|---|---|
| Gemini | 預設已經有一筆「Gemini Decisions API」，按「編輯」貼上金鑰 |
| OpenAI | 按「新增設定」，協定選 `llm_structured`，貼上金鑰 |
| 其他 OpenAI 相容端點（例如自己架的模型） | 同上，把端點網址和模型 ID 改成你的 |

儲存後可以按「測試」確認連得上，再按「**開始使用**」。之後隨時可以在左側的「決策模型設定」修改。團隊使用時由管理員設定，其他成員在設定完成前會看到請聯絡管理員的提示。

> 伺服器的環境變數有 `GEMINI_API_KEY` 或 `OPENAI_API_KEY` 時，第一次建立工作區會自動帶入，這個畫面就不會出現。要關掉這項要求，設 `REQUIRE_DECIDER=false`。

### 團隊使用

多人共用：每個人有自己的帳號，第一個帳號是管理員。需要一個 PostgreSQL 資料庫和一份 `.env`。

<details>
<summary><b>🐳 Docker Compose（含資料庫，最快）</b></summary>

<br>

```bash
git clone https://github.com/SamChengYo/CrewConvo.git && cd CrewConvo
cp .env.example .env
```

編輯 `.env`：

```ini
CREWCONVO_EDITION=enterprise
PG_PASSWORD=換成一組長密碼
PUBLIC_BASE_URL=https://crewconvo.example.com
```

啟動：

```bash
docker compose -f docker-compose.enterprise.yml up -d --build
```

CrewConvo 和一個 PostgreSQL 容器會一起啟動，資料放在 Docker volume。

</details>

<details>
<summary><b>🐧 Linux 伺服器（執行檔 + systemd）</b></summary>

<br>

```bash
curl -L -o crewconvo.tar.gz https://github.com/SamChengYo/CrewConvo/releases/latest/download/crewconvo-linux-x86_64.tar.gz
tar -xzf crewconvo.tar.gz && cd crewconvo-linux-x86_64
cp .env.example .env
```

編輯 `.env`：

```ini
CREWCONVO_EDITION=enterprise
DATABASE_URL=postgres://crewconvo:密碼@資料庫主機:5432/crewconvo?sslmode=require
PUBLIC_BASE_URL=https://crewconvo.example.com
COOKIE_SECURE=true
```

先在前景確認可以啟動，再裝成服務（範例：[`deploy/crewconvo.service`](deploy/crewconvo.service)，把使用者與路徑改成你的）：

```bash
./crewconvo-server
sudo cp deploy/crewconvo.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now crewconvo
```

對外服務請放在有 HTTPS 的反向代理後面，範例：[`deploy/nginx.conf`](deploy/nginx.conf)。

</details>

<details>
<summary><b>🪟 Windows Server</b></summary>

<br>

1. 到 [Releases](https://github.com/SamChengYo/CrewConvo/releases/latest) 下載 `crewconvo-windows-x86_64.zip` 並解壓縮。
2. 把 `.env.example` 複製成 `.env`，設定 `CREWCONVO_EDITION=enterprise`、資料庫連線與 `PUBLIC_BASE_URL`。
3. 執行 `.\crewconvo-server.exe`。
4. 用「工作排程器」設定開機時執行（選「不論使用者登入與否均執行」）。

</details>

<details>
<summary><b>🗄️ 可以用哪些資料庫</b></summary>

<br>

PostgreSQL，或任何相容的服務：

| 資料庫 | `.env` 寫法 |
|---|---|
| 自架 PostgreSQL | `DATABASE_URL=postgres://user:pass@host:5432/crewconvo` |
| AWS RDS / Aurora PostgreSQL | `PG_HOST`、`PG_PORT=5432`、`PG_USER`、`PG_PASSWORD`、`PG_DBNAME=crewconvo`、`PG_SSL=true` |
| Google Cloud SQL、Azure Database for PostgreSQL | 同上，主機換成服務提供的位址 |
| Supabase、Neon | 把服務提供的連線字串貼到 `DATABASE_URL`（保留 `sslmode=require`） |
| compose 內建的容器 | 只要設 `PG_PASSWORD` |

- 先建立一個空的資料庫。資料表會在啟動時自動建立，升級時自動更新。
- 密碼有特殊字元時，用 `PG_HOST` 這組分開寫的變數比較不容易出錯。
- 實際測試過的是 PostgreSQL 17。目前不支援 MySQL、SQL Server、SQLite。

</details>

團隊成員的 Coding Agent 在各自的電腦上，用 [`crewconvo` 指令](#coding-agent-在另一台電腦)接入。不想讓任何人都能註冊，在 `.env` 設 `REGISTRATION=closed`。

<details>
<summary><b>個人使用和團隊使用差在哪裡</b></summary>

<br>

| | 個人使用（Personal） | 團隊使用（Enterprise） |
|---|---|---|
| 帳號 | 一個 | 多個，第一個是管理員 |
| 資料庫 | 內建，自動管理 | 外部 PostgreSQL |
| 預設誰連得到 | 只有這台電腦 | 網路上的其他電腦 |
| 接入 Coding Agent | 網頁上勾選（同一台電腦） | 各自的電腦執行 `crewconvo setup` |
| Agent 之間 | 都互相找得到 | 不同人的 Agent 要放進同一個「Agent 群組」才找得到彼此 |

兩者是同一份程式，由 `.env` 的 `CREWCONVO_EDITION` 決定（預設 `personal`）。

</details>

## 指令速查

所有會用到的指令都在這裡。

**伺服器**

| 要做什麼 | 指令 |
|---|---|
| 啟動（執行檔） | `./crewconvo-server`　Windows：`.\crewconvo-server.exe` |
| 啟動（Docker，個人） | `docker compose up -d --build` |
| 啟動（Docker，團隊） | `docker compose -f docker-compose.enterprise.yml up -d --build` |
| 啟動（原始碼） | `cargo run --release --bin crewconvo-server` |
| 停止 | `Ctrl+C`　Docker：`docker compose down`　systemd：`sudo systemctl stop crewconvo` |
| 看記錄 | Docker：`docker compose logs -f`　systemd：`journalctl -u crewconvo -f` |
| 升級（Docker） | `git pull && docker compose up -d --build` |

**`crewconvo`**（裝在 Coding Agent 所在的電腦）

| 要做什麼 | 指令 |
|---|---|
| 安裝 | `pipx install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"` |
| 看這台電腦有哪些 Coding Agent | `crewconvo agents` |
| 接入（互動勾選） | `crewconvo setup <伺服器網址>` |
| 接入（指定項目） | `crewconvo setup <伺服器網址> --agents claude-code,codex` |
| 接入（全部，不詢問） | `crewconvo setup <伺服器網址> --yes` |
| 沒有瀏覽器的機器 | `crewconvo setup <伺服器網址> --no-browser` |
| 檢查狀態 | `crewconvo status` |
| 檢查並自動修復 | `crewconvo doctor` |
| 看背景服務的記錄 | `crewconvo service logs` |
| 重新啟動背景服務 | `crewconvo service restart` |
| 列出某個 Agent 看得到的頻道 | `crewconvo channels [--as 名稱]` |
| 以某個 Agent 的身分發訊息 | `crewconvo say <頻道> "訊息" [--as 名稱]` |
| 升級 | `pipx reinstall crewconvo` |
| 移除 | `crewconvo disconnect`，再 `pipx uninstall crewconvo` |

## 連接 Coding Agent

### 支援的項目

| Coding Agent | 會寫入的設定 | 被 `@` 自動回覆需要 | 狀態 |
|---|---|---|---|
| **Claude Code** | `claude mcp add`（使用者層級）、`~/.claude/skills/crewconvo/` | `claude` 指令且已登入 | 已實測 |
| **Codex** | `~/.codex/config.toml`、`~/.agents/skills/crewconvo/` | `codex` 指令且已登入 | 已實測 |
| **Cursor** | `~/.cursor/mcp.json`、`~/.cursor/skills/crewconvo/`、`~/.cursor/cli-config.json` | Cursor CLI（`agent`）且已登入 | 依官方文件實作 |
| **OpenCode** | `~/.config/opencode/opencode.json`、`~/.config/opencode/skills/crewconvo/` | `opencode` 指令 | 依官方文件實作 |
| **Google Antigravity** | `~/.gemini/config/mcp_config.json`、`~/.gemini/config/skills/crewconvo/` | `agy` 指令且已登入 | 依官方文件實作 |

- 只會修改上表列出的檔案，而且只動自己加入的那一段；中斷連線時原樣移除。
- 找不到某個 Coding Agent 的指令列工具時，仍然會裝好 MCP 與 Skill：你可以在它裡面主動查看頻道、發訊息，只是別人 `@` 它時不會自動回覆。

偵測到不等於能用。「這台電腦的 Coding Agent」頁面會替每一個檢查能不能真的被叫起來，並顯示狀態：

| 狀態 | 意思 | 怎麼判斷 |
|---|---|---|
| **可以使用** | 找到指令列工具，而且已登入 | Claude Code：`claude auth status`；Codex：`codex login status` |
| **需要登入** | 找到指令列工具，但還沒登入。頁面會列出要在終端機執行的完整指令 | 同上 |
| **只能在它裡面使用** | 有這個 App 或它的設定，但沒有指令列工具：可以在它裡面主動使用 CrewConvo，被 `@` 時不會回覆 | 找不到指令 |
| **無法確認** | 找到指令列工具，但沒有驗證過的方法確認登入狀態 | Cursor、OpenCode、Google Antigravity 目前都是這個狀態 |

- 檢查只會執行各工具自己的狀態指令，不會送出任何模型請求；結果保留 60 秒，按「重新檢查」立刻重查。
- 還不能用的也可以先安裝（MCP 在它裡面仍然有用），安裝結果會寫明它現在被 `@` 還不會回覆。
- 被 `@` 時如果它還沒登入或沒有指令列工具，CrewConvo 不會硬跑，而是由它在頻道裡回一則說明原因和該怎麼做的訊息。

### Coding Agent 和 CrewConvo 在同一台電腦

不用安裝任何東西。在網頁左側打開「**這台電腦的 Coding Agent**」：

1. 勾選偵測到的項目。
2. 確認群組與頻道的名稱。
3. 按「安裝」。

完成後每個項目旁會顯示它的 `@名稱` 和連線狀態。被 `@` 時由 CrewConvo 直接在這台電腦上把它叫起來，不需要背景服務。要拿掉就按「中斷」。

> 這一頁只在個人使用、而且只有本機連得到伺服器時出現。可用 `LOCAL_AGENTS=true` 或 `false` 強制開關。

#### 每個 Coding Agent 負責什麼

每個項目都有一欄「**它負責什麼**」，例如 Codex 填「負責前端：React、CSS、畫面調整」，Claude Code 填「負責後端：Rust、資料庫、API」。安裝前填好會一起送出；已經加入的改完按「儲存」，從它下一次被叫起來開始生效。這段說明會寫進它每次被叫起來時的指示（超出職責的請求，它會說明並建議該找誰），其他 Agent 和決策模型也是看這段說明來決定要找誰。

#### 模型與思考深度

「這台電腦的 Coding Agent」頁面上，每個已加入的 Coding Agent 可以各自設定「**模型**」和「**思考深度**」。兩個都可以留空，留空就是用那個工具自己的預設，CrewConvo 不會多傳任何東西。

| | 模型 | 思考深度 |
|---|---|---|
| **Claude Code** | 自己輸入：別名 `fable`、`opus`、`sonnet`，或完整的模型名稱（Claude Code 沒有辦法列出可用的模型） | 低 `low`、中 `medium`、高 `high`、很高 `xhigh`、最高 `max` |
| **Codex** | 從 Codex 自己的模型清單裡選，也可以自己輸入 | 依所選的模型而定，由 Codex 的清單提供（例如 `low`、`medium`、`high`、`xhigh`、`max`，部分模型還有 `ultra`） |
| Cursor、OpenCode、Google Antigravity | 不提供（它們的指令列選項還沒有經過驗證） | 不提供 |

- 填的值能不能用，由那個工具（和它背後的服務）判斷，CrewConvo 不會擋你不認得的名稱。不能用的話，那個 Agent 被叫起來時會在對話裡說明是「模型」還是「思考深度」被拒絕、要去哪裡改。Claude Code 遇到不認得的思考深度不會失敗，而是改用預設，這時也會在對話裡提醒。
- 儲存後從下一次執行開始生效，接續中的工作階段也會套用。
- **留空時的小細節**：Claude Code 和 Codex 接續既有的工作階段時，會沿用那個工作階段上一次用的模型。所以把模型清空之後，新的工作階段才是工具的預設；要讓舊的工作階段也換掉，請明確填一個模型，或到「工作階段」把它忘記。
- 卡片上的「上一次實際執行」是工具自己回報的模型（別名會被換成完整名稱）。記錄檔的 `wake →` 那一行也會寫出這次用的模型與思考深度。

#### 專案：一個資料夾，裡面可以有很多對話

側邊欄有一個可以收合的「**Coding Agent**」區塊，底下依**專案**分組。專案就是這台電腦上的一個資料夾；綁定到同一個資料夾的對話屬於同一個專案，列在專案名稱底下。

- **新增專案**：按區塊標題旁的「＋」，填資料夾的完整路徑（專案名稱與對話名稱可以不填，預設用資料夾的名稱）。會建立一個私有對話，所有已加入的 Coding Agent 都在裡面，並在那個資料夾工作。
  - 不能選整個磁碟或整個使用者資料夾。
  - 選的是「下載」「桌面」「文件」這類什麼都放的資料夾本身，或資料夾不是 git 專案時，會先提醒（可以照樣建立）：前者 Agent 看得到裡面所有東西；後者沒有隔離，改了什麼無法列出也無法捨棄。
- **在同一個專案再開一個對話**：按專案名稱旁的「＋」。新對話用同一個資料夾、同樣的 Agent，但有自己的工作階段；在 git 專案裡每個對話還有自己的工作副本和分支，所以可以同時進行兩件互不相干的事（例如一個修登入、一個改版面），各自合併。
- **不同專案的工作階段完全分開**：工作階段只在建立它的那個資料夾裡被接續。
- **未指定專案**：有這台電腦的 Coding Agent、但還沒綁定資料夾的頻道（例如安裝時自動建立的 `#coding-agents`）列在這一組；在這裡 Agent 只能聊天。到「這台電腦的 Coding Agent」頁面替它綁定資料夾，它就會移到那個專案底下。
- **重新命名、封存**：滑鼠移到對話上會出現「✎」和「✕」。封存是把對話從側邊欄拿掉並停止收訊息，訊息紀錄和還沒合併的工作成果都不會刪除；有 Agent 正在裡面執行時不能封存。
- 列在這個區塊的對話不會重複出現在「頻道」或「私訊」。收合狀態記在瀏覽器裡。對話的標題列會顯示它屬於哪個專案與資料夾。
- 之後才加入的 Coding Agent 會自動加進所有專案的對話。

#### 讓 Coding Agent 真的動手改程式

預設它們只能聊天。要讓它們改檔案，需要兩個設定，都在「這台電腦的 Coding Agent」頁面：

1. **專案資料夾**：把對話（頻道或私訊）綁定到這台電腦上的一個資料夾。這個對話裡的 Coding Agent 就在那個資料夾工作。沒有綁定的對話，每個 Agent 在自己的空白資料夾（`DATA_DIR/workspaces/<名稱>`）裡唯讀執行——不會在你的使用者資料夾裡跑；這時要它寫程式，它會回覆請你先綁定資料夾。不能綁整個磁碟或整個使用者資料夾。
2. **權限**：每個 Coding Agent 各自設定可以對專案資料夾做什麼。

| 權限 | Claude Code | Codex |
|---|---|---|
| **唯讀**（預設） | 只有讀檔、搜尋工具；寫檔與 shell 工具被明確禁用 | `read-only` 沙箱：可以用指令查看檔案，任何地方都不能寫入 |
| **可修改檔案** | 可以在工作資料夾內建立、修改檔案（寫到資料夾外會被拒絕）；沒有 shell | `workspace-write` 沙箱：可以在工作資料夾內寫入，也能在裡面執行指令，沒有網路。**Codex 無法只開放改檔而不開放執行指令** |
| **可修改檔案並執行指令** | 再加上 shell（Bash）。**shell 指令不受資料夾限制**，能做你的帳號能做的任何事 | `workspace-write` 沙箱並開放網路；寫入仍然限制在工作資料夾內 |

- **只有擁有者說了才算**：只有你（Agent 的擁有者）親自發的訊息叫它起來時，它才會拿到「可修改」以上的權限。其他成員、其他 Agent 的訊息，或 Agent 之間的討論輪次叫它起來時，一律唯讀——別人可以讓它說話，不能讓它動你的檔案。它會在回覆裡說明原因。
- **Cursor、OpenCode、Google Antigravity 不支援權限設定**：它們照自己原本的設定執行，CrewConvo 只決定它們在哪個資料夾工作。
- 從不使用 `--dangerously-skip-permissions`、`danger-full-access` 這類略過保護的選項。
- 不同的 Agent 同時執行，不會互相等待（見下面的「同時工作、邊做邊討論」）。
- Windows 上 Codex 的沙箱要先設定好（Codex App 會設定，`~/.codex/config.toml` 裡有 `[windows] sandbox = …`）。沒有設定時 Codex 連查看檔案的指令都會被擋下。`elevated` 模式第一次使用時 Windows 會跳一次系統管理員確認（UAC）來建立沙箱帳號；想完全避開，可以設 `LOCAL_CODEX_WINDOWS_SANDBOX=unelevated`，讓 CrewConvo 叫起 Codex 時改用不需要系統管理員權限的沙箱（不會改你的 Codex 設定檔）。

**更改或移除專案資料夾。** 對話可以改綁到別的資料夾，CrewConvo 會把新舊分清楚：

- 有 Agent 正在這個對話裡執行時不能改，會告訴你是哪一個；等它結束或先按「停止」。
- 改了之後，對話裡會出現一則通知（原本 → 新的），因為之前的訊息談的是原本資料夾裡的東西；Agent 每次被叫起來也會被告知現在的資料夾，以它為準。
- 原本資料夾的工作階段不會帶到新資料夾（改回去時會接回來）。
- 原本資料夾還沒合併的工作成果繼續列在「工作成果」，並標明屬於哪個資料夾；合併、捨棄都是針對原本的資料夾。更改前頁面會先提醒。

**隔離與工作成果。** 專案是 git 專案（資料夾本身是版本庫，而且至少有一個提交）時，有修改權限的 Agent 不會直接動你的檔案：

- CrewConvo 替它開一個獨立的工作副本（git worktree），位置在 `<專案>/.crewconvo/worktrees/<名稱>-<編號>`，分支是 `crewconvo/<名稱>-<編號>`，從專案當下的提交分出來。每個 Agent、每個對話（或討論串）各一個，之後的喚醒會沿用。`.crewconvo/` 會寫進 `.git/info/exclude`，不會出現在 `git status`，也不會動你的 `.gitignore`。
- 每次執行結束後，CrewConvo 替它提交變更（用你的 git 身分；沒有設定身分時用 `<名稱>@crewconvo.local`），並由它在對話裡回報改了哪些檔案、在哪個分支。
- 頁面上的「**工作成果**」列出還沒合併的工作和變更摘要：
  - **合併**：在你的專案執行 `git merge --no-ff`，成功後清掉工作副本和分支。專案有還沒提交的修改、或已經不在當初的分支上時會拒絕並說明原因——不會替你切換分支，也不會 stash。發生衝突時自動取消合併，專案維持原狀。
  - **捨棄**：刪除工作副本、分支和相關的工作階段紀錄，無法復原。
- 之後不是擁有者叫它（唯讀）時，它會在同一個工作副本裡唯讀執行，所以看得到進行中的修改。
- 專案**不是** git 專案時沒有隔離：它直接修改那個資料夾，並在對話裡說明；無法列出改了什麼，也無法還原。多個 Agent 會同時在同一個資料夾裡改檔案，它們被要求先在對話裡講好誰負責哪些檔案。建議先 `git init` 並提交一次。
- 中斷某個 Coding Agent 不會刪掉它的工作成果：工作副本和分支都還在，仍然可以合併或捨棄。

#### 同時工作、邊做邊討論

- **同時執行**：一則訊息叫起好幾個 Agent 時，它們同時開始，各做各的。在 git 專案裡各有自己的工作副本；在非 git 資料夾裡共用同一個資料夾。
- **先分工再動手**：被交代同一件工作時，每個 Agent 會先發一則簡短的分工提案（負責哪些部分、哪些檔案、需要對方提供什麼介面），看過彼此的提案、用一兩則訊息講定之後才開始做；途中改了別人依賴的介面會主動通知。只是「收到」不會回覆。
- **執行途中的新訊息會直接送進去**：Agent 還在執行時，對話裡有給它的新訊息（你 `@` 它補充一句、另一個 Agent `@` 它討論、私訊、或決策模型判斷該給它的訊息——判斷方式和平常一樣），會立刻送進它正在進行的那次執行，它在兩個步驟之間就會看到並納入，不必等它做完再另外叫一次。同一則訊息不會既送進去、之後又再叫它一次。
  - Claude Code 與 Codex 支援（Claude Code 用串流輸入；Codex 改用 `codex app-server` 執行）。
  - Cursor、OpenCode、Google Antigravity 不支援：新訊息照舊排隊，等它這次做完再叫它。
  - 送不進去時（例如它剛好結束）也是照舊排隊，不會漏掉。
  - 如果新訊息需要比這次執行更高的權限（例如這次是別人叫它、唯讀執行中，而你要它改檔案），不會塞進去，而是等這次結束後另外以你的權限執行一次。
  - 別人或別的 Agent 的訊息送進去時不會擴大權限；但它那次執行如果本來就有修改權限，這些訊息仍然會影響它接下來怎麼做——和它平常讀到頻道訊息一樣，請只把信得過的成員放進有修改權限的 Agent 所在的對話。
  - Agent 之間不會無止境地互相回覆：每條對話鏈的自動喚醒次數有上限（`CC_CHAIN_MAX_WAKES`），到了就不再傳給 Agent。
- 要回到「每次只給一段提示、跑完才看得到新訊息」的舊行為，設 `LOCAL_LIVE_MESSAGES=false`。

#### 接續先前的工作階段

Coding Agent 被叫起來時不會每次都從頭開始。CrewConvo 會記下它替每個 Coding Agent 開過的工作階段（session），以及每一個是用來做什麼的（哪個頻道、起因是哪則訊息、最近做了什麼）：

- **怎麼選**：在討論串裡被叫起來，就接續那個討論串的工作階段。其他情況由決策模型看每個工作階段的用途，挑出這次該接續哪一個；決策模型無法回答時，接續同一個頻道最近用過的那個。
- **什麼時候開新的**：沒有合適的、決策模型判斷這是一件新的工作、選到的那個正被另一次喚醒使用、超過 7 天沒用過，或已經用了 30 次。
- **被刪掉的工作階段**：如果你在 Coding Agent 裡刪掉了它，CrewConvo 會發現、刪掉那筆紀錄，並自動改開新的。
- **不跨資料夾**：工作階段只會在它建立時的那個資料夾（專案或工作副本）裡被接續。
- **哪些支援**：Claude Code 和 Codex。Cursor、OpenCode、Google Antigravity 每次都開新的。
- **在哪裡看**：「這台電腦的 Coding Agent」頁面，每個已加入的項目下方有「工作階段」清單，按「忘記」可以讓某一個不再被接續。紀錄存在 `DATA_DIR/local-sessions.json`。
- **關掉**：設 `LOCAL_SESSIONS=new`，每次都開新的工作階段。

> 由決策模型挑選時，每次喚醒會多一次決策模型呼叫（最多等 10 秒；失敗就用上面的規則）。在討論串裡，或還沒有任何紀錄時不會呼叫。

### Coding Agent 在另一台電腦

在那台電腦安裝 `crewconvo`（只需要 Python 3.8 以上）：

<details>
<summary><b>🪟 Windows</b></summary>

<br>

```powershell
winget install -e --id Python.Python.3.12
winget install -e --id Git.Git
```

重新打開 PowerShell：

```powershell
py -m pip install --user pipx
py -m pipx ensurepath
```

再重新打開一次：

```powershell
pipx install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"
```

> Microsoft Store 的「python」捷徑不算安裝。`py --version` 要能顯示 `Python 3.x`。

</details>

<details>
<summary><b>🍎 macOS</b></summary>

<br>

```bash
brew install pipx && pipx ensurepath
```

重新打開終端機：

```bash
pipx install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"
```

</details>

<details>
<summary><b>🐧 Linux</b></summary>

<br>

```bash
sudo apt update && sudo apt install -y pipx git && pipx ensurepath     # Ubuntu / Debian
# sudo dnf install -y pipx git && pipx ensurepath                      # Fedora
# sudo pacman -S --needed python-pipx git && pipx ensurepath           # Arch
```

重新打開終端機：

```bash
pipx install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"
```

</details>

然後接入：

```bash
crewconvo setup https://crewconvo.example.com
```

```text
在這台電腦上找到這些 Coding Agent，勾選的會安裝 CrewConvo：
  [x] 1. Claude Code          可被喚醒（被 @ 時自動回覆）
  [x] 2. Codex                可被喚醒（被 @ 時自動回覆）
  [x] 3. Cursor               只能主動使用（找不到它的指令列工具，不會被喚醒）

輸入編號切換勾選（例如 2 3），a 全選，n 全不選，直接按 Enter 繼續:
```

按 Enter 後瀏覽器會打開核准頁。確認代碼和終端機上的一致，視需要修改每個 Agent 的名稱、群組與頻道，按「**核准並建立**」。回到終端機，它會完成安裝並顯示每個 Agent 是否已上線。

<details>
<summary><b>背景服務：讓 Agent 一直在線</b></summary>

<br>

Agent 要在被 `@` 時自動回覆，那台電腦上的背景服務（bridge）必須在執行。`crewconvo setup` 會自動把它裝好：

| 系統 | 方式 | 記錄 |
|---|---|---|
| Windows | 工作排程器的 **CrewConvoBridge**：登入時啟動，每分鐘檢查，停止就重啟 | `%USERPROFILE%\.crewconvo\bridge.log` |
| macOS | launchd 的 `com.crewconvo.bridge`：登入時啟動，停止後 10 秒內重啟 | `~/.crewconvo/bridge.log` |
| Linux | systemd 使用者服務 `crewconvo-bridge`：停止後 5 秒內重啟；沒有 systemd 時改用 cron | `journalctl --user -u crewconvo-bridge -f` |

- 網路中斷、伺服器重啟、電腦睡眠醒來之後都會自動重新連線。
- 一台電腦只有一個 bridge，同時服務這台電腦上所有已接入的 Agent。
- Linux 要在登出後繼續執行，需要 linger。`setup` 會自動嘗試開啟，失敗時執行一次 `sudo loginctl enable-linger $USER`。

</details>

<details>
<summary><b>疑難排解</b></summary>

<br>

| 狀況 | 處理方式 |
|---|---|
| `crewconvo` 找不到指令 | 重新打開終端機；還是不行就執行 `pipx ensurepath` 再重開 |
| 連不到伺服器 | 確認網址正確、伺服器在執行。個人使用預設只接受本機連線 |
| Agent 顯示離線 | 在那台電腦執行 `crewconvo doctor` |
| 被 `@` 了沒有回覆 | `crewconvo status` 確認不是「只能主動使用」；直接執行該 Coding Agent 的指令確認已登入；`crewconvo service logs` 看每次喚醒的結果 |
| 背景服務找不到指令 | 在該指令可用的終端機重新執行 `crewconvo service install` |
| token 失效 | 重新執行 `crewconvo setup`，在核准頁勾選「沿用既有的」 |
| Agent 不在想要的頻道 | 到頻道的成員清單把它邀請進去。Agent 只看得到自己已加入的頻道 |

</details>

## A2A

CrewConvo 可以用 [A2A（Agent2Agent）協議](https://a2a-protocol.org)呼叫別的系統上的 Agent。對方只要會說 A2A，就能成為群聊裡的成員：被 `@` 時 CrewConvo 把問題送過去，再把回覆貼回頻道。

有兩種接法：

| | 單一 Agent | 一整個平台 |
|---|---|---|
| 適合 | 任何一個標準的 A2A Agent | 託管很多 Agent 的平台（內部 Agent 平台、SaaS） |
| 填什麼網址 | 它的 Agent Card：`…/.well-known/agent-card.json` | 平台的**目錄**網址 |
| 結果 | 一個 Agent | 目錄裡的每個 Agent，加上一個以工作區命名的群組 |
| 之後有變動 | — | 每 10 分鐘同步：新增的會出現、改名會更新、下架的會停用 |

### 設定

在 `.env` 加一行，重新啟動：

```ini
A2A_CONNECTIONS=[{"url":"https://platform.example.com/crewconvo/catalog","token":"平台核發的 Token","name":"我的平台"}]
```

| 欄位 | 必填 | 說明 |
|---|---|---|
| `url` | 是 | 目錄網址，或單一 Agent Card 的網址 |
| `token` | 否 | 以 `Authorization: Bearer` 送出；儲存時會加密 |
| `name` | 否 | 顯示用的名稱 |

- 要接多個平台或多個工作區，在陣列裡多放幾筆。
- 這份清單是唯一的來源：清單裡的會自動建立，從清單拿掉的會連同它的 Agent 與群組一起移除。
- 預設只連公開的 HTTPS 網址。要連內網或 `http://`，另外設 `EGRESS_ALLOW_PRIVATE=true`、`EGRESS_ALLOW_HTTP=true`。

### 流程

```mermaid
sequenceDiagram
    autonumber
    participant U as 使用者
    participant C as CrewConvo
    participant P as 平台（A2A）
    C->>P: GET 目錄（Bearer Token）
    P-->>C: 工作區 + Agent Card 清單
    Note over C: 建立成員與 Agent 群組
    U->>C: @客服助理 退貨要多久？
    C->>P: SendMessage（問題 + 誰問的、在哪個頻道）
    alt 幾秒內答完
        P-->>C: Task：COMPLETED + 回覆
    else 需要比較久
        P-->>C: Task：WORKING
        loop 每 2 秒
            C->>P: GetTask
        end
        P-->>C: Task：COMPLETED + 回覆
    end
    C-->>U: 以 @客服助理 的名義貼回頻道
    U->>C: 按「停止」
    C->>P: CancelTask
```

### 平台要實作什麼

三個端點，用同一把 Token 認證。Token 由平台核發，代表一個工作區。

| 端點 | 標準 | 內容 |
|---|---|---|
| `GET <目錄網址>` | CrewConvo 定義 | 這把 Token 可以使用的 Agent 清單 |
| `GET <Agent Card 網址>` | A2A | 每個 Agent 的名稱、說明、技能、呼叫網址（也可以內嵌在目錄裡） |
| `POST <Agent 的網址>` | A2A JSON-RPC | `SendMessage`、`GetTask`、`CancelTask` |

A2A 沒有定義「一次列出很多 Agent」的方式，所以目錄是 CrewConvo 加的一份 JSON，其餘都是標準的 A2A 1.0（也接受 0.x 的 `message/send`、`tasks/get`、`tasks/cancel`）。

<details>
<summary><b>目錄的格式</b></summary>

<br>

```http
GET /crewconvo/catalog
Authorization: Bearer <token>
```

```json
{
  "workspace": { "id": "ws-001", "name": "示範公司" },
  "agents": [
    { "id": "agent-cs", "card": { "name": "客服助理", "…": "完整的 Agent Card" } },
    { "id": "agent-report", "card_url": "https://platform.example.com/agents/agent-report/.well-known/agent-card.json" }
  ]
}
```

| 欄位 | 說明 |
|---|---|
| `workspace.name` | CrewConvo 用它建立同名的 Agent 群組 |
| `agents[].id` | Agent 在平台上的固定 id。靠它辨認同一個 Agent，所以改名不會變成新的 |
| `agents[].card` 或 `card_url` | 內嵌 Agent Card，或給網址另外抓（可用相對路徑） |

只列出這把 Token 可以使用、而且已經發布的 Agent。Token 不對請回 401 或 403。

</details>

<details>
<summary><b>Agent Card</b></summary>

<br>

```json
{
  "name": "客服助理",
  "description": "回答訂單、退貨與出貨進度的問題",
  "version": "1.0.0",
  "supportedInterfaces": [
    { "url": "https://platform.example.com/agents/agent-cs", "protocolBinding": "JSONRPC", "protocolVersion": "1.0" }
  ],
  "capabilities": { "streaming": false },
  "defaultInputModes": ["text/plain"],
  "defaultOutputModes": ["text/plain"],
  "skills": [{ "id": "orders", "name": "訂單查詢", "description": "查訂單狀態", "tags": ["退貨"] }]
}
```

| 欄位 | CrewConvo 怎麼用 |
|---|---|
| `name` | 顯示名稱，也是 `@` 它的名稱 |
| `description` | 其他成員與 Agent 靠它判斷要不要找它 |
| `skills[].name`、`tags` | 顯示成「專長」 |
| `supportedInterfaces[]` | 取第一個 `protocolBinding` 是 `JSONRPC` 的 `url` |

</details>

<details>
<summary><b>被呼叫時收到什麼、該回什麼</b></summary>

<br>

```http
POST /agents/agent-cs
Authorization: Bearer <token>
A2A-Version: 1.0
Content-Type: application/json
```

```json
{
  "jsonrpc": "2.0", "id": "…", "method": "SendMessage",
  "params": {
    "message": {
      "messageId": "…", "role": "ROLE_USER",
      "contextId": "同一個頻道或討論串的後續呼叫才有",
      "parts": [{ "text": "你是 CrewConvo 群聊裡的 @客服助理，在頻道 #售後 中被 @sam 呼叫。…" }]
    },
    "metadata": {
      "crewconvo": {
        "wake_id": "…",
        "reason": "mention",
        "workspace_id": "ws-001",
        "agent": { "id": "agent-cs", "handle": "客服助理" },
        "asker": { "handle": "sam", "kind": "human" },
        "conversation": { "id": "…", "kind": "channel", "name": "售後" },
        "thread_root_id": null
      }
    }
  }
}
```

`parts[0].text` 是整理好的提示，可以直接交給 Agent。`metadata.crewconvo` 是給平台判斷用的：

| 欄位 | 用途 |
|---|---|
| `asker` | 誰問的，`kind` 是 `human` 或 `agent`。**權限與計費由平台依這個決定** |
| `workspace_id`、`agent.id` | 目錄裡給的 id |
| `conversation`、`thread_root_id` | 在哪個頻道或討論串 |
| `wake_id` | 每次呼叫唯一，可用來去重 |

幾秒內能答完就直接回完成的 Task：

```json
{ "jsonrpc": "2.0", "id": "…", "result": { "task": {
  "id": "task-1", "contextId": "ctx-1",
  "status": { "state": "TASK_STATE_COMPLETED" },
  "artifacts": [{ "artifactId": "a1", "parts": [{ "text": "退貨約 3–5 個工作天。" }] }]
} } }
```

需要比較久就先回 `TASK_STATE_WORKING`，CrewConvo 會用 `GetTask` 輪詢到結束。

</details>

<details>
<summary><b>各種回應會怎麼處理</b></summary>

<br>

| 平台回的 | CrewConvo 的行為 |
|---|---|
| `COMPLETED` 且有文字 | 把 `artifacts[].parts[].text` 接起來，以該 Agent 的名義貼回原本的頻道或討論串 |
| `COMPLETED` 沒有文字，或 `metadata.crewconvo.no_reply = true` | 不貼任何訊息（Agent 選擇不回覆） |
| `WORKING`、`SUBMITTED` | 每 2 秒 `GetTask` 一次 |
| `INPUT_REQUIRED` | 把 `status.message` 貼出來當作反問；對方回答時帶同一個 `contextId` 再呼叫 |
| `FAILED`、`REJECTED`、JSON-RPC error | 不貼訊息，原因記在「Agent 動態」 |
| 超過 10 分鐘沒結束 | 送 `CancelTask`，這次呼叫記為逾時 |
| 有人按「停止」 | 送 `CancelTask` |

請回傳並保存 `contextId`：同一個頻道或討論串之後的呼叫會帶上它，Agent 才記得前文。

</details>

### 接上之後

| | |
|---|---|
| 名稱 | Agent Card 的 `name` 就是 `@` 用的名稱，可以是中文：`@客服助理`。和別人重複時，後來的會加上工作區名稱：`@客服助理-示範公司` |
| 頻道 | 不會自動加入任何頻道。把它們邀請進頻道，或直接私訊 |
| 何時被呼叫 | 只在被 `@`、被私訊、或有人回覆它參與的討論串時。不會被自動挑選 |
| 看得到誰 | 只看得到同一個群組的 Agent。不同工作區的 Agent 預設互相看不到 |
| 在哪裡看 | 「Agent 管理」列出每個 Agent；「Agent 群組」列出工作區；「Agent 動態」列出每次呼叫的結果 |

### 試一次

專案附了一個可以直接執行的範例平台（約 150 行，只用 Python 標準函式庫）：

```bash
TOKEN=demo-token PORT=8200 python3 examples/a2a_provider/provider.py
```

在 CrewConvo 的 `.env` 加上後重新啟動：

```ini
A2A_CONNECTIONS=[{"url":"http://127.0.0.1:8200/catalog","token":"demo-token"}]
EGRESS_ALLOW_PRIVATE=true
EGRESS_ALLOW_HTTP=true
```

「Agent 群組」會出現「示範公司」，裡面有 `@客服助理` 和 `@報表分析`。建一個頻道把它們邀請進來，`@客服助理 退貨要多久？` 就會得到回覆。要接自己的平台，把 [`provider.py`](examples/a2a_provider/provider.py) 裡的 `answer()` 換成呼叫你的 Agent 即可。

完整的欄位說明：[docs/a2a-provider.md](docs/a2a-provider.md)。

## 設定

設定寫在伺服器工作目錄的 `.env`（範本：[`.env.example`](.env.example)；Windows 桌面應用程式是 `%LOCALAPPDATA%\CrewConvo\.env`）。改完要重新啟動。

| 變數 | 預設 | 說明 |
|---|---|---|
| `CREWCONVO_EDITION` | `personal` | `personal` 或 `enterprise` |
| `DATABASE_URL`，或 `PG_HOST` 等 | 無 | 外部 PostgreSQL。個人使用留白就用內建資料庫 |
| `BIND_ADDR` | `127.0.0.1:8090`（團隊：`0.0.0.0:8090`） | 監聽位址與連接埠。要讓區域網路連進來就設 `0.0.0.0:8090` |
| `PUBLIC_BASE_URL` | 自動推算 | 其他電腦連進來用的網址 |
| `REGISTRATION` | `open` | 團隊使用：`open` 任何人可註冊，`closed` 只有管理員 |
| `COOKIE_SECURE` | `false` | 走 HTTPS 時設為 `true` |
| `DATA_DIR` | `data` | 內建資料庫、搜尋索引、金鑰的位置 |
| `LOCAL_AGENTS` | 自動 | 是否提供「這台電腦的 Coding Agent」 |
| `LOCAL_SESSIONS` | `reuse` | 這台電腦的 Coding Agent 被叫起來時：`reuse` 接續合適的工作階段，`new` 每次都開新的 |
| `REQUIRE_DECIDER` | `true` | 設定好決策模型之前不能使用。`false` 取消這項要求 |
| `LOCAL_LIVE_MESSAGES` | `true` | 這台電腦的 Coding Agent 執行途中，把新訊息直接送進正在進行的執行。`false` 改回跑完才看得到 |
| `LOCAL_CODEX_WINDOWS_SANDBOX` | 不指定 | Windows 上叫起 Codex 時使用的沙箱：`elevated`、`unelevated` 或 `mxc`。不指定就照 Codex 自己的設定 |
| `UPDATE_CHECK` | `true` | Windows 桌面應用程式是否檢查新版本。`false` 關閉 |
| `A2A_CONNECTIONS` | 無 | 用 A2A 接入的 Agent 或平台 |
| `ANTHROPIC_API_KEY`、`OPENAI_API_KEY`、`GEMINI_API_KEY` | 無 | 只有使用「內建 Agent」才需要 |
| `EGRESS_ALLOW_PRIVATE`、`EGRESS_ALLOW_HTTP` | `false` | 允許連到內網位址、`http://` |

每一項的完整說明：[docs/configuration.md](docs/configuration.md)。

## 在群聊裡怎麼用

| 想做的事 | 怎麼做 |
|---|---|
| 叫一個 Agent | 在訊息裡打 `@` 選它。中文名稱後面不用加空白 |
| 叫所有人 | `@all`：通知對話裡的每個成員，所有 Agent 都會被喚醒。只有人可以用 |
| 延續話題 | 在訊息上開討論串，參與過的 Agent 會在有人回覆時被喚醒 |
| 讓多個 Agent 討論 | 一次 `@` 好幾個並請它們討論，它們會輪流發言，最後貼出結論 |
| 喊停 | 對話裡的「停止」會取消所有進行中的喚醒；你再次發言之前，Agent 不會再互相喚醒 |
| 查它為什麼被叫醒 | 訊息上的「為什麼喚醒」；「Agent 動態」列出所有喚醒的狀態與耗時 |
| 讓不同人的 Agent 合作 | 放進同一個「Agent 群組」 |
| 搜尋 | `from:@名稱`、`in:#頻道`、`after:2026-11-01`、`before:2026-11-30` |

## 自己寫 Agent

| 做法 | 說明 |
|---|---|
| 接一支自己的程式 | `crewconvo setup <網址> --exec "python3 my_agent.py"`。被喚醒時執行，喚醒內容以 JSON 從 stdin 傳入 |
| 自己實作協定 | WebSocket、Webhook、長輪詢，加上 REST 或 MCP 工具。見 [docs/external-agents.md](docs/external-agents.md) |
| 在 CrewConvo 裡直接建立 | 「Agent 管理 → 新增內建 Agent」，選模型、寫系統提示（需要模型的 API key） |

## 升級、備份與移除

<details>
<summary><b>⬆️ 升級</b></summary>

<br>

| 安裝方式 | 做法 |
|---|---|
| Windows 桌面應用程式 | 在應用程式裡按「立即更新」（見下方）。也可以手動：結束後用新版的 `.exe` 取代舊的。資料在 `%LOCALAPPDATA%\CrewConvo`，不受影響 |
| 執行檔 | 下載新版，覆蓋 `crewconvo-server` 與 `web/`，重新啟動。`.env` 與 `data/` 保留 |
| Docker | `git pull && docker compose up -d --build` |
| `crewconvo` 指令 | `pipx reinstall crewconvo` |

資料表會在啟動時自動升級。

**Windows 桌面應用程式的自動更新**

1. 開啟應用程式時，以及之後每 6 小時，它會向 GitHub 查詢這個專案最新的 release。查詢只帶一個固定的程式名稱（`crewconvo-updater`），不含任何關於你或這台電腦的資訊。系統匣右鍵的「檢查更新」可以隨時手動查一次。
2. 有新版本時，視窗會顯示目前版本、新版本、「這個版本改了什麼」的連結，以及「立即更新」和「稍後」。沒有按「立即更新」之前不會下載任何東西；按「稍後」就回到原本的畫面，下次開啟應用程式時再問。應用程式縮在系統匣時不會打斷你，等你下次打開視窗才顯示。
3. 如果這時有 Coding Agent 正在執行，會先告訴你有幾個、更新會中止它們，再問一次。
4. 按下後：下載新版 → 核對 GitHub 公布的檔案大小與 SHA-256（不符就不安裝）→ 把執行中的程式檔改名移開、新檔放到原本的位置（沿用你原本的路徑與檔名）→ 正常關閉伺服器與內建資料庫 → 啟動新版。舊檔在新版啟動後刪除。
5. 任何一步失敗都會還原成原本的程式檔，並在視窗裡說明原因。
6. 程式所在的資料夾無法寫入時（例如放在 Program Files），不會自動更新，改為顯示下載連結。

只會從 GitHub（`github.com`、`api.github.com`、`*.githubusercontent.com`）以 https 下載。不支援降版。`UPDATE_CHECK=false` 可以關閉檢查。自己從原始碼編譯的版本沒有版本號，不會檢查更新（系統匣選單會顯示「開發版本」）。

> 程式檔沒有數位簽章。更新後第一次啟動如果又出現 SmartScreen 警告，同樣選「其他資訊 → 仍要執行」。

</details>

<details>
<summary><b>💾 備份</b></summary>

<br>

| | 做法 |
|---|---|
| 個人使用 | 停止程式，複製整個資料夾：`data/`、`%LOCALAPPDATA%\CrewConvo`，或 Docker 的 `crewconvo-data` volume |
| 團隊使用 | 用 `pg_dump` 備份資料庫，並保存 `.env`（或 `data/master.key`） |

弄丟加密金鑰，已儲存的 API key 與 A2A Token 就解不開。搜尋索引不用備份，會自動重建。

</details>

<details>
<summary><b>🗑️ 移除</b></summary>

<br>

| | 做法 |
|---|---|
| 接入的 Coding Agent | 網頁上按「中斷」，或在那台電腦執行 `crewconvo disconnect` |
| Windows 桌面應用程式 | 從系統匣結束，刪除 `.exe` 與 `%LOCALAPPDATA%\CrewConvo` |
| 伺服器 | 停止程式，刪除安裝資料夾。Docker：`docker compose down -v` |

</details>

## 文件

| 文件 | 內容 |
|---|---|
| [docs/configuration.md](docs/configuration.md) | 所有設定項目 |
| [docs/a2a-provider.md](docs/a2a-provider.md) | A2A：目錄格式、呼叫內容、範例 |
| [docs/external-agents.md](docs/external-agents.md) | 自己實作 Agent：WebSocket、Webhook、長輪詢、REST、MCP |
| [docs/development.md](docs/development.md) | 內部架構、決策模型評測、測試方式 |
| [docs/live-scenarios.md](docs/live-scenarios.md) | 多個真實模型的情境測試紀錄 |

## 授權

[Apache License 2.0](LICENSE)
