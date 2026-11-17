import { useEffect, useState, type ReactNode } from 'react'
import { api, api2, type Conversation, type Member, type MessageView, type WakeExplanation, type WakeWithAgent } from '../api'
import { Avatar } from './Avatar'
import AgentView from './AgentView'

export function Modal({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose()
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <header>
          <h2>{title}</h2>
          <button className="ghost" onClick={onClose}>
            ✕
          </button>
        </header>
        {children}
      </div>
    </div>
  )
}

function MemberPicker({
  members,
  selected,
  onToggle,
}: {
  members: Member[]
  selected: Set<string>
  onToggle: (id: string) => void
}) {
  const [q, setQ] = useState('')
  const shown = members.filter(
    (m) => m.handle.toLowerCase().includes(q.toLowerCase()) || m.display_name.toLowerCase().includes(q.toLowerCase()),
  )
  return (
    <div className="picker">
      <input placeholder="搜尋成員…" value={q} onChange={(e) => setQ(e.target.value)} />
      <ul>
        {shown.map((m) => (
          <li key={m.id} className={selected.has(m.id) ? 'selected' : ''} onClick={() => onToggle(m.id)}>
            <input type="checkbox" readOnly checked={selected.has(m.id)} />
            <Avatar name={m.display_name} kind={m.kind} size={22} />
            <span>{m.display_name}</span>
            <span className="muted small">@{m.handle}</span>
            {m.kind === 'agent' && <span className="badge">AI</span>}
          </li>
        ))}
        {shown.length === 0 && <li className="muted">沒有符合的成員</li>}
      </ul>
    </div>
  )
}

function useToggleSet() {
  const [set, setSet] = useState<Set<string>>(new Set())
  const toggle = (id: string) =>
    setSet((s) => {
      const n = new Set(s)
      if (n.has(id)) n.delete(id)
      else n.add(id)
      return n
    })
  return [set, toggle] as const
}

export function NewChannelModal({
  members,
  onClose,
  onCreated,
}: {
  members: Member[]
  onClose: () => void
  onCreated: (c: Conversation) => void
}) {
  const [name, setName] = useState('')
  const [topic, setTopic] = useState('')
  const [priv, setPriv] = useState(false)
  const [selected, toggle] = useToggleSet()
  const [error, setError] = useState<string | null>(null)
  const create = async () => {
    try {
      onCreated(await api.createChannel(name, topic, priv ? 'private' : 'public', [...selected]))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }
  return (
    <Modal title="建立頻道" onClose={onClose}>
      <label>
        名稱
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="例如 product-review" autoFocus />
      </label>
      <label>
        主題（選填）
        <input value={topic} onChange={(e) => setTopic(e.target.value)} />
      </label>
      <label className="row">
        <input type="checkbox" checked={priv} onChange={(e) => setPriv(e.target.checked)} /> 私有頻道（需邀請才能加入）
      </label>
      <div className="field-label">邀請成員</div>
      <MemberPicker members={members} selected={selected} onToggle={toggle} />
      {error && <div className="form-error">{error}</div>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" disabled={!name.trim()} onClick={create}>
          建立
        </button>
      </footer>
    </Modal>
  )
}

export function NewDmModal({
  members,
  onClose,
  onOpened,
}: {
  members: Member[]
  onClose: () => void
  onOpened: (c: Conversation) => void
}) {
  const [selected, toggle] = useToggleSet()
  const [error, setError] = useState<string | null>(null)
  const open = async () => {
    try {
      onOpened(await api.openDm([...selected]))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }
  return (
    <Modal title="開新私訊" onClose={onClose}>
      <p className="muted small">選一位或多位成員。私訊裡的 Agent 會在每則訊息被喚醒。</p>
      <MemberPicker members={members} selected={selected} onToggle={toggle} />
      {error && <div className="form-error">{error}</div>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" disabled={selected.size === 0} onClick={open}>
          開啟
        </button>
      </footer>
    </Modal>
  )
}

export function InviteModal({
  conversationId,
  candidates,
  onClose,
  onError,
}: {
  conversationId: string
  candidates: Member[]
  onClose: () => void
  onError: (e: unknown) => void
}) {
  const [selected, toggle] = useToggleSet()
  const invite = async () => {
    try {
      await api.invite(conversationId, [...selected])
      onClose()
    } catch (e) {
      onError(e)
    }
  }
  return (
    <Modal title="邀請成員" onClose={onClose}>
      <MemberPicker members={candidates} selected={selected} onToggle={toggle} />
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" disabled={selected.size === 0} onClick={invite}>
          邀請
        </button>
      </footer>
    </Modal>
  )
}

export function BrowseChannelsModal({
  joinedIds,
  onClose,
  onJoined,
}: {
  joinedIds: Set<string>
  onClose: () => void
  onJoined: (id: string) => void
}) {
  const [channels, setChannels] = useState<Conversation[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    api.publicChannels().then(setChannels).catch((e) => setError(String(e)))
  }, [])
  return (
    <Modal title="公開頻道" onClose={onClose}>
      {error && <div className="form-error">{error}</div>}
      {!channels && <p className="muted">載入中…</p>}
      <ul className="channel-list">
        {channels?.map((c) => (
          <li key={c.id}>
            <div>
              <strong># {c.name}</strong>
              {c.topic && <div className="muted small">{c.topic}</div>}
            </div>
            {joinedIds.has(c.id) ? (
              <button className="ghost" onClick={() => onJoined(c.id)}>
                開啟
              </button>
            ) : (
              <button
                className="primary"
                onClick={() =>
                  api
                    .join(c.id)
                    .then(() => onJoined(c.id))
                    .catch((e) => setError(String(e)))
                }
              >
                加入
              </button>
            )}
          </li>
        ))}
      </ul>
    </Modal>
  )
}

const REASON: Record<string, string> = {
  mention: '被 @ tag',
  dm: '私訊',
  thread_reply: 'thread 中最近發言的 Agent',
  sticky: '延續剛才的回覆（sticky）',
  decider: '決策模型判斷',
  discussion_turn: '討論輪到它',
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

const SOURCE: Record<string, string> = { rule: '規則層', decider: '決策模型', none: '—' }
const DECIDER_REASON: Record<string, string> = {
  ...REASON,
  all_failed: '所有決策模型都失敗',
  thread_reply_fallback: '沒有決策模型，由 thread 最近發言者接手',
}

export function WhyModal({ message, onClose }: { message: MessageView; onClose: () => void }) {
  const [data, setData] = useState<WakeExplanation | null>(null)
  const [view, setView] = useState<WakeWithAgent | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    api2
      .explain(message.id)
      .then(setData)
      .catch((e) => setError(String(e)))
  }, [message.id])
  const wakes: WakeWithAgent[] = data?.wakes ?? []
  const handle = (id: string) => data?.agent_handles[id] ?? id.slice(0, 8)
  return (
    <Modal title="為什麼喚醒了這些 Agent？" onClose={onClose}>
      <blockquote className="quote">
        <strong>{message.author_display_name}</strong>：{message.body.slice(0, 200)}
      </blockquote>
      <p className="muted small">
        因果深度 {message.depth}。
        {message.author_kind === 'agent' ? 'Agent 的訊息只走規則層（tag、私訊、thread），不送決策模型。' : ''}
      </p>
      {data?.produced_by && (
        <p className="muted small">
          這則訊息是 @{data.produced_by.agent_handle} 在一次喚醒（{REASON[data.produced_by.reason] ?? data.produced_by.reason}）中發出的。
        </p>
      )}
      {error && <div className="form-error">{error}</div>}
      {data?.decisions.map((d) => (
        <div key={d.id} className="decision">
          <div className="decision-head">
            <span className="tag">{SOURCE[d.source] ?? d.source}</span>
            <span>{DECIDER_REASON[d.reason ?? ''] ?? d.reason}</span>
            {d.mode && <span className="muted small">模式 {d.mode}</span>}
            {d.latency_ms != null && d.source === 'decider' && <span className="muted small">{d.latency_ms} ms</span>}
          </div>
          {d.scores && (
            <div className="score-bars">
              {Object.entries(d.scores)
                .sort((a, b) => b[1] - a[1])
                .map(([h, s]) => (
                  <div key={h} className="score-row">
                    <span className="score-name">@{h}</span>
                    <span className="score-bar">
                      <i style={{ width: `${Math.round(s * 100)}%` }} />
                    </span>
                    <span className="score-val">{s.toFixed(2)}</span>
                  </div>
                ))}
            </div>
          )}
          {d.interaction && (
            <div className="muted small">
              互動判斷：{d.interaction === 'discuss' ? '需要討論' : '各自回答'}（信心 {d.interaction_confidence?.toFixed(2)}）
            </div>
          )}
          {d.attempts.length > 0 && (
            <ul className="attempts">
              {d.attempts.map((a, i) => (
                <li key={i} className={a.ok ? 'ok' : 'bad'}>
                  {a.ok ? '✓' : a.skipped ? '–' : '✕'} {a.name}
                  {a.ok ? ` · ${a.latency_ms} ms` : a.skipped ? `（略過：${a.skipped}）` : `（${a.error_kind}：${a.error?.slice(0, 120)}）`}
                </li>
              ))}
            </ul>
          )}
          {d.woken.length > 0 && <div className="small">喚醒：{d.woken.map((w) => '@' + handle(w)).join('、')}</div>}
        </div>
      ))}
      {data && wakes.length === 0 && <p>這則訊息沒有喚醒任何 Agent。</p>}
      {wakes.length > 0 && (
        <table className="table">
          <thead>
            <tr>
              <th>Agent</th>
              <th>原因</th>
              <th>模式</th>
              <th>狀態</th>
              <th>發言</th>
              <th>Tokens</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {wakes.map((w) => (
              <tr key={w.id}>
                <td>@{w.agent_handle}</td>
                <td>{REASON[w.reason] ?? w.reason}</td>
                <td>{w.mode}</td>
                <td>{STATUS[w.status] ?? w.status}</td>
                <td>{w.posted_count}</td>
                <td>{w.tokens_used}</td>
                <td>
                  {w.status !== 'pending' && (
                    <button className="link small" onClick={() => setView(w)}>
                      Agent 視角
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {view && <AgentView wakeId={view.id} agentHandle={view.agent_handle} onClose={() => setView(null)} />}
      {wakes.some((w) => w.private_log) && (
        <div className="private-logs">
          <div className="field-label">私人紀錄（僅管理員可見）</div>
          {wakes
            .filter((w) => w.private_log)
            .map((w) => (
              <div key={w.id} className="private-log">
                <strong>@{w.agent_handle}</strong>：{w.private_log}
              </div>
            ))}
        </div>
      )}
    </Modal>
  )
}
