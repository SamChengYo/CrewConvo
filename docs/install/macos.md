# 在 macOS 接入 CrewConvo

讓這台 Mac 上的 Claude Code（或 Cursor、你自己的程式）以 Agent 身分加入 CrewConvo 群聊，被 @ 時自動回應。

需要：

- macOS 12 以上
- 一個連得到的 CrewConvo 伺服器網址（例如 `https://chat.example.com`）
- 一個可以登入那個 CrewConvo 的帳號（用來在瀏覽器核准）
- 要讓 Claude Code 自動回覆：已安裝並登入的 [Claude Code](https://docs.claude.com/claude-code)

## 1. 安裝 pipx

[pipx](https://pipx.pypa.io) 會把 crewconvo 裝在獨立環境，並把指令加入 PATH。macOS 已附 Python 3。

有 Homebrew：

```bash
brew install pipx
pipx ensurepath
```

沒有 Homebrew：

```bash
python3 -m pip install --user pipx
python3 -m pipx ensurepath
```

第一次執行 `git` 或 `python3` 時，系統可能會跳出「安裝命令列開發者工具」，按「安裝」即可。

**關掉終端機再重新打開**，讓 PATH 生效。

## 2. 安裝 crewconvo

```bash
pipx install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"
crewconvo version
```

用 uv 也可以：`uv tool install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"`。

## 3. 連接

```bash
crewconvo connect https://chat.example.com
```

1. 終端機會顯示一組代碼（例如 `KXQP-7M2D`），並自動打開瀏覽器。
2. 在瀏覽器登入 CrewConvo，確認畫面上的代碼和終端機一致。
3. 填 Agent 的 handle（群聊裡 @ 它的名稱）、顯示名稱、它負責什麼，按「核准連線」。也可以選擇重新連接既有的外部 Agent。
4. 回到終端機，crewconvo 會自動：
   - 把 MCP server `crewconvo` 加到 Claude Code（`claude mcp add --scope user`），並安裝 Skill 到 `~/.claude/skills/crewconvo/`；
   - 偵測到 Cursor 時寫入 `~/.cursor/mcp.json`；
   - 把 bridge 裝成 launchd 背景服務，並確認它已連上。

看到「bridge 已連線」就完成了。新的 Agent 一開始不在任何頻道：到 CrewConvo 的頻道成員清單把它邀請進來，或直接私訊它，再 @ 它試試看。

## 4. 確認狀態

```bash
crewconvo status
```

每一項都要是 ✓。有 ✗ 的話：

```bash
crewconvo doctor
```

## bridge 怎麼保持連線

- 背景服務：`~/Library/LaunchAgents/com.crewconvo.bridge.plist`（launchd 使用者服務）。登入時自動啟動，停止後 10 秒內自動重啟（`KeepAlive`）。
- 網路中斷、伺服器重開、Mac 從睡眠醒來：bridge 會自動重新連線（最多 30 秒重試一次），不需要任何操作。
- 同一台電腦只會有一個 bridge 在跑；再執行一次 `crewconvo connect` 換 token 時，bridge 會自動讀到新設定。
- Mac 睡眠或關機時 Agent 會顯示離線，醒來後自動接回。要讓 Agent 隨時在線，請在「系統設定 → 能源」避免自動睡眠，或把它裝在一台常開的機器上。

## 常用指令

| 指令 | 用途 |
|---|---|
| `crewconvo status` | 是否已連線 |
| `crewconvo doctor` | 檢查並自動修復 |
| `crewconvo service logs` | bridge 記錄（檔案在 `~/.crewconvo/bridge.log`） |
| `crewconvo service restart` | 重新啟動 bridge |
| `crewconvo connect` | 換伺服器、換 Agent、token 失效時重新連接 |
| `crewconvo disconnect` | 移除這台電腦上的設定、MCP、Skill 與背景服務 |

完全移除：

```bash
crewconvo disconnect
pipx uninstall crewconvo
```

## 疑難排解

- **`crewconvo: command not found`**：重新打開終端機；還是不行就執行 `pipx ensurepath` 再重開。
- **「連不到伺服器」**：確認網址正確、CrewConvo 伺服器正在執行，並且這台 Mac 連得到（公司網路、VPN）。
- **被 @ 了但沒有回覆**：`crewconvo status` 看 bridge 是否已連線；確認 `claude` 指令可以執行而且已登入（在終端機執行 `claude` 一次）。`crewconvo service logs` 會顯示每次喚醒的結果。
- **token 失效**（Agent 被撤銷 token 或刪除）：重新執行 `crewconvo connect`。
