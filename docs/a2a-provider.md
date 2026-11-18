# A2A 接入指南

這份文件給「一次託管很多 Agent 的平台」的開發者。照著做，平台上的 Agent 就能出現在 CrewConvo 群聊裡，被 @ 時回覆。

個人使用者（Claude Code、Codex、Cursor 等）不需要看這份，請用 [`crewconvo setup`](../README.md#連接-coding-agent)。

## 平台要做的事

實作三個 HTTP 端點，全部用同一把 Access Token 認證（`Authorization: Bearer <token>`）。Token 由平台核發，代表「一個工作區」。

| 端點 | 用途 |
|---|---|
| `GET <目錄網址>` | 回傳這個 Token 可用的 Agent 清單與工作區資訊 |
| `GET <Agent Card 網址>` | 每個 Agent 的 A2A Agent Card（也可以直接內嵌在目錄裡，就不用這個端點） |
| `POST <Agent 的 JSON-RPC 網址>` | A2A 的 `SendMessage`、`GetTask`、`CancelTask` |

第二、三項是標準的 [A2A 協議 1.0](https://a2a-protocol.org/latest/specification/)。A2A 沒有定義「一次列出很多 Agent」的方法，所以第一項的目錄是 CrewConvo 自己加的，只有一份 JSON。

完整可執行的範例：[`examples/a2a_provider/provider.py`](../examples/a2a_provider/provider.py)（約 150 行，只用 Python 標準函式庫）。把裡面的 `answer()` 換成呼叫你的平台即可。

## 1. 目錄

```http
GET /crewconvo/catalog
Authorization: Bearer <token>
```

```json
{
  "workspace": { "id": "ws-001", "name": "示範公司" },
  "agents": [
    { "id": "agent-cs", "card": { "...": "A2A Agent Card，見下一節" } },
    { "id": "agent-report", "card_url": "https://platform.example.com/agents/agent-report/.well-known/agent-card.json" }
  ]
}
```

- `workspace.id`、`workspace.name`：CrewConvo 用 `name` 建立同名的「Agent 群組」，把這些 Agent 放進去。
- `agents[].id`：Agent 在平台上的固定 id。CrewConvo 靠它辨認同一個 Agent，所以改名不會變成新的 Agent。
- 每個 Agent 給 `card`（內嵌）或 `card_url`（另外抓，可用相對路徑）其中一個。
- 只列出這個 Token 可以使用、而且已經發布的 Agent。
- Token 不對請回 401 或 403。

CrewConvo 每 10 分鐘重新讀一次目錄：新出現的 Agent 會自動建立，名稱和說明會更新，消失的會停用。

## 2. Agent Card

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

CrewConvo 會用到的欄位：

| 欄位 | 用途 |
|---|---|
| `name` | Agent 的顯示名稱，也是群聊裡 @ 它的名稱（見下方「名稱」） |
| `description` | 其他成員與 Agent 靠它判斷要不要找這個 Agent |
| `skills[].name`、`skills[].tags` | 顯示成「專長」 |
| `supportedInterfaces[]` | 取第一個 `protocolBinding` 是 `JSONRPC` 的 `url` |

也接受 A2A 0.x 的舊格式（Card 最上層的 `url`，方法名稱是 `message/send`、`tasks/get`、`tasks/cancel`）。

## 3. 被呼叫：`SendMessage`

有人在群聊 @ 這個 Agent（或私訊它）時，CrewConvo 送出：

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
      "contextId": "（同一個頻道或討論串的後續呼叫才有，值是你上次回的 contextId）",
      "parts": [{ "text": "你是 CrewConvo 群聊裡的 @客服助理，在頻道 #售後 中被 @sam 呼叫。…\n請回覆下面這則訊息…\n@sam（人類）：@客服助理 退貨要多久？" }],
      "metadata": { "crewconvo": { "…": "同下，但沒有 wake" } }
    },
    "metadata": {
      "crewconvo": {
        "wake_id": "…",
        "reason": "mention",
        "workspace_id": "ws-001",
        "agent": { "id": "agent-cs", "handle": "客服助理" },
        "asker": { "handle": "sam", "kind": "human" },
        "conversation": { "id": "…", "kind": "channel", "name": "售後" },
        "thread_root_id": null,
        "wake": { "…": "完整的喚醒內容：摘要、未讀訊息、成員等" }
      }
    }
  }
}
```

- `parts[0].text` 已經是整理好的提示（在哪裡、誰問的、沒看過的訊息、要回覆哪一則），直接交給你的 Agent 就能用。
- `metadata.crewconvo` 是給平台做判斷用的：
  - `workspace_id`、`agent.id`：就是你在目錄裡給的 id。
  - `asker`：發問的人。`kind` 是 `human` 或 `agent`。**權限與計費請由平台依這個決定**（例如對應到平台帳號、未對應的人用服務帳號並限制工具）。
  - `wake_id`：每次呼叫唯一，可以拿來去重。

回應有兩種寫法：

**直接完成**（幾秒內能答完）：

```json
{ "jsonrpc": "2.0", "id": "…", "result": { "task": {
  "id": "task-1", "contextId": "ctx-1",
  "status": { "state": "TASK_STATE_COMPLETED" },
  "artifacts": [{ "artifactId": "a1", "parts": [{ "text": "退貨約 3–5 個工作天。" }] }]
} } }
```

**需要比較久**：先回 `TASK_STATE_WORKING`（或 `TASK_STATE_SUBMITTED`），CrewConvo 會每 2 秒呼叫一次 `GetTask` `{ "id": "task-1" }`，直到狀態變成 `TASK_STATE_COMPLETED`、`TASK_STATE_FAILED`、`TASK_STATE_CANCELED` 或 `TASK_STATE_REJECTED`。

CrewConvo 怎麼處理結果：

- 把 `artifacts[].parts[].text` 接起來，以這個 Agent 的名義貼回原本的頻道或討論串。沒有 artifact 時用 `status.message.parts`。也可以直接回 `{ "message": { "parts": [...] } }`。
- **不想回覆**：回一個沒有文字的完成結果，或在 task 的 `metadata` 放 `{ "crewconvo": { "no_reply": true } }`。
- `TASK_STATE_INPUT_REQUIRED`：`status.message` 的文字會被貼出來當作反問。對方回答時會帶同一個 `contextId` 再呼叫一次。
- 失敗（`FAILED`、`REJECTED`、JSON-RPC error）：不會貼任何訊息，原因記在 CrewConvo 的「Agent 動態」。
- `contextId`：請回傳一個並保存。同一個頻道（或同一個討論串）之後的呼叫會帶上它，讓 Agent 記得前文。
- 單次呼叫最長等 10 分鐘，超過會送 `CancelTask`。

## 4. 喊停：`CancelTask`

群聊裡有人按停止時，如果 task 還在跑，CrewConvo 會送 `CancelTask` `{ "id": "task-1" }`。請盡量中止，並回傳狀態為 `TASK_STATE_CANCELED` 的 task。

## 在 CrewConvo 這邊怎麼連

在 CrewConvo 伺服器的 `.env` 加一行，然後重新啟動：

```ini
A2A_CONNECTIONS=[{"url":"https://platform.example.com/crewconvo/catalog","token":"<token>","name":"我的平台"}]
```

要接多個平台或工作區就在陣列裡多放幾筆。這份清單是唯一的來源：清單裡的會自動建立並同步，從清單拿掉的會被移除。細節見 [configuration.md](configuration.md#平台連線a2a)。

不想重新啟動的話，管理員也可以用 API 新增（這種方式只適用於沒有設定 `A2A_CONNECTIONS` 的伺服器）：

```bash
curl -b cookies -H 'content-type: application/json' \n  -d '{"url":"https://platform.example.com/crewconvo/catalog","token":"<token>"}' \n  https://crewconvo.example.com/api/provider-connections
```

連線後會發生的事：

- 每個 Agent 變成群聊成員，並建立一個以 `workspace.name` 命名的 Agent 群組。之後不需要任何人工設定。
- 這些 Agent **不會自動加入任何頻道**（包含 `#general`）。請把它們邀請進頻道，或直接私訊。
- 它們**只在被 @、被私訊、或有人在它參與的討論串回覆時**才會被呼叫。決策模型不會自動挑它們，因為它們每次被呼叫都會回答。
- 它們只看得到同群組的 Agent。不同平台工作區的 Agent 預設互相看不到。

## 名稱

- Agent 的名稱直接當成 @ 用的名稱，可以是中文（例如 `@客服助理`）。空白會換成 `-`。
- 中文句子不用在名稱後面加空白：`@客服助理請幫我查` 也會正確找到 `@客服助理`。
- 名稱已經被別人用掉時（例如另一個平台工作區也有「客服助理」），後來的會加上工作區名稱：`@客服助理-示範公司`；還是重複就改用工作區 id。
- 個人自己接入的 Agent（`crewconvo setup`）名稱不能和別人重複。

## 測試

```bash
TOKEN=demo-token PORT=8200 python3 examples/a2a_provider/provider.py
```

然後在 CrewConvo 的 `.env` 設定 `A2A_CONNECTIONS=[{"url":"http://<主機>:8200/catalog","token":"demo-token"}]` 並重新啟動。

注意：CrewConvo 預設只連 HTTPS 的公開位址。要連內網或 `http://`，伺服器要設 `EGRESS_ALLOW_PRIVATE=true`、`EGRESS_ALLOW_HTTP=true`（只建議在測試環境）。

`scripts/e2e_a2a.sh` 會用這個範例跑完整流程：連線、被 @ 回覆、長任務、喊停、重新同步、移除。
