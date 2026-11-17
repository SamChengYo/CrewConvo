import { useEffect, useState } from 'react'
import { api3 } from '../api'
import { Modal } from './Modals'
import Markdown from './Markdown'

interface PMsg {
  id: string
  seq: number
  author_handle: string
  author_kind: string
  body: string
  thread_root_id: string | null
}

interface Payload {
  reason?: string
  mode?: string
  conversation?: { name: string | null; kind: string; members: { handle: string; kind: string }[] }
  summary?: string | null
  recent?: PMsg[]
  unread?: PMsg[]
  unread_omitted?: number
  previous_private_logs?: string[] | string
  discussion?: { phase: string; round: number; max_rounds: number; participants: string[]; instruction: string } | null
  also_woken?: string[]
  limits?: { depth: number; max_depth: number; deadline_seconds: number }
  trigger_message_id?: string
  thread_root?: PMsg | null
}

function Msgs({ items, trigger }: { items: PMsg[]; trigger?: string }) {
  return (
    <div className="av-msgs">
      {items.map((m) => (
        <div key={m.id} className={`av-msg ${m.id === trigger ? 'trigger' : ''}`}>
          <span className="av-seq">#{m.seq}</span>
          <strong>@{m.author_handle}</strong>
          {m.author_kind === 'agent' && <span className="app-badge">AI</span>}
          {m.thread_root_id && <span className="muted small"> （討論串）</span>}
          <div className="av-body">
            <Markdown text={m.body.length > 600 ? m.body.slice(0, 600) + '…' : m.body} />
          </div>
        </div>
      ))}
    </div>
  )
}

/** "What did the agent see?" — the exact payload delivered for one wake. */
export default function AgentView({ wakeId, agentHandle, onClose }: { wakeId: string; agentHandle: string; onClose: () => void }) {
  const [p, setP] = useState<Payload | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [raw, setRaw] = useState(false)
  useEffect(() => {
    api3
      .wakePayload(wakeId)
      .then((x) => setP(x as Payload))
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
  }, [wakeId])
  return (
    <Modal title={`@${agentHandle} 看到了什麼`} onClose={onClose}>
      {error && <div className="form-error">{error}</div>}
      {!p && !error && <p className="muted">載入中…</p>}
      {p && (
        <div className="agent-view">
          <div className="av-facts">
            <span>原因 <code>{p.reason}</code></span>
            <span>模式 <code>{p.mode}</code></span>
            {p.limits && (
              <span>
                深度 {p.limits.depth}/{p.limits.max_depth} · 期限 {p.limits.deadline_seconds}s
              </span>
            )}
            {p.also_woken && p.also_woken.length > 0 && <span>同時被喚醒：{p.also_woken.map((h) => '@' + h).join('、')}</span>}
            <button className="link small" onClick={() => setRaw(!raw)}>
              {raw ? '整理過的檢視' : '原始 JSON'}
            </button>
          </div>
          {raw ? (
            <pre className="md-pre av-raw">{JSON.stringify(p, null, 2)}</pre>
          ) : (
            <>
              {p.discussion && (
                <section>
                  <h3>討論指示（{p.discussion.phase === 'conclusion' ? '寫結論' : `第 ${p.discussion.round}/${p.discussion.max_rounds} 輪`}）</h3>
                  <p className="av-note">{p.discussion.instruction}</p>
                </section>
              )}
              <section>
                <h3>對話摘要</h3>
                {p.summary ? <p className="av-note">{p.summary}</p> : <p className="muted small">（沒有摘要：對話還不夠長）</p>}
              </section>
              {p.thread_root && (
                <section>
                  <h3>討論串根訊息</h3>
                  <Msgs items={[p.thread_root]} />
                </section>
              )}
              <section>
                <h3>已讀的近期訊息（{p.recent?.length ?? 0}）</h3>
                {p.recent?.length ? <Msgs items={p.recent} /> : <p className="muted small">（無）</p>}
              </section>
              <section>
                <h3>
                  未讀訊息（{p.unread?.length ?? 0}
                  {p.unread_omitted ? `，另有 ${p.unread_omitted} 則因預算省略` : ''}）
                </h3>
                {p.unread?.length ? <Msgs items={p.unread} trigger={p.trigger_message_id} /> : <p className="muted small">（無）</p>}
              </section>
              <section>
                <h3>上次留下的私人紀錄</h3>
                {typeof p.previous_private_logs === 'string' ? (
                  <p className="muted small">{p.previous_private_logs}</p>
                ) : p.previous_private_logs?.length ? (
                  p.previous_private_logs.map((l, i) => (
                    <p key={i} className="av-note">
                      {l}
                    </p>
                  ))
                ) : (
                  <p className="muted small">（無）</p>
                )}
              </section>
            </>
          )}
        </div>
      )}
    </Modal>
  )
}
