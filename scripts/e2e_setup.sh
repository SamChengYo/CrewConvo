#!/bin/sh
# End-to-end check of `crewconvo setup` + the multi-agent bridge against a running server:
#   scripts/e2e_setup.sh http://127.0.0.1:8091 <handle> <password>
# Installs the CLI from this checkout into a throwaway venv with an isolated HOME (nothing on this
# computer is touched), sets up two coding agents (Claude Code and Codex — their config files are
# written, the programs themselves are not needed), approves once through the web API like a
# person would, and checks that the group, the channel and both agents work.
# Optional: RESTART_SERVER_CMD="..." restarts the server mid-test to check that the bridge reconnects.
set -e
BASE="$1"; ADMIN="$2"; PASS="$3"
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
export HOME="$WORK/home" CREWCONVO_HOME="$WORK/home/.crewconvo" CREWCONVO_NO_LINGER=1
mkdir -p "$HOME"
JAR="$WORK/cookies"; TS=$(date +%s)
fail() { echo "== FAIL: $*"; [ -f "$WORK/bridge.out" ] && { echo "-- bridge log"; tail -20 "$WORK/bridge.out"; }; exit 1; }
j() { python3 -c "import sys,json; d=json.load(sys.stdin); print($1)"; }
api() { m="$1"; p="$2"; shift 2; curl -s -b "$JAR" -X "$m" -H 'content-type: application/json' "$@" "$BASE$p"; }
echo "== workdir $WORK"

python3 -m venv "$WORK/venv"
"$WORK/venv/bin/pip" install -q "$ROOT/clients/python"
CC="$WORK/venv/bin/crewconvo"
"$CC" version
"$CC" agents || true

curl -sf -c "$JAR" -H 'content-type: application/json' -d "{\"handle\":\"$ADMIN\",\"password\":\"$PASS\"}" "$BASE/api/auth/login" >/dev/null

# 1. setup: the CLI prints a code; a person reviews the agents in the browser and approves once.
"$CC" setup "$BASE" --agents claude-code,codex --no-browser --no-service > "$WORK/setup.log" 2>&1 &
SPID=$!
CODE=""
for i in $(seq 1 40); do
  CODE=$(grep -oE '\b[A-Z2-9]{4}-[A-Z2-9]{4}\b' "$WORK/setup.log" | head -1 || true)
  [ -n "$CODE" ] && break; sleep 0.5
done
[ -n "$CODE" ] || { cat "$WORK/setup.log"; fail "no user code printed"; }
REQ=$(api GET "/api/connect/$CODE")
echo "== approver sees: $(echo "$REQ" | j '", ".join(a["name"] for a in d["client"]["agents"])') on $(echo "$REQ" | j 'd["client"]["hostname"]')"
BODY=$(echo "$REQ" | python3 -c "
import sys, json
c = json.load(sys.stdin)['client']
print(json.dumps({
  'agents': [{'platform': a['platform'], 'handle': a['platform'] + '-$TS', 'display_name': a['name'],
              'description': a['name'] + ' on ' + c['hostname']} for a in c['agents']],
  'group_name': 'e2e-$TS', 'channel_name': 'e2e-agents-$TS'}))")
RES=$(api POST "/api/connect/$CODE/approve" -d "$BODY")
CH=$(echo "$RES" | j 'd["channel"]["id"]') || fail "approve: $RES"
echo "== approved: group $(echo "$RES" | j 'd["group"]["name"]') ($(echo "$RES" | j 'len(d["group"]["agent_ids"])') agents), channel #$(echo "$RES" | j 'd["channel"]["name"]'), warnings $(echo "$RES" | j 'd["warnings"]')"
wait $SPID || { cat "$WORK/setup.log"; fail "setup exited with an error"; }
grep -E "→|✓|✗|!" "$WORK/setup.log"

# 2. Each coding agent got its own MCP entry and Skill.
grep -q '"crewconvo"' "$HOME/.claude.json" || fail "Claude Code MCP entry missing"
grep -q '^\[mcp_servers.crewconvo\]' "$HOME/.codex/config.toml" || fail "Codex MCP entry missing"
[ -f "$HOME/.claude/skills/crewconvo/SKILL.md" ] && [ -f "$HOME/.agents/skills/crewconvo/SKILL.md" ] || fail "Skill missing"
H1=$(python3 -c "import json;print(json.load(open('$CREWCONVO_HOME/config.json'))['agents'][0]['handle'])")
H2=$(python3 -c "import json;print(json.load(open('$CREWCONVO_HOME/config.json'))['agents'][1]['handle'])")
echo "== installed: @$H1 (Claude Code), @$H2 (Codex)"
# Re-running setup keeps user edits and replaces only its own block.
printf '\n[other]\nkeep = true\n' >> "$HOME/.codex/config.toml"

# 3. The bridge serves both agents (here with a tiny program instead of the real coding agents).
cat > "$WORK/agent.py" <<'PY'
import json, os, sys, urllib.request
p = json.load(sys.stdin)
trig = next((m for m in p["unread"] if m["id"] == p["trigger_message_id"]), {"body": "?"})
body = {"conversation_id": p["conversation"]["id"], "body": f"bridge 收到：「{trig['body'][:40]}」", "thread_root_id": p["thread_root_id"]}
req = urllib.request.Request(os.environ["CREWCONVO_SERVER"] + "/agent/v1/tools/post_message", data=json.dumps(body).encode(),
                             headers={"Authorization": "Bearer " + os.environ["CREWCONVO_TOKEN"], "Content-Type": "application/json"})
urllib.request.urlopen(req).read()
PY
"$CC" bridge --service --exec "python3 $WORK/agent.py" > "$WORK/bridge.out" 2>&1 &
BRIDGE=$!
trap 'kill $BRIDGE 2>/dev/null || true' EXIT
online() { api GET /api/agents | j "sum(1 for a in d if a['member']['handle'] in ('$H1','$H2') and a['status']=='online')"; }
for i in $(seq 1 20); do [ "$(online)" = 2 ] && break; sleep 1; done
[ "$(online)" = 2 ] || fail "both agents should be online"
echo "== both agents online"
timeout 10 "$CC" bridge --ensure || fail "--ensure did not exit while a bridge is running"
if timeout 10 "$CC" bridge >/dev/null 2>&1; then fail "a second foreground bridge started"; fi
echo "== single instance OK"

mention() { # handle, text
  SEQ=$(api POST "/api/conversations/$CH/messages" -d "{\"body\":\"@$1 $2\"}" | j 'd["message"]["seq"]')
  R=""
  for i in $(seq 1 40); do
    R=$(api GET "/api/conversations/$CH/messages?limit=10" | j "next((m['body'] for m in d if m['author_handle']=='$1' and m['seq']>$SEQ), '')")
    [ -n "$R" ] && break; sleep 1
  done
  [ -n "$R" ] || fail "no reply from @$1"
  echo "== @$1 replied: $R"
}
mention "$H1" "你收得到嗎？"
mention "$H2" "你呢？"

# 4. Optional: the server restarts; the bridge must reconnect by itself.
if [ -n "$RESTART_SERVER_CMD" ]; then
  echo "== restarting server"
  sh -c "$RESTART_SERVER_CMD"
  for i in $(seq 1 90); do curl -sf "$BASE/api/health" >/dev/null 2>&1 && break; sleep 1; done
  curl -sf -c "$JAR" -H 'content-type: application/json' -d "{\"handle\":\"$ADMIN\",\"password\":\"$PASS\"}" "$BASE/api/auth/login" >/dev/null
  for i in $(seq 1 60); do [ "$(online 2>/dev/null)" = 2 ] && break; sleep 1; done
  [ "$(online)" = 2 ] || fail "bridge did not reconnect after the server restart"
  echo "== bridge reconnected after restart"
  mention "$H2" "伺服器重啟後還在嗎？"
fi

# 5. disconnect removes what setup wrote, and nothing else.
kill $BRIDGE 2>/dev/null || true; sleep 1
"$CC" disconnect > "$WORK/disconnect.log" 2>&1 || { cat "$WORK/disconnect.log"; fail "disconnect failed"; }
grep -q 'mcp_servers.crewconvo' "$HOME/.codex/config.toml" && fail "Codex MCP entry not removed"
grep -q 'keep = true' "$HOME/.codex/config.toml" || fail "disconnect removed the user's own Codex settings"
[ -f "$HOME/.agents/skills/crewconvo/SKILL.md" ] && fail "Skill not removed"
echo "== disconnect cleaned up"
echo "== PASS"
