#!/usr/bin/env python3
"""Live scenario run against a CrewConvo server with real models.

Usage: python3 scripts/live_scenarios.py --base http://127.0.0.1:8090 --handle admin --password ... [--setup]
Writes docs/live-scenarios.md. Stdlib only.
"""
import argparse
import http.cookiejar
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

P = argparse.ArgumentParser()
P.add_argument("--base", default="http://127.0.0.1:8090")
P.add_argument("--handle", required=True)
P.add_argument("--password", required=True)
P.add_argument("--setup", action="store_true")
P.add_argument("--out", default="docs/live-scenarios.md")
P.add_argument("--only", default="")
ARGS = P.parse_args()

jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))


def call(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(ARGS.base + path, data=data, method=method)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    try:
        with opener.open(req, timeout=60) as r:
            t = r.read().decode()
            return json.loads(t) if t else None
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"{method} {path} -> {e.code} {e.read().decode()[:300]}")


AGENTS = [
    {
        "handle": "atlas",
        "display_name": "Atlas · Claude Sonnet 5.5",
        "model": "anthropic:claude-sonnet-5-5",
        "description": "後端架構師兼資料庫專家：系統架構設計、資料庫效能調校、SQL 與索引、程式碼審查",
        "expertise": ["系統架構", "PostgreSQL", "索引", "程式碼審查", "Python"],
        "system_prompt": "你是資深後端架構師。回答精簡、具體、可執行，用繁體中文。",
    },
    {
        "handle": "nova",
        "display_name": "Nova · GPT-6 Luna",
        "model": "openai:gpt-6-luna",
        "description": "維運／SRE：部署、Kubernetes、CI/CD、監控告警、事故處理與容量規劃",
        "expertise": ["Kubernetes", "部署", "監控", "事故處理", "雲端成本"],
        "system_prompt": "你是資深 SRE。回答務實，優先考慮穩定性與可觀測性，用繁體中文。",
    },
    {
        "handle": "iris",
        "display_name": "Iris · Gemini 4 Hydrogen",
        "model": "gemini:gemini-4-hydrogen",
        "description": "產品設計師：UI/UX 設計、使用者研究、介面文案與設計系統",
        "expertise": ["UI", "UX", "文案", "設計系統", "無障礙"],
        "system_prompt": "你是資深產品設計師，重視使用者體驗與可及性。回答簡潔，用繁體中文。",
    },
]


def login():
    try:
        call("POST", "/api/auth/login", {"handle": ARGS.handle, "password": ARGS.password})
    except RuntimeError:
        call("POST", "/api/auth/register", {"handle": ARGS.handle, "display_name": "Admin", "password": ARGS.password})
    return call("GET", "/api/me")


def conversations():
    return call("GET", "/api/conversations")


def channel(name, topic, member_ids):
    for c in conversations():
        if c["kind"] == "channel" and c["name"] == name:
            if member_ids:
                call("POST", f"/api/conversations/{c['id']}/invite", {"member_ids": member_ids})
            return c["id"]
    return call("POST", "/api/conversations", {"name": name, "topic": topic, "visibility": "public", "invite": member_ids})["id"]


def setup():
    agents = {a["member"]["handle"]: a for a in call("GET", "/api/agents")}
    for spec in AGENTS:
        if spec["handle"] not in agents:
            a = call("POST", "/api/agents", {**spec, "tools": [], "max_wake_seconds": 300})
            agents[spec["handle"]] = a
    ids = {h: a["member"]["id"] for h, a in agents.items()}
    general = channel("general", "全體成員的預設頻道", [ids["atlas"], ids["nova"], ids["iris"]])
    eng = channel("engineering", "後端、維運與架構討論", [ids["atlas"], ids["nova"], ids["iris"]])
    design = channel("design", "設計與文案", [ids["iris"], ids["atlas"]])
    ops = channel("ops", "維運值班與事故", [ids["atlas"], ids["nova"], ids["iris"]])
    arch = channel("architecture", "架構決策討論", [ids["atlas"], ids["nova"], ids["iris"]])
    ux = channel("product-ux", "產品體驗與介面", [ids["atlas"], ids["nova"], ids["iris"]])
    return ids, {"general": general, "engineering": eng, "design": design, "ops": ops, "architecture": arch, "ux": ux}


def messages(conv, thread=None, limit=100):
    q = f"?limit={limit}" + (f"&thread_root_id={thread}" if thread else "")
    return call("GET", f"/api/conversations/{conv}/messages{q}")


def all_after(conv, seq):
    """Main timeline + threads of recent roots, newer than seq."""
    out = []
    main = messages(conv)
    for m in main:
        if m["seq"] > seq:
            out.append(m)
        if m["reply_count"] > 0 and m["seq"] >= seq - 40:
            for t in messages(conv, m["id"]):
                if t["seq"] > seq and t["id"] != m["id"]:
                    out.append(t)
    seen, uniq = set(), []
    for m in sorted(out, key=lambda m: m["seq"]):
        if m["id"] not in seen:
            seen.add(m["id"])
            uniq.append(m)
    return uniq


def settle(conv, timeout=420, min_wait=6):
    start = time.time()
    time.sleep(min_wait)
    quiet = 0
    while time.time() - start < timeout:
        d = call("GET", f"/api/conversations/{conv}")
        if not d["active_wakes"]:
            quiet += 1
            if quiet >= 3:
                return time.time() - start
        else:
            quiet = 0
        time.sleep(2)
    return time.time() - start


def post(conv, body, thread=None):
    return call("POST", f"/api/conversations/{conv}/messages", {"body": body, "thread_root_id": thread})["message"]


def explain(mid):
    return call("GET", f"/api/messages/{mid}/wakes")


REPORT = []


def record(title, expectation, conv, msg, elapsed, verdict_fn=None, extra=""):
    after = all_after(conv, msg["seq"] - 1)
    ex = explain(msg["id"])
    lines = [f"### {title}", "", f"**預期**：{expectation}", ""]
    for d in ex["decisions"]:
        scores = ""
        if d.get("scores"):
            scores = "，分數 " + "、".join(f"@{h} {s:.2f}" for h, s in sorted(d["scores"].items(), key=lambda x: -x[1]))
        woken = "、".join("@" + ex["agent_handles"].get(w, w[:8]) for w in d["woken"]) or "無"
        lines.append(f"- 判斷：{d['source']} / {d.get('reason')}，模式 {d.get('mode')}，喚醒 {woken}{scores}"
                     + (f"，{d['latency_ms']} ms" if d.get("source") == "decider" else ""))
    for w in ex["wakes"]:
        lines.append(f"- Wake @{w['agent_handle']}：{w['reason']} / {w['mode']} → {w['status']}，發言 {w['posted_count']}，tokens {w['tokens_used']}")
    lines.append(f"- 耗時約 {elapsed:.0f} 秒")
    lines.append("")
    lines.append("對話：")
    lines.append("")
    for m in after:
        body = m["body"].replace("\n", " ")
        if len(body) > 400:
            body = body[:400] + "…"
        where = "（thread）" if m["thread_root_id"] else ""
        react = ""
        if m["reactions"]:
            react = " " + " ".join(f"[{r['emoji']} {','.join(r['handles'])}]" for r in m["reactions"])
        lines.append(f"> **@{m['author_handle']}**{where}：{body}{react}")
        lines.append(">")
    verdict = verdict_fn(after, ex) if verdict_fn else None
    if verdict is not None:
        ok, why = verdict
        lines.append("")
        lines.append(f"**結果**：{'✅' if ok else '⚠️'} {why}")
    if extra:
        lines.append("")
        lines.append(extra)
    lines.append("")
    REPORT.append("\n".join(lines))
    print(f"[{title}] {'OK' if (verdict or (True,))[0] else 'CHECK'} ({elapsed:.0f}s)", flush=True)
    return after, ex


def agents_spoke(after):
    return {m["author_handle"] for m in after if m["author_kind"] == "agent"}


def main():
    me = login()
    print("logged in as", me["handle"], "admin" if me["is_admin"] else "")
    ids, ch = setup()
    eng, design, general = ch["engineering"], ch["design"], ch["general"]
    ops, arch, ux = ch["ops"], ch["architecture"], ch["ux"]
    root = None
    only = set(ARGS.only.split(",")) if ARGS.only else None
    run = lambda k: only is None or k in only

    if run("1"):
        m = post(eng, "@atlas 我們 orders 表有 2 億筆，查「某使用者最近 30 天的訂單」很慢，索引該怎麼設計？")
        t = settle(eng)
        r1 = record("1. 單一 tag（Claude）", "@atlas 被喚醒並回答（它若主動 tag 別人，對方也會被喚醒）", eng, m, t,
                    lambda a, e: ("atlas" in agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}"))
        root = m
    if run("2"):
        m = post(eng, "@nova @iris 下週要上線新版結帳頁，請各自列出你們負責的上線前檢查項目（各 3 點）")
        t = settle(eng)
        record("2. 同時 tag 兩位（parallel：GPT + Gemini）", "@nova 與 @iris 各自回答，模式 parallel", eng, m, t,
               lambda a, e: ({"nova", "iris"} <= agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}"))
    if run("3"):
        m = post(ops, "Kubernetes 的 pod 一直被 OOMKilled，記憶體 limit 和 request 應該怎麼設比較好？")
        t = settle(ops)
        record("3. 沒有 tag，交給決策模型", "決策模型挑出 @nova（維運）", ops, m, t,
               lambda a, e: ("nova" in agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}"))
    if run("4"):
        m = post(ops, "那 JVM 服務的話，heap 要怎麼跟 limit 搭配？")
        t = settle(ops)
        record("4. 緊接著追問（sticky）", "同一位 Agent（剛回答的 @nova）延續回答，不需要再 tag", ops, m, t,
               lambda a, e: ("nova" in agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}；喚醒原因：{[w['reason'] for w in e['wakes']]}"))
    if run("5"):
        m = post(general, "大家午安！今天天氣真好 ☀️")
        t = settle(general)
        record("5. 閒聊", "決策模型判斷不喚醒任何 Agent", general, m, t,
               lambda a, e: (not agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a)) or '無'}"))
    if run("6"):
        if root is None:
            root = next(x for x in messages(eng, limit=100) if x["body"].startswith("@atlas 我們 orders 表"))
        m = post(eng, "那如果查詢條件是 user_id 加 created_at 範圍，再依 created_at 排序取前 50 筆呢？", thread=root["id"])
        t = settle(eng)
        record("6. 在 thread 中追問（thread_reply）", "thread 根訊息 tag 的 @atlas 在 thread 中回答", eng, m, t,
               lambda a, e: (any(x["author_handle"] == "atlas" and x["thread_root_id"] for x in a), "atlas 在 thread 中回覆" if any(x["author_handle"] == "atlas" and x["thread_root_id"] for x in a) else "未在 thread 中回覆"))
    if run("7"):
        m = post(arch, "新產品後端要用單體架構還是微服務？請架構和維運兩位互相討論利弊，最後給一個共識結論。")
        t = settle(arch, timeout=900)
        record("7. 多 Agent 討論（discussion：Claude × GPT）", "決策模型判斷需要討論，Agent 在 thread 中輪流發言，最後有結論貼回主對話", arch, m, t,
               lambda a, e: (any(d.get("mode") == "discussion" for d in e["decisions"]), f"模式：{[d.get('mode') for d in e['decisions']]}，{len([x for x in a if x['author_kind']=='agent'])} 則 Agent 訊息"))
    if run("16"):
        m = post(ux, "結帳頁的按鈕顏色對比度太低，色弱的使用者幾乎看不出哪個是主要按鈕，該怎麼改？")
        t = settle(ux)
        record("16. 沒有 tag，交給決策模型（設計問題）", "決策模型挑出 @iris（設計）", ux, m, t,
               lambda a, e: ("iris" in agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}"))
    if run("8"):
        dm = call("POST", "/api/dms", {"member_ids": [ids["iris"]]})["id"]
        m = post(dm, "幫我想三個「深色模式設定」頁面的標題文案，要溫和不生硬")
        t = settle(dm)
        record("8. 私訊（DM：Gemini）", "@iris 直接回覆", dm, m, t,
               lambda a, e: ("iris" in agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}"))
    if run("9"):
        m = post(design, "@iris 新的 onboarding 設計稿我已經放到 Figma 了，你有空再看，不用回覆我")
        t = settle(design)
        record("9. 告知型訊息（可以只 react）", "@iris 被喚醒，最好只按表情、不發言", design, m, t,
               lambda a, e: (True, f"發言 {sum(w['posted_count'] for w in e['wakes'])} 則；表情：{[r['emoji'] for x in messages(design) if x['id']==m['id'] for r in x['reactions']]}"))
    if run("10"):
        m = post(eng, "@atlas 請你直接 @nova 確認：資料庫每晚 2 點的備份，會不會跟維運的監控或部署排程衝突？")
        t = settle(eng)
        record("10. Agent 互相 tag（因果深度）", "@atlas tag @nova，@nova 回覆；深度遞增，不會無限循環", eng, m, t,
               lambda a, e: ({"atlas", "nova"} <= agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}；深度：{sorted({x['depth'] for x in a if x['author_kind']=='agent'})}"))
    if run("11"):
        m = post(eng, "@atlas 用搜尋找一下我們剛剛討論過的 OOMKilled，整理成一句話的結論")
        t = settle(eng)
        record("11. 搜尋舊訊息（search_messages）", "@atlas 呼叫 search_messages 找到先前的討論並總結", eng, m, t,
               lambda a, e: ("atlas" in agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}"))
    if run("12"):
        m = post(eng, "@atlas 請在你的沙箱寫一個 Python 腳本存成 /workspace/fib.py，算出費氏數列第 90 項並執行，回報結果")
        t = settle(eng, timeout=600)
        record("12. 沙箱（Claude 寫程式並執行）", "回報 2880067194370816120", eng, m, t,
               lambda a, e: (any("2880067194370816120" in x["body"].replace(",", "") for x in a), "結果正確" if any("2880067194370816120" in x["body"].replace(",", "") for x in a) else "未看到正確數字"))
    if run("13"):
        m = post(eng, "@nova 在你的沙箱用 bash 查一下 python 版本、CPU 核心數和記憶體上限，回報給我")
        t = settle(eng, timeout=600)
        record("13. 沙箱（GPT 用 bash）", "@nova 執行指令並回報（應該看到 1 核、512MB 上限、沒有網路）", eng, m, t,
               lambda a, e: ("nova" in agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}"))
    if run("14"):
        m = post(design, "@iris 請在沙箱的 /workspace/memory/principles.md 寫下你的五條設計原則，寫完告訴我檔名")
        t = settle(design, timeout=600)
        record("14. 沙箱長期筆記（Gemini 寫檔）", "@iris 用 write_file 寫入 /workspace/memory", design, m, t,
               lambda a, e: ("iris" in agents_spoke(a), f"發言的 Agent：{sorted(agents_spoke(a))}"))
    if run("15"):
        m = post(eng, "@atlas @nova 請兩位詳細比較 Redis、Memcached、PostgreSQL unlogged table 當快取的差異，越詳細越好")
        time.sleep(4)
        stop = call("POST", f"/api/conversations/{eng}/stop")
        t = settle(eng, min_wait=3)
        after = all_after(eng, m["seq"] - 1)
        conv = call("GET", f"/api/conversations/{eng}")
        record("15. 喊停", "按下停止後，進行中的喚醒被取消、對話凍結，直到人類再發言", eng, m, t,
               lambda a, e: (stop["cancelled"] >= 1 and conv["frozen_until_human"], f"取消 {stop['cancelled']} 個喚醒；對話凍結={conv['frozen_until_human']}"))
        post(eng, "好，先到這裡，謝謝兩位。")  # unfreeze
        settle(eng)

    head = (
        "# 正式站情境測試\n\n"
        f"- 時間：{time.strftime('%Y-%m-%d %H:%M', time.gmtime())} UTC\n"
        "- Agent：@atlas（anthropic:claude-sonnet-5-5）、@nova（openai:gpt-6-luna）、@iris（gemini:gemini-4-hydrogen）\n"
        "- 決策模型：Gemini Decisions API（gemini-decision-1），後備 gpt-6-luna 結構化輸出\n\n"
    )
    Path(ARGS.out).write_text(head + "\n".join(REPORT), encoding="utf-8")
    print("report:", ARGS.out)


if __name__ == "__main__":
    main()
