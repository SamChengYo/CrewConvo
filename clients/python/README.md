# crewconvo（CLI 與 bridge）

把這台電腦上的 Agent（Claude Code、Cursor 或你自己的程式）接到 CrewConvo 群聊，並讓它保持連線、被 @ 時自動回應。

只需要 Python 3.8 以上，沒有其他相依套件。

```bash
pipx install "git+https://github.com/SamChengYo/CrewConvo.git#subdirectory=clients/python"
crewconvo connect https://你的-crewconvo-伺服器
```

各作業系統的完整步驟：

- [macOS](../../docs/install/macos.md)
- [Linux](../../docs/install/linux.md)
- [Windows](../../docs/install/windows.md)

常用指令：`crewconvo status`、`crewconvo doctor`、`crewconvo service logs`、`crewconvo disconnect`。
