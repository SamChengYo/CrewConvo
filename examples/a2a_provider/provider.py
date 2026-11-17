#!/usr/bin/env python3
"""A tiny "agent provider" for CrewConvo — a reference for platforms that host many agents.

It implements exactly what CrewConvo needs (Python 3.8+, standard library only):

  GET  /catalog                                   list this workspace's agents (CrewConvo's one addition to A2A)
  GET  /agents/<id>/.well-known/agent-card.json   the agent's A2A Agent Card
  POST /agents/<id>                               A2A 1.0 JSON-RPC: SendMessage, GetTask, CancelTask

Every request must carry `Authorization: Bearer <TOKEN>`. Run it, then in CrewConvo open
「接入其他平台 → 平台連線」 and enter http://<this host>:8200/catalog with the token.

    TOKEN=demo-token PORT=8200 python3 provider.py

Replace `answer()` with a call into your own platform. Everything else is protocol plumbing.
"""
import json
import os
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(os.environ.get("PORT", "8200"))
TOKEN = os.environ.get("TOKEN", "demo-token")
PUBLIC_URL = os.environ.get("PUBLIC_URL", f"http://127.0.0.1:{PORT}").rstrip("/")

# The workspace this access token belongs to, and the agents it may use.
WORKSPACE = {"id": os.environ.get("WORKSPACE_ID", "ws-demo-001"), "name": os.environ.get("WORKSPACE_NAME", "示範公司")}
AGENTS = {
    "agent-cs": {"name": "客服助理", "description": "回答訂單、退貨與出貨進度的問題", "skills": ["訂單查詢", "退貨"]},
    "agent-report": {"name": "報表分析", "description": "整理銷售數字並產生摘要（需要幾秒鐘）", "skills": ["銷售報表"], "slow": 4},
}

TASKS = {}  # task id -> Task object
LOCK = threading.Lock()


def card(agent_id):
    a = AGENTS[agent_id]
    return {
        "name": a["name"],
        "description": a["description"],
        "version": "1.0.0",
        "supportedInterfaces": [
            {"url": f"{PUBLIC_URL}/agents/{agent_id}", "protocolBinding": "JSONRPC", "protocolVersion": "1.0"}
        ],
        "capabilities": {"streaming": False, "pushNotifications": False},
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [{"id": f"s{i}", "name": s, "description": s, "tags": []} for i, s in enumerate(a["skills"])],
        "securitySchemes": {"bearer": {"httpAuthSecurityScheme": {"scheme": "Bearer"}}},
    }


def answer(agent_id, text, meta):
    """Your platform goes here: run the agent and return its reply (or None to stay silent).

    `meta` is `params.metadata.crewconvo`: who asked (`asker.handle`), where (`conversation`),
    which workspace / agent (`workspace_id`, `agent.id`) and the full wake payload (`wake`).
    Use it to decide permissions and billing.
    """
    asker = (meta.get("asker") or {}).get("handle", "?")
    question = text.strip().splitlines()[-1]
    return f"（{AGENTS[agent_id]['name']}）收到 @{asker} 的訊息：{question}"


def task_obj(task_id, context_id, state, text=None):
    t = {"id": task_id, "contextId": context_id, "status": {"state": state, "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}}
    if text is not None:
        t["artifacts"] = [{"artifactId": str(uuid.uuid4()), "parts": [{"text": text}]}]
    return t


def send_message(agent_id, params):
    msg = params.get("message") or {}
    text = "\n".join(p.get("text", "") for p in msg.get("parts", []) if "text" in p)
    meta = (params.get("metadata") or {}).get("crewconvo") or (msg.get("metadata") or {}).get("crewconvo") or {}
    context_id = msg.get("contextId") or str(uuid.uuid4())  # keep it to remember earlier turns
    task_id = str(uuid.uuid4())
    print(f"[provider] {agent_id} <- @{(meta.get('asker') or {}).get('handle')} in {(meta.get('conversation') or {}).get('name')}"
          f" (workspace {meta.get('workspace_id')}, wake {meta.get('wake_id')})", flush=True)
    slow = AGENTS[agent_id].get("slow")
    if not slow:
        reply = answer(agent_id, text, meta)
        if reply is None:
            return {"task": dict(task_obj(task_id, context_id, "TASK_STATE_COMPLETED"), metadata={"crewconvo": {"no_reply": True}})}
        return {"task": task_obj(task_id, context_id, "TASK_STATE_COMPLETED", reply)}

    # Long-running: answer "working" now; CrewConvo polls GetTask until the task is final.
    with LOCK:
        TASKS[task_id] = task_obj(task_id, context_id, "TASK_STATE_WORKING")

    def work():
        time.sleep(slow)
        reply = answer(agent_id, text, meta)
        with LOCK:
            if TASKS[task_id]["status"]["state"] == "TASK_STATE_WORKING":
                TASKS[task_id] = task_obj(task_id, context_id, "TASK_STATE_COMPLETED", reply)

    threading.Thread(target=work, daemon=True).start()
    return {"task": TASKS[task_id]}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, status, body):
        data = json.dumps(body, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def authorized(self):
        if self.headers.get("Authorization") == f"Bearer {TOKEN}":
            return True
        self.reply(401, {"error": "invalid access token"})
        return False

    def do_GET(self):
        if not self.authorized():
            return
        parts = self.path.split("?")[0].strip("/").split("/")
        if parts == ["catalog"]:
            return self.reply(200, {"workspace": WORKSPACE, "agents": [{"id": i, "card": card(i)} for i in AGENTS]})
        if len(parts) == 4 and parts[0] == "agents" and parts[1] in AGENTS and parts[2:] == [".well-known", "agent-card.json"]:
            return self.reply(200, card(parts[1]))
        self.reply(404, {"error": "not found"})

    def do_POST(self):
        if not self.authorized():
            return
        parts = self.path.strip("/").split("/")
        if len(parts) != 2 or parts[0] != "agents" or parts[1] not in AGENTS:
            return self.reply(404, {"error": "not found"})
        req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))) or b"{}")
        method, params, rid = req.get("method"), req.get("params") or {}, req.get("id")

        def ok(result):
            self.reply(200, {"jsonrpc": "2.0", "id": rid, "result": result})

        def err(code, message):
            self.reply(200, {"jsonrpc": "2.0", "id": rid, "error": {"code": code, "message": message}})

        if method == "SendMessage":
            return ok(send_message(parts[1], params))
        if method in ("GetTask", "CancelTask"):
            with LOCK:
                task = TASKS.get(params.get("id"))
                if task is None:
                    return err(-32001, "Task not found")
                if method == "CancelTask":
                    if task["status"]["state"] != "TASK_STATE_WORKING":
                        return err(-32002, "Task cannot be canceled")
                    task = TASKS[task["id"]] = task_obj(task["id"], task["contextId"], "TASK_STATE_CANCELED")
                    print(f"[provider] task {task['id']} canceled", flush=True)
            return ok(task)
        err(-32601, f"Method not found: {method}")


if __name__ == "__main__":
    print(f"[provider] catalog: {PUBLIC_URL}/catalog   token: {TOKEN}", flush=True)
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
