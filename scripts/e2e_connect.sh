#!/bin/sh
# End-to-end check of `crewconvo connect` + bridge against a running server:
#   scripts/e2e_connect.sh http://127.0.0.1:8091 <admin-handle> <admin-password>
# Installs the CLI from this checkout into a throwaway venv with an isolated HOME (nothing on
# this machine is touched), approves the connection through the web API like a person would,
# runs the bridge, and checks that a mention gets answered.
# Optional: RESTART_SERVER_CMD="..." restarts the server mid-test to check that the bridge reconnects.
set -e
BASE="$1"; ADMIN="$2"; PASS="$3"
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
export HOME="$WORK/home" CREWCONVO_HOME="$WORK/home/.crewconvo"
mkdir -p "$HOME"
JAR="$WORK/cookies"
fail() { echo "== FAIL: $*"; [ -f "$WORK/bridge.out" ] && { echo "-- bridge log"; tail -20 "$WORK/bridge.out"; }; exit 1; }
echo "== workdir $WORK"

python3 -m venv "$WORK/venv"
"$WORK/venv/bin/pip" install -q "$ROOT/clients/python"
CC="$WORK/venv/bin/crewconvo"
"$CC" version

curl -sf -c "$JAR" -H 'content-type: application/json' -d "{\"handle\":\"$ADMIN\",\"password\":\"$PASS\"}" "$BASE/api/auth/login" >/dev/null

# 1. connect: the CLI prints a code, a person approves it in the browser (here: the same API call).
HANDLE="e2e-$(date +%s)"
"$CC" connect "$BASE" --no-browser --platform generic --no-service > "$WORK/connect.log" 2>&1 &
CPID=$!
CODE=""
for i in $(seq 1 40); do
  CODE=$(grep -oE '\b[A-Z2-9]{4}-[A-Z2-9]{4}\b' "$WORK/connect.log" | head -1 || true)
  [ -n "$CODE" ] && break; sleep 0.5
done
[ -n "$CODE" ] || { cat "$WORK/connect.log"; fail "no user code printed"; }
echo "== user code $CODE"
curl -sf -b "$JAR" "$BASE/api/connect/$CODE" | python3 -c 'import sys,json; d=json.load(sys.stdin); print("== approver sees:", d["client"]["hostname"], d["client"]["os"], d["status"])'
curl -sf -b "$JAR" -H 'content-type: application/json' \
  -d "{\"handle\":\"$HANDLE\",\"display_name\":\"E2E Bot\",\"description\":\"連線流程測試用的外部 Agent\",\"expertise\":[\"測試\"]}" \
  "$BASE/api/connect/$CODE/approve" >/dev/null
wait $CPID || { cat "$WORK/connect.log"; fail "connect exited with an error"; }
cat "$WORK/connect.log"
grep -q "已連上 Agent @$HANDLE" "$WORK/connect.log" || fail "connect did not finish"

# 2. bridge (what the background service runs), with a tiny agent program.
cat > "$WORK/agent.py" <<'PY'
import json, os, sys, urllib.request
p = json.load(sys.stdin)
trig = next((m for m in p["unread"] if m["id"] == p["trigger_message_id"]), {"body": "?"})
body = {"conversation_id": p["conversation"]["id"], "body": f"bridge 收到：「{trig['body'][:40]}」", "thread_root_id": p["thread_root_id"]}
req = urllib.request.Request(os.environ["CREWCONVO_SERVER"] + "/agent/v1/tools/post_message", data=json.dumps(body).encode(),
                             headers={"Authorization": "Bearer " + os.environ["CREWCONVO_TOKEN"], "Content-Type": "application/json"})
urllib.request.urlopen(req).read()
print("已回覆")
PY
"$CC" bridge --service --exec "python3 $WORK/agent.py" > "$WORK/bridge.out" 2>&1 &
BRIDGE=$!
trap 'kill $BRIDGE 2>/dev/null || true' EXIT

online() { curl -sf -H "Authorization: Bearer $(python3 -c "import json;print(json.load(open('$CREWCONVO_HOME/config.json'))['token'])")" "$BASE/agent/v1/me" | python3 -c 'import sys,json;print(json.load(sys.stdin)["status"])'; }
for i in $(seq 1 20); do [ "$(online)" = online ] && break; sleep 1; done
[ "$(online)" = online ] || fail "agent never came online"
echo "== agent online"

# Only one bridge per machine: a watchdog run exits at once, a manual run refuses.
timeout 10 "$CC" bridge --ensure || fail "--ensure did not exit while a bridge is running"
if timeout 10 "$CC" bridge >/dev/null 2>&1; then fail "a second foreground bridge started"; fi
echo "== single instance OK"
"$CC" status > "$WORK/status.txt" || true
cat "$WORK/status.txt"
grep -q "bridge 執行中、已連線" "$WORK/status.txt" || fail "status does not report a connected bridge"

# A new agent is in no channel: put it in one, as its owner would.
AID=$(python3 -c "import json;print(json.load(open('$CREWCONVO_HOME/config.json'))['agent_id'])")
GEN=$(curl -sf -b "$JAR" -H 'content-type: application/json' -d "{\"name\":\"connect-$(date +%s)\",\"invite\":[\"$AID\"]}" "$BASE/api/conversations" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
mention() {
  SEQ=$(curl -sf -b "$JAR" -H 'content-type: application/json' -d "{\"body\":\"@$HANDLE $1\"}" "$BASE/api/conversations/$GEN/messages" | python3 -c 'import sys,json;print(json.load(sys.stdin)["message"]["seq"])')
  R=""
  for i in $(seq 1 40); do
    R=$(curl -sf -b "$JAR" "$BASE/api/conversations/$GEN/messages?limit=10" | python3 -c "import sys,json;print(next((m['body'] for m in json.load(sys.stdin) if m['author_handle']=='$HANDLE' and m['seq']>$SEQ), ''))")
    [ -n "$R" ] && break; sleep 1
  done
  [ -n "$R" ] || fail "no reply to: $1"
  echo "== reply: $R"
}
mention "你收得到嗎？"

# 3. Optional: the server restarts; the bridge must reconnect by itself.
if [ -n "$RESTART_SERVER_CMD" ]; then
  echo "== restarting server"
  sh -c "$RESTART_SERVER_CMD"
  for i in $(seq 1 90); do curl -sf "$BASE/api/health" >/dev/null 2>&1 && break; sleep 1; done
  curl -sf -c "$JAR" -H 'content-type: application/json' -d "{\"handle\":\"$ADMIN\",\"password\":\"$PASS\"}" "$BASE/api/auth/login" >/dev/null
  for i in $(seq 1 60); do [ "$(online 2>/dev/null)" = online ] && break; sleep 1; done
  [ "$(online)" = online ] || fail "bridge did not reconnect after the server restart"
  echo "== bridge reconnected after restart"
  mention "伺服器重啟後還在嗎？"
fi

echo "-- bridge log"; tail -8 "$WORK/bridge.out"
echo "== PASS"
