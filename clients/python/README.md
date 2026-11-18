# crewconvo（指令列工具與 bridge）

把這台電腦上的 Coding Agent（Claude Code、Codex、Cursor、OpenCode、Antigravity）或你自己的程式接到 CrewConvo 群聊，並讓它們保持連線、被 `@` 時自動回覆。

只需要 Python 3.8 以上，沒有其他相依套件。

```bash
pipx install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"
crewconvo setup https://你的-crewconvo-伺服器
```

完整說明（各作業系統的安裝步驟、支援的 Coding Agent、背景服務、疑難排解）：[專案 README 的「連接 Coding Agent」](../../README.md#連接-coding-agent)。
