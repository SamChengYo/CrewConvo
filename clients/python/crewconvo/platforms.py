"""Coding agents CrewConvo can be installed into: detection, MCP + Skill setup, headless runs.

Each platform knows
  detect()            is it installed on this computer?
  runner()            the command-line program that can run it without a window, or None
  install(server, token) / uninstall()
  command(prompt, tools)  how to run one wake → (argv, stdin text or None)

Sources for file locations and flags are the tools' official docs (checked 2026-11). Only the
Claude Code path is exercised by this project's end-to-end tests.
"""
import json
import os
import platform as _platform
import re
import shutil
import subprocess
from pathlib import Path

MCP_NAME = "crewconvo"
SKILL_SRC = Path(__file__).with_name("SKILL.md")
IS_WIN = _platform.system() == "Windows"
IS_MAC = _platform.system() == "Darwin"
HOME = Path.home()
BEGIN = "# >>> crewconvo (managed by `crewconvo setup`; edits here are overwritten) >>>"
END = "# <<< crewconvo <<<"


def which(*names):
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    return None


def _write(path, text, private=True):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".crewconvo-tmp")
    tmp.write_text(text, encoding="utf-8")
    if private:
        try:
            os.chmod(tmp, 0o600)
        except OSError:
            pass
    os.replace(tmp, path)


def _load_json(path):
    """Returns (data, ok). ok is False when the file exists but cannot be read as JSON."""
    if not path.exists():
        return {}, True
    raw = path.read_text(encoding="utf-8")
    if not raw.strip():
        return {}, True
    try:
        data = json.loads(raw)
    except ValueError:
        try:  # JSON with // comments or trailing commas
            stripped = re.sub(r"^\s*//.*$", "", raw, flags=re.M)
            data = json.loads(re.sub(r",(\s*[}\]])", r"\1", stripped))
        except ValueError:
            return {}, False
    return (data, True) if isinstance(data, dict) else ({}, False)


def _install_skill(dirpath):
    target = dirpath / "crewconvo" / "SKILL.md"
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(SKILL_SRC, target)
    return target


def _remove_skill(dirpath):
    shutil.rmtree(dirpath / "crewconvo", ignore_errors=True)


def _json_set(path, keys, value):
    """Sets data[keys...] = value in a JSON file. Returns False if the file is not plain JSON."""
    data, ok = _load_json(path)
    if not ok:
        return False
    node = data
    for k in keys[:-1]:
        if not isinstance(node.get(k), dict):
            node[k] = {}
        node = node[k]
    node[keys[-1]] = value
    _write(path, json.dumps(data, indent=2, ensure_ascii=False) + "\n")
    return True


def _json_del(path, keys):
    data, ok = _load_json(path)
    if not ok or not path.exists():
        return
    node = data
    for k in keys[:-1]:
        node = node.get(k) if isinstance(node, dict) else None
        if not isinstance(node, dict):
            return
    if node.pop(keys[-1], None) is not None:
        _write(path, json.dumps(data, indent=2, ensure_ascii=False) + "\n")


def _json_allow(path, keys, rule):
    """Adds `rule` to the list at data[keys...] (permission allow-lists)."""
    data, ok = _load_json(path)
    if not ok:
        return False
    node = data
    for k in keys[:-1]:
        if not isinstance(node.get(k), dict):
            node[k] = {}
        node = node[k]
    lst = node.get(keys[-1])
    if not isinstance(lst, list):
        lst = node[keys[-1]] = []
    if rule not in lst:
        lst.append(rule)
        _write(path, json.dumps(data, indent=2, ensure_ascii=False) + "\n")
    return True


def _json_disallow(path, keys, rule):
    data, ok = _load_json(path)
    if not ok or not path.exists():
        return
    node = data
    for k in keys:
        node = node.get(k) if isinstance(node, dict) else None
    if isinstance(node, list) and rule in node:
        node.remove(rule)
        _write(path, json.dumps(data, indent=2, ensure_ascii=False) + "\n")


class Result:
    def __init__(self, ok, lines):
        self.ok = ok
        self.lines = lines  # human-readable, already localized


class Platform:
    id = ""
    name = ""
    short = ""  # used in suggested handles
    description = ""

    def detect(self):
        raise NotImplementedError

    def runner(self):
        return None

    def install(self, server, token):
        raise NotImplementedError

    def uninstall(self):
        pass

    def command(self, prompt, tools):
        return None, None

    def wake_note(self):
        return ""


class ClaudeCode(Platform):
    id, name, short = "claude-code", "Claude Code", "claude"
    description = "Claude Code（Anthropic 的 coding agent）"

    def detect(self):
        return bool(which("claude"))

    def runner(self):
        return which("claude")

    def install(self, server, token):
        exe = which("claude")
        if not exe:
            return Result(False, ["找不到 claude 指令"])
        subprocess.run([exe, "mcp", "remove", MCP_NAME, "-s", "user"], capture_output=True)
        r = subprocess.run(
            [exe, "mcp", "add", "--transport", "http", "--scope", "user", MCP_NAME, f"{server}/mcp",
             "--header", f"Authorization: Bearer {token}"], capture_output=True, text=True)
        if r.returncode != 0:
            return Result(False, [f"claude mcp add 失敗：{(r.stderr or r.stdout).strip()[:300]}"])
        skill = _install_skill(HOME / ".claude" / "skills")
        return Result(True, [f"MCP server「{MCP_NAME}」已加入（claude mcp add --scope user）", f"Skill：{skill}"])

    def uninstall(self):
        exe = which("claude")
        if exe:
            subprocess.run([exe, "mcp", "remove", MCP_NAME, "-s", "user"], capture_output=True)
        _remove_skill(HOME / ".claude" / "skills")

    def command(self, prompt, tools):
        allowed = ",".join(f"mcp__{MCP_NAME}__{t}" for t in tools) or f"mcp__{MCP_NAME}"
        return [self.runner(), "-p", "--allowedTools", allowed, "--output-format", "text"], prompt


class Codex(Platform):
    id, name, short = "codex", "Codex", "codex"
    description = "OpenAI Codex CLI"

    def _dir(self):
        return Path(os.environ.get("CODEX_HOME") or HOME / ".codex")

    def detect(self):
        return bool(which("codex")) or self._dir().exists()

    def runner(self):
        return which("codex")

    def _strip(self, text):
        pat = re.compile(re.escape(BEGIN) + r".*?" + re.escape(END) + r"\n?", re.S)
        return pat.sub("", text)

    def install(self, server, token):
        path = self._dir() / "config.toml"
        text = self._strip(path.read_text(encoding="utf-8")) if path.exists() else ""
        if re.search(r"^\s*\[mcp_servers\." + MCP_NAME + r"\]", text, re.M):
            return Result(False, [f"{path} 已經有你自己寫的 [mcp_servers.{MCP_NAME}]，請先移除它再重跑"])
        block = (
            f"{BEGIN}\n[mcp_servers.{MCP_NAME}]\nurl = \"{server}/mcp\"\n"
            f"http_headers = {{ \"Authorization\" = \"Bearer {token}\" }}\n"
            "default_tools_approval_mode = \"approve\"\n"
            f"{END}\n"
        )
        _write(path, (text.rstrip("\n") + "\n\n" if text.strip() else "") + block)
        skill = _install_skill(HOME / ".agents" / "skills")
        return Result(True, [f"MCP server 已寫入 {path}", f"Skill：{skill}"])

    def uninstall(self):
        path = self._dir() / "config.toml"
        if path.exists():
            text = path.read_text(encoding="utf-8")
            new = self._strip(text)
            if new != text:
                _write(path, new)
        _remove_skill(HOME / ".agents" / "skills")

    def command(self, prompt, tools):
        # `-` reads the whole prompt from stdin; the default sandbox stays read-only.
        return [self.runner(), "exec", "--skip-git-repo-check", "-"], prompt


class Cursor(Platform):
    id, name, short = "cursor", "Cursor", "cursor"
    description = "Cursor 編輯器與 Cursor CLI"

    def detect(self):
        return (HOME / ".cursor").exists() or bool(which("cursor", "cursor-agent"))

    def runner(self):
        exe = which("cursor-agent")
        if exe:
            return exe
        # The CLI is now called `agent`; only trust that generic name when Cursor is installed.
        return which("agent") if (HOME / ".cursor").exists() else None

    def install(self, server, token):
        path = HOME / ".cursor" / "mcp.json"
        entry = {"url": f"{server}/mcp", "headers": {"Authorization": f"Bearer {token}"}}
        if not _json_set(path, ["mcpServers", MCP_NAME], entry):
            return Result(False, [f"{path} 不是有效的 JSON，請手動加入 MCP server"])
        skill = _install_skill(HOME / ".cursor" / "skills")
        lines = [f"MCP server 已寫入 {path}", f"Skill：{skill}"]
        if _json_allow(HOME / ".cursor" / "cli-config.json", ["permissions", "allow"], f"Mcp({MCP_NAME}:*)"):
            lines.append("Cursor CLI：已允許 crewconvo 的 MCP 工具（~/.cursor/cli-config.json）")
        return Result(True, lines)

    def uninstall(self):
        _json_del(HOME / ".cursor" / "mcp.json", ["mcpServers", MCP_NAME])
        _json_disallow(HOME / ".cursor" / "cli-config.json", ["permissions", "allow"], f"Mcp({MCP_NAME}:*)")
        _remove_skill(HOME / ".cursor" / "skills")

    def command(self, prompt, tools):
        return [self.runner(), "-p", prompt, "--approve-mcps", "--output-format", "text"], None

    def wake_note(self):
        return "需要已登入的 Cursor CLI（agent）；用 CURSOR_API_KEY 或先互動登入一次"


class OpenCode(Platform):
    id, name, short = "opencode", "OpenCode", "opencode"
    description = "OpenCode（opencode.ai）"

    def _dir(self):
        return Path(os.environ.get("OPENCODE_CONFIG_DIR") or HOME / ".config" / "opencode")

    def _file(self):
        d = self._dir()
        return d / "opencode.jsonc" if (d / "opencode.jsonc").exists() else d / "opencode.json"

    def detect(self):
        return bool(which("opencode")) or self._dir().exists()

    def runner(self):
        return which("opencode")

    def install(self, server, token):
        path = self._file()
        data, ok = _load_json(path)
        if not ok:
            return Result(False, [f"{path} 無法解析，請手動加入 MCP server（見 README）"])
        header = {"Authorization": f"Bearer {token}"}
        mcp = data.get("mcp") if isinstance(data.get("mcp"), dict) else {}
        v2 = isinstance(mcp.get("servers"), dict)
        exe = which("opencode")
        if not v2 and exe and not mcp:
            # V2 has a non-interactive `mcp add`; V1's is interactive and fails without a terminal.
            try:
                r = subprocess.run(
                    [exe, "mcp", "add", MCP_NAME, "--url", f"{server}/mcp", "--header",
                     f"Authorization=Bearer {token}", "--global"],
                    capture_output=True, text=True, timeout=20, stdin=subprocess.DEVNULL)
                if r.returncode == 0:
                    skill = _install_skill(self._dir() / "skills")
                    return Result(True, ["MCP server 已加入（opencode mcp add --global）", f"Skill：{skill}"])
            except (subprocess.TimeoutExpired, OSError):
                pass
        if v2:
            entry = {"type": "remote", "url": f"{server}/mcp", "oauth": False, "headers": header}
            _json_set(path, ["mcp", "servers", MCP_NAME], entry)
        else:
            entry = {"type": "remote", "url": f"{server}/mcp", "enabled": True, "oauth": False, "headers": header}
            _json_set(path, ["mcp", MCP_NAME], entry)
        skill = _install_skill(self._dir() / "skills")
        return Result(True, [f"MCP server 已寫入 {path}（{'V2' if v2 else 'V1'} 格式）", f"Skill：{skill}"])

    def uninstall(self):
        path = self._file()
        _json_del(path, ["mcp", "servers", MCP_NAME])
        _json_del(path, ["mcp", MCP_NAME])
        _remove_skill(self._dir() / "skills")

    def command(self, prompt, tools):
        return [self.runner(), "run", prompt], None


class Antigravity(Platform):
    id, name, short = "antigravity", "Google Antigravity", "antigravity"
    description = "Google Antigravity（App、IDE 與 agy CLI）"

    def _cfg(self):
        return HOME / ".gemini" / "config"

    def detect(self):
        if which("agy", "antigravity") or self._cfg().exists() or (HOME / ".gemini" / "antigravity").exists():
            return True
        apps = []
        if IS_WIN:
            apps.append(Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Antigravity")
        if IS_MAC:
            apps.append(Path("/Applications/Antigravity.app"))
        return any(str(a) not in ("", ".") and a.exists() for a in apps)

    def runner(self):
        return which("agy")

    def install(self, server, token):
        path = self._cfg() / "mcp_config.json"
        entry = {"serverUrl": f"{server}/mcp", "headers": {"Authorization": f"Bearer {token}"}}
        if not _json_set(path, ["mcpServers", MCP_NAME], entry):
            return Result(False, [f"{path} 不是有效的 JSON，請手動加入 MCP server"])
        skill = _install_skill(self._cfg() / "skills")
        lines = [f"MCP server 已寫入 {path}", f"Skill：{skill}"]
        settings = HOME / ".gemini" / "antigravity-cli" / "settings.json"
        if self.runner() and _json_allow(settings, ["permissions", "allow"], f"mcp({MCP_NAME}/*)"):
            lines.append(f"agy CLI：已允許 crewconvo 的 MCP 工具（{settings}）")
        return Result(True, lines)

    def uninstall(self):
        _json_del(self._cfg() / "mcp_config.json", ["mcpServers", MCP_NAME])
        _json_disallow(HOME / ".gemini" / "antigravity-cli" / "settings.json", ["permissions", "allow"],
                       f"mcp({MCP_NAME}/*)")
        _remove_skill(self._cfg() / "skills")

    def command(self, prompt, tools):
        return [self.runner(), "-p", prompt, "--output-format", "text"], None

    def wake_note(self):
        return "需要 agy CLI 並先互動登入一次"


PLATFORMS = [ClaudeCode(), Codex(), Cursor(), OpenCode(), Antigravity()]
BY_ID = {p.id: p for p in PLATFORMS}


def get(platform_id):
    return BY_ID.get(platform_id)
