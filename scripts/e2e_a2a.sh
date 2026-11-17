#!/bin/sh
# End-to-end check of platform connections (A2A) against a running server, using the example
# provider in examples/a2a_provider:
#   scripts/e2e_a2a.sh http://127.0.0.1:8091 <admin-handle> <admin-password>
# The server must allow private addresses for this local test (EGRESS_ALLOW_PRIVATE=true, EGRESS_ALLOW_HTTP=true).
set -e
BASE="$1"; ADMIN="$2"; PASS="$3"
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d); JAR="$WORK/cookies"; PORT=${PROVIDER_PORT:-8201}; TS=$(date +%s)
fail() { echo "== FAIL: $*"; echo "-- provider log"; tail -15 "$WORK/provider.log"; exit 1; }
j() { python3 -c "import sys,json; d=json.load(sys.stdin); print($1)"; }
api() { m="$1"; p="$2"; shift 2; curl -s -b "$JAR" -X "$m" -H 'content-type: application/json' "$@" "$BASE$p"; }

TOKEN="tok-$TS" PORT=$PORT WORKSPACE_ID="ws-$TS" WORKSPACE_NAME="示範公司$TS" python3 "$ROOT/examples/a2a_provider/provider.py" > "$WORK/provider.log" 2>&1 &
PROV=$!
trap 'kill $PROV 2>/dev/null || true' EXIT
sleep 1
curl -sf -c "$JAR" -H 'content-type: application/json' -d "{\"handle\":\"$ADMIN\",\"password\":\"$PASS\"}" "$BASE/api/auth/login" >/dev/null
URL="http://127.0.0.1:$PORT/catalog"

# 1. A wrong token is rejected and nothing is stored.
BEFORE=$(api GET /api/provider-connections | j 'len(d)')
R=$(api POST /api/provider-connections -d "{\"url\":\"$URL\",\"token\":\"wrong\"}")
echo "$R" | grep -q "error" || fail "wrong token accepted: $R"
[ "$(api GET /api/provider-connections | j 'len(d)')" = "$BEFORE" ] || fail "failed connection was kept"
echo "== wrong token rejected"

# 2. Connect: agents and the group appear without any manual step.
R=$(api POST /api/provider-connections -d "{\"url\":\"$URL\",\"token\":\"tok-$TS\"}")
CONN=$(echo "$R" | j 'd["connection"]["id"]') || fail "connect: $R"
GROUP=$(echo "$R" | j 'd["connection"]["group_id"]')
H1=$(echo "$R" | j 'd["created"][0]'); H2=$(echo "$R" | j 'd["created"][1]')
echo "== connected: agents @$H1 @$H2, workspace $(echo "$R" | j 'd["connection"]["external_workspace_name"]')"
api GET /api/agent-groups | j "[print('== group:', g['name'], len(g['agent_ids']), 'agents') for g in d if g['id']=='$GROUP'] and ''"
IDS=$(api GET /api/agents | j "json.dumps([a['member']['id'] for a in d if a.get('provider_connection_id')=='$CONN' and not a['member'].get('deactivated_at')])")
[ "$(echo "$IDS" | j 'len(d)')" = 2 ] || fail "expected 2 agents, got $IDS"

CH=$(api POST /api/conversations -d "{\"name\":\"a2a-$TS\",\"invite\":$IDS}" | j 'd["id"]')
say() { api POST "/api/conversations/$CH/messages" -d "{\"body\":\"$1\"}" | j 'd["message"]["seq"]'; }
reply_after() { # seq, handle, seconds
  for i in $(seq 1 "$3"); do
    X=$(api GET "/api/conversations/$CH/messages?limit=20" | j "next((m['body'] for m in d if m['author_handle']=='$2' and m['seq']>$1), '')")
    [ -n "$X" ] && { echo "$X"; return 0; }; sleep 1
  done; return 1
}

# 3. A mention glued to Chinese text wakes the agent; the reply is posted as the agent.
S=$(say "@${H1}請問退貨流程")
X=$(reply_after "$S" "$H1" 30) || fail "no reply from @$H1"
echo "== @$H1 replied: $X"
# 4. A long-running task is polled until it completes.
S=$(say "@$H2 本月銷售如何")
X=$(reply_after "$S" "$H2" 40) || fail "no reply from @$H2 (slow task)"
echo "== @$H2 replied: $X"
# 5. Without a mention, platform agents are never picked by the decision model.
S=$(say "大家午安，今天天氣不錯")
sleep 8
N=$(api GET "/api/conversations/$CH/messages?limit=20" | j "len([m for m in d if m['seq']>$S and m['author_handle'] in ('$H1','$H2')])")
[ "$N" = 0 ] || fail "platform agent answered an untagged message"
echo "== untagged message left alone"
# 6. Stop cancels the running A2A task.
say "@$H2 再跑一次" >/dev/null; sleep 2
api POST "/api/conversations/$CH/stop" >/dev/null
for i in 1 2 3 4 5; do grep -q "canceled" "$WORK/provider.log" && break; sleep 1; done
grep -q "canceled" "$WORK/provider.log" || fail "stop did not cancel the A2A task"
echo "== stop sent CancelTask"
# 7. Re-sync is idempotent.
R=$(api POST "/api/provider-connections/$CONN/sync")
[ "$(echo "$R" | j 'len(d["created"])')" = 0 ] && [ "$(echo "$R" | j 'd["agents"]')" = 2 ] || fail "re-sync: $R"
echo "== re-sync: nothing new"
# 8. Removing the connection retires its agents and group.
api DELETE "/api/provider-connections/$CONN" >/dev/null
LEFT=$(api GET /api/agents | j "len([a for a in d if a['member']['handle'] in ('$H1','$H2') and not a['member'].get('deactivated_at')])")
[ "$LEFT" = 0 ] || fail "agents still active after removing the connection"
[ "$(api GET /api/agent-groups | j "len([g for g in d if g['id']=='$GROUP'])")" = 0 ] || fail "group not removed"
echo "-- provider log"; cat "$WORK/provider.log"
echo "== PASS"
