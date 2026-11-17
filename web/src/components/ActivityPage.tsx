import { useCallback, useEffect, useState } from 'react'
import { api3, type WakeActivity } from '../api'
import { useRealtime } from '../realtime'
import { Avatar } from './Avatar'
import AgentView from './AgentView'

const REASON: Record<string, string> = {
  mention: '被 tag',
  dm: '私訊',
  thread_reply: '討論串回覆',
  sticky: '延續對話',
  decider: '決策模型',
  discussion_turn: '討論輪次',
}

const STATUS: Record<string, string> = {
  pending: '排隊中',
  delivered: '已送達',
  running: '處理中',
  done: '完成',
  timeout: '逾時',
  failed: '失敗',
  cancelled: '已取消',
}

function duration(a: WakeActivity) {
  if (!a.started_at) return '—'
  const end = a.finished_at ? new Date(a.finished_at).getTime() : Date.now()
  const s = Math.max(0, Math.round((end - new Date(a.started_at).getTime()) / 1000))
  return s >= 60 ? `${Math.floor(s / 60)}分${s % 60}秒` : `${s} 秒`
}

/** Live feed of agent wakes in the user's conversations (who, why, status, tokens, time). */
export default function ActivityPage({ onOpen, onError }: { onOpen: (conversationId: string) => void; onError: (m: string) => void }) {
  const [rows, setRows] = useState<WakeActivity[]>([])
  const [view, setView] = useState<WakeActivity | null>(null)
  const [more, setMore] = useState(true)

  const reload = useCallback(() => {
    api3
      .activity()
      .then((r) => {
        setRows(r)
        setMore(r.length === 60)
      })
      .catch((e) => onError(String(e)))
  }, [onError])

  useEffect(() => {
    reload()
    const t = setInterval(reload, 10_000)
    return () => clearInterval(t)
  }, [reload])

  useRealtime((ev) => {
    if (ev.type === 'wake_status') reload()
  })

  const older = async () => {
    const last = rows[rows.length - 1]
    if (!last) return
    const r = await api3.activity(last.created_at)
    setRows((x) => [...x, ...r])
    setMore(r.length === 60)
  }

  const tokens = rows.reduce((s, r) => s + r.tokens_used, 0)
  const active = rows.filter((r) => ['pending', 'delivered', 'running'].includes(r.status)).length

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>Agent 動態</h1>
          <p className="muted">每一次喚醒：誰、為什麼、做了什麼、花了多少。點「Agent 視角」可以看到 Agent 當時收到的完整內容。</p>
        </div>
        <div className="activity-stats">
          <div>
            <strong>{active}</strong>
            <span>進行中</span>
          </div>
          <div>
            <strong>{rows.length}</strong>
            <span>最近喚醒</span>
          </div>
          <div>
            <strong>{tokens.toLocaleString()}</strong>
            <span>tokens</span>
          </div>
        </div>
      </header>
      <div className="activity-list">
        {rows.map((r) => (
          <div key={r.id} className={`activity-row ${r.status}`}>
            <Avatar name={r.agent_display_name} kind="agent" size={32} />
            <div className="activity-main">
              <div className="activity-line">
                <strong>{r.agent_display_name}</strong>
                <span className={`wake-status ${r.status}`}>{STATUS[r.status] ?? r.status}</span>
                <span className="muted small">
                  {REASON[r.reason] ?? r.reason} · {r.mode} · 深度 {r.depth}
                </span>
              </div>
              <div className="activity-trigger">
                <button className="link" onClick={() => onOpen(r.conversation_id)}>
                  {r.conversation_kind === 'channel' ? `#${r.conversation_name}` : '私訊'}
                </button>
                {r.trigger_excerpt && (
                  <span className="muted">
                    ・@{r.trigger_author_handle}：{r.trigger_excerpt}
                  </span>
                )}
              </div>
              {r.private_log && <div className="activity-log">📝 {r.private_log}</div>}
            </div>
            <div className="activity-side">
              <div className="small muted">{new Date(r.created_at).toLocaleTimeString('zh-TW', { hour: 'numeric', minute: '2-digit', second: '2-digit' })}</div>
              <div className="small">
                發言 {r.posted_count} · {r.tokens_used.toLocaleString()} tok · {duration(r)}
              </div>
              <button className="ghost small" onClick={() => setView(r)}>
                Agent 視角
              </button>
            </div>
          </div>
        ))}
        {rows.length === 0 && <div className="empty-hint">還沒有任何喚醒。</div>}
        {more && rows.length > 0 && (
          <button className="load-older" onClick={older}>
            載入更早的動態
          </button>
        )}
      </div>
      {view && <AgentView wakeId={view.id} agentHandle={view.agent_handle} onClose={() => setView(null)} />}
    </div>
  )
}
