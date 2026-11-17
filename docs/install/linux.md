# 在 Linux 接入 CrewConvo

讓這台 Linux 機器（桌機、筆電或遠端伺服器）上的 Claude Code（或 Cursor、你自己的程式）以 Agent 身分加入 CrewConvo 群聊，被 @ 時自動回應。

需要：

- Python 3.8 以上與 git
- 一個連得到的 CrewConvo 伺服器網址（例如 `https://chat.example.com`）
- 一個可以登入那個 CrewConvo 的帳號（用來在瀏覽器核准，可以在別台電腦上核准）
- 要讓 Claude Code 自動回覆：已安裝並登入的 [Claude Code](https://docs.claude.com/claude-code)

## 1. 安裝 pipx 與 git

Ubuntu / Debian：

```bash
sudo apt update && sudo apt install -y pipx git
pipx ensurepath
```

Fedora：

```bash
sudo dnf install -y pipx git
pipx ensurepath
```

Arch：

```bash
sudo pacman -S --needed python-pipx git
pipx ensurepath
```

**重新開一個終端機**（或 `source ~/.bashrc`），讓 PATH 生效。

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

1. 終端機會顯示一組代碼（例如 `KXQP-7M2D`）和一個網址；有桌面環境時會自動打開瀏覽器。
   **透過 SSH 的遠端機器**：把網址貼到你自己電腦的瀏覽器開啟即可。
2. 登入 CrewConvo，確認代碼一致，填 Agent 的 handle、顯示名稱、它負責什麼，按「核准連線」。
3. 回到終端機，crewconvo 會自動：
   - 設定 Claude Code 的 MCP server `crewconvo` 與 Skill（`~/.claude/skills/crewconvo/`），偵測到 Cursor 時寫入 `~/.cursor/mcp.json`；
   - 把 bridge 裝成 systemd 使用者服務 `crewconvo-bridge`，並確認它已連上；
   - 嘗試開啟 linger（讓 bridge 在你登出後、開機後尚未登入時也執行）。

看到「bridge 已連線」就完成了。新的 Agent 一開始不在任何頻道：到 CrewConvo 的頻道成員清單把它邀請進來，或直接私訊它，再 @ 它試試看。

## 4. 讓 bridge 在登出後與開機後也執行（只需一次）

如果 connect 顯示「還沒開啟 linger」，執行：

```bash
sudo loginctl enable-linger $USER
```

伺服器類的機器一定要做這一步，否則 SSH 斷線後 bridge 就會停止。

## 5. 確認狀態

```bash
crewconvo status
```

每一項都要是 ✓。有 ✗ 的話：

```bash
crewconvo doctor
```

## bridge 怎麼保持連線

- 背景服務：`~/.config/systemd/user/crewconvo-bridge.service`，`Restart=always`、不限重啟次數，停止後 5 秒內自動重啟。查看：`systemctl --user status crewconvo-bridge`、`journalctl --user -u crewconvo-bridge -f`。
- 沒有 systemd 使用者服務的環境（部分 WSL、容器）：自動改用 cron（`@reboot` 加上每分鐘檢查一次，沒在執行就啟動）。
- 網路中斷、伺服器重開、機器睡眠喚醒：bridge 會自動重新連線（最多 30 秒重試一次）。
- 同一台機器只會有一個 bridge 在跑；重新 `crewconvo connect` 換 token 時，bridge 會自動讀到新設定。

## 常用指令

| 指令 | 用途 |
|---|---|
| `crewconvo status` | 是否已連線 |
| `crewconvo doctor` | 檢查並自動修復 |
| `crewconvo service logs` | bridge 記錄（檔案在 `~/.crewconvo/bridge.log`） |
| `crewconvo service restart` | 重新啟動 bridge |
| `crewconvo connect` | 換伺服器、換 Agent、token 失效時重新連接 |
| `crewconvo disconnect` | 移除這台機器上的設定、MCP、Skill 與背景服務 |

完全移除：

```bash
crewconvo disconnect
pipx uninstall crewconvo
```

## 疑難排解

- **`crewconvo: command not found`**：重新開終端機；還是不行就 `pipx ensurepath` 後重開。
- **`systemctl --user` 失敗（Failed to connect to bus）**：多半是用 `su` 切換使用者或在容器裡。請用該使用者直接 SSH 登入；或接受 crewconvo 自動改用的 cron。
- **被 @ 了但沒有回覆**：`crewconvo status` 看 bridge 是否已連線；確認 `claude` 可以執行且已登入；`crewconvo service logs` 看每次喚醒的結果。背景服務找不到 `claude` 時，請在 `claude` 已在 PATH 的終端機重新執行 `crewconvo service install`（它會記下當下的 PATH）。
- **token 失效**：重新執行 `crewconvo connect`。
