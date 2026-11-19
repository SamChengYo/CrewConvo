import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { api, type Agent, type ConversationDetail, type LocalProject, type Member, type MessageView, type WakeStatus } from '../api'
import { useRealtime } from '../realtime'
import { Avatar } from './Avatar'
import Composer from './Composer'
import MessageItem, { formatDay } from './MessageItem'
import { InviteModal, WhyModal } from './Modals'

interface ActiveWake {
  agent_handle: string
  status: WakeStatus
  thread_root_id: string | null
}

type Pane = { kind: 'thread'; root: MessageView } | { kind: 'members' } | { kind: 'profile'; memberId: string } | null

const ACTIVE: WakeStatus[] = ['pending', 'delivered', 'running']
const PAGE = 50

interface Props {
  me: Member
  conversationId: string
  /** The coding-agent project this conversation belongs to, when it is one. */
  project?: LocalProject
  members: Member[]
  agents: Agent[]
  onRead: (id: string, seq: number) => void
  onError: (msg: string) => void
  onLeft: () => void
  onOpenAgent: (agentId: string) => void
}

function upsert(list: MessageView[], m: MessageView) {
  const i = list.findIndex((x) => x.id === m.id)
  if (i >= 0) {
    const next = list.slice()
    next[i] = m
    return next
  }
  return [...list, m].sort((a, b) => a.seq - b.seq)
}

function sameDay(a: string, b: string) {
  return new Date(a).toDateString() === new Date(b).toDateString()
}

function groupCompact(list: MessageView[], i: number) {
  if (i === 0) return false
  const prev = list[i - 1]
  const cur = list[i]
  return (
    prev.author_id === cur.author_id &&
    cur.author_kind !== 'system' &&
    prev.author_kind !== 'system' &&
    sameDay(prev.created_at, cur.created_at) &&
    new Date(cur.created_at).getTime() - new Date(prev.created_at).getTime() < 5 * 60_000
  )
}

function MessageList({
  items,
  me,
  inThread,
  onReply,
  onReact,
  onWhy,
  onDelete,
  onProfile,
}: {
  items: MessageView[]
  me: Member
  inThread?: boolean
  onReply?: (m: MessageView) => void
  onReact: (m: MessageView, emoji: string, add: boolean) => void
  onWhy: (m: MessageView) => void
  onDelete: (m: MessageView) => void
  onProfile: (id: string) => void
}) {
  return (
    <>
      {items.map((m, i) => {
        const newDay = i === 0 || !sameDay(items[i - 1].created_at, m.created_at)
        return (
          <Fragment key={m.id}>
            {newDay && !inThread && (
              <div className="day-divider">
                <span>{formatDay(m.created_at)}</span>
              </div>
            )}
            <MessageItem
              me={me}
              msg={m}
              inThread={inThread}
              compact={!newDay && groupCompact(items, i)}
              onReply={onReply ? () => onReply(m) : undefined}
              onReact={(e, add) => onReact(m, e, add)}
              onWhy={() => onWhy(m)}
              onDelete={() => onDelete(m)}
              onProfile={onProfile}
            />
          </Fragment>
        )
      })}
    </>
  )
}

export default function ConversationView({ me, conversationId, project, members, agents, onRead, onError, onLeft, onOpenAgent }: Props) {
  const [detail, setDetail] = useState<ConversationDetail | null>(null)
  const [messages, setMessages] = useState<MessageView[]>([])
  const [hasMore, setHasMore] = useState(false)
  const [pane, setPane] = useState<Pane>(null)
  const [wakes, setWakes] = useState<Map<string, ActiveWake>>(new Map())
  const [why, setWhy] = useState<MessageView | null>(null)
  const [inviting, setInviting] = useState(false)
  const [menu, setMenu] = useState(false)
  const listRef = useRef<HTMLDivElement>(null)
  const stickBottom = useRef(true)

  const fail = useCallback((e: unknown) => onError(e instanceof Error ? e.message : String(e)), [onError])

  const loadDetail = useCallback(async () => {
    const d = await api.conversation(conversationId)
    setDetail(d)
    const byId = new Map(d.members.map((m) => [m.member_id, m.handle]))
    setWakes(
      new Map(
        d.active_wakes.map((w) => [
          w.id,
          { agent_handle: byId.get(w.agent_id) ?? '?', status: w.status, thread_root_id: w.thread_root_id },
        ]),
      ),
    )
    return d
  }, [conversationId])

  useEffect(() => {
    let cancelled = false
    loadDetail()
      .then(async (d) => {
        if (!d.is_member) return
        const msgs = await api.messages(conversationId, { limit: PAGE })
        if (cancelled) return
        stickBottom.current = true
        setMessages(msgs)
        setHasMore(msgs.length === PAGE)
      })
      .catch(fail)
    return () => {
      cancelled = true
    }
  }, [conversationId, loadDetail, fail])

  const maxSeq = messages.length ? messages[messages.length - 1].seq : 0
  useEffect(() => {
    if (!maxSeq || !detail?.is_member) return
    const t = setTimeout(() => {
      api.markRead(conversationId, maxSeq).catch(() => {})
      onRead(conversationId, maxSeq)
    }, 300)
    return () => clearTimeout(t)
  }, [maxSeq, conversationId, detail?.is_member, onRead])

  useLayoutEffect(() => {
    const el = listRef.current
    if (el && stickBottom.current) el.scrollTop = el.scrollHeight
  }, [messages, wakes])

  useRealtime((ev) => {
    if ('conversation_id' in ev && ev.conversation_id !== conversationId) return
    switch (ev.type) {
      case 'message_created': {
        const m = ev.message
        if (m.conversation_id !== conversationId) return
        if (m.thread_root_id === null) {
          const el = listRef.current
          stickBottom.current = !el || el.scrollHeight - el.scrollTop - el.clientHeight < 120
          setMessages((list) => upsert(list, m))
          setPane((p) => (p?.kind === 'thread' && p.root.id === m.id ? { kind: 'thread', root: m } : p))
        }
        break
      }
      case 'message_deleted':
        setMessages((list) => list.filter((m) => m.id !== ev.message_id))
        break
      case 'reactions_changed':
        setMessages((list) => list.map((m) => (m.id === ev.message_id ? { ...m, reactions: ev.reactions } : m)))
        break
      case 'wake_status':
        setWakes((prev) => {
          const next = new Map(prev)
          if (ACTIVE.includes(ev.status)) {
            next.set(ev.wake_id, { agent_handle: ev.agent_handle, status: ev.status, thread_root_id: ev.thread_root_id })
          } else {
            next.delete(ev.wake_id)
          }
          return next
        })
        break
      case 'conversation_updated':
        if (ev.conversation.id === conversationId) setDetail((d) => (d ? { ...d, ...ev.conversation } : d))
        break
      case 'membership_changed':
        loadDetail().catch(() => {})
        break
      case 'resync':
        loadDetail().catch(() => {})
        api.messages(conversationId, { limit: PAGE }).then(setMessages).catch(() => {})
        break
    }
  })

  const loadOlder = async () => {
    if (!messages.length) return
    const el = listRef.current
    const prevHeight = el?.scrollHeight ?? 0
    const older = await api.messages(conversationId, { before_seq: messages[0].seq, limit: PAGE })
    stickBottom.current = false
    setMessages((list) => [...older, ...list])
    setHasMore(older.length === PAGE)
    requestAnimationFrame(() => {
      if (el) el.scrollTop = el.scrollHeight - prevHeight
    })
  }

  const send = async (body: string, thread?: string | null) => {
    try {
      stickBottom.current = true
      await api.post(conversationId, body, thread)
      return true
    } catch (e) {
      fail(e)
      return false
    }
  }

  const react = (m: MessageView, emoji: string, add: boolean) => api.react(m.id, emoji, add).catch(fail)
  const del = (m: MessageView) => {
    if (confirm('刪除這則訊息？')) api.deleteMessage(m.id).catch(fail)
  }
  const stop = () => api.stop(conversationId).catch(fail)

  const handleOf = useMemo(() => new Map(detail?.members.map((m) => [m.handle, m]) ?? []), [detail])
  const title = useMemo(() => {
    if (!detail) return ''
    if (detail.kind === 'channel') return detail.name ?? ''
    return detail.members
      .filter((m) => m.member_id !== me.id)
      .map((m) => m.display_name)
      .join('、')
  }, [detail, me.id])

  if (!detail) return <div className="empty-state">載入中…</div>

  if (!detail.is_member) {
    return (
      <div className="empty-state">
        <p>你還不是 #{detail.name} 的成員。</p>
        <button
          className="primary"
          onClick={() =>
            api
              .join(conversationId)
              .then(() => loadDetail())
              .then(() => api.messages(conversationId, { limit: PAGE }))
              .then(setMessages)
              .catch(fail)
          }
        >
          加入頻道
        </button>
      </div>
    )
  }

  const mainWakes = [...wakes.values()].filter((w) => w.thread_root_id === null)
  const isChannel = detail.kind === 'channel'
  const dmPeer = !isChannel ? detail.members.find((m) => m.member_id !== me.id) : undefined
  const profile = (id: string) => setPane({ kind: 'profile', memberId: id })

  return (
    <div className="conversation">
      <section className="conv-main">
        <header className="conv-head">
          <div className="conv-head-left">
            <button className="conv-title" onClick={() => setPane({ kind: 'members' })}>
              {isChannel ? (
                <span className="conv-hash">{detail.visibility === 'private' ? '🔒' : '#'}</span>
              ) : (
                <Avatar name={title} kind={dmPeer?.kind ?? 'human'} size={22} />
              )}
              <span>{title}</span>
              <span className="conv-caret">▾</span>
            </button>
            {project &&
              (project.path ? (
                <span className="conv-project" title={`專案資料夾：${project.path}`}>
                  📁 {project.project_name}
                  <span className="conv-project-path">{project.path}</span>
                  {!project.exists && <span className="pill warn">找不到資料夾</span>}
                </span>
              ) : (
                <span className="conv-project muted" title="這個對話還沒有綁定專案資料夾：Coding Agent 在這裡只能聊天。到「這台電腦的 Coding Agent」頁面綁定，或從側邊欄「新增專案」開一個新的對話">
                  📁 未指定專案
                </span>
              ))}
            {detail.topic && <span className="conv-topic">{detail.topic}</span>}
          </div>
          <div className="conv-head-right">
            {detail.frozen_until_human && <span className="frozen-pill">已停止 · 等待人類發言</span>}
            <button className="member-stack" onClick={() => setPane({ kind: 'members' })} title="成員">
              {detail.members.slice(0, 3).map((m) => (
                <Avatar key={m.member_id} name={m.display_name} kind={m.kind} size={22} />
              ))}
              <span className="member-count">{detail.members.length}</span>
            </button>
            <button className="stop-btn" title="取消所有進行中的喚醒，Agent 訊息不再觸發喚醒，直到人類發言" onClick={stop}>
              ■ 停止
            </button>
            {isChannel && (
              <div className="menu-wrap">
                <button className="icon-btn" onClick={() => setMenu(!menu)} title="更多">
                  ⋮
                </button>
                {menu && (
                  <div className="dropdown" onMouseLeave={() => setMenu(false)}>
                    <button
                      onClick={() => {
                        setMenu(false)
                        setInviting(true)
                      }}
                    >
                      邀請成員
                    </button>
                    {detail.name !== 'general' && (
                      <button
                        className="danger-text"
                        onClick={() => {
                          setMenu(false)
                          if (confirm(`離開 #${detail.name}？`)) api.leave(conversationId).then(onLeft).catch(fail)
                        }}
                      >
                        離開頻道
                      </button>
                    )}
                  </div>
                )}
              </div>
            )}
          </div>
        </header>
        <div className="messages" ref={listRef}>
          {hasMore ? (
            <button className="load-older" onClick={loadOlder}>
              載入較早的訊息
            </button>
          ) : (
            <div className="conv-intro">
              {isChannel ? (
                <>
                  <h2># {detail.name}</h2>
                  <p>
                    這是 <strong>#{detail.name}</strong> 頻道的開頭。{detail.topic ? `主題：${detail.topic}。` : ''}用 <code>@handle</code>{' '}
                    叫 Agent 出來，或直接提問讓決策模型挑人。
                  </p>
                </>
              ) : (
                <>
                  <Avatar name={title} kind={dmPeer?.kind ?? 'human'} size={64} />
                  <h2>{title}</h2>
                  <p>這是你和 {title} 的私訊開頭。{dmPeer?.kind === 'agent' ? '私訊裡的每則訊息都會喚醒這個 Agent。' : ''}</p>
                </>
              )}
            </div>
          )}
          <MessageList
            items={messages}
            me={me}
            onReply={(m) => setPane({ kind: 'thread', root: m })}
            onReact={react}
            onWhy={setWhy}
            onDelete={del}
            onProfile={profile}
          />
        </div>
        <Composer
          members={detail.members}
          placeholder={isChannel ? `傳訊息到 #${detail.name}` : `傳訊息給 ${title}`}
          onSend={(b) => send(b)}
          working={mainWakes}
          nameOf={(h) => handleOf.get(h)?.display_name ?? h}
        />
      </section>
      {pane?.kind === 'thread' && (
        <ThreadPane
          key={pane.root.id}
          me={me}
          root={pane.root}
          detail={detail}
          wakes={[...wakes.values()].filter((w) => w.thread_root_id === pane.root.id)}
          nameOf={(h) => handleOf.get(h)?.display_name ?? h}
          onClose={() => setPane(null)}
          onSend={(b) => send(b, pane.root.id)}
          onReact={react}
          onWhy={setWhy}
          onDelete={del}
          onError={fail}
          onProfile={profile}
        />
      )}
      {pane?.kind === 'members' && (
        <aside className="side-pane">
          <header className="pane-head">
            <div>
              <div className="pane-title">成員</div>
              <div className="pane-sub">{isChannel ? `#${detail.name}` : title}</div>
            </div>
            <button className="icon-btn" onClick={() => setPane(null)}>
              ✕
            </button>
          </header>
          <div className="pane-body">
            {isChannel && (
              <button className="member-row add" onClick={() => setInviting(true)}>
                <span className="add-box">＋</span> 邀請成員
              </button>
            )}
            {detail.members.map((m) => (
              <button key={m.member_id} className="member-row" onClick={() => profile(m.member_id)}>
                <Avatar
                  name={m.display_name}
                  kind={m.kind}
                  size={32}
                  presence={m.kind === 'agent' ? (m.agent_status === 'online' ? 'online' : 'offline') : null}
                />
                <div className="member-text">
                  <div className="member-name">
                    {m.display_name} {m.kind === 'agent' && <span className="app-badge">AI</span>}
                  </div>
                  <div className="muted small">@{m.handle}</div>
                </div>
              </button>
            ))}
          </div>
        </aside>
      )}
      {pane?.kind === 'profile' && (
        <ProfilePane
          memberId={pane.memberId}
          detail={detail}
          members={members}
          agents={agents}
          onClose={() => setPane(null)}
          onMessage={onOpenAgent}
        />
      )}
      {why && <WhyModal message={why} onClose={() => setWhy(null)} />}
      {inviting && (
        <InviteModal
          conversationId={conversationId}
          candidates={members.filter((m) => !detail.members.some((x) => x.member_id === m.id))}
          onClose={() => setInviting(false)}
          onError={fail}
        />
      )}
    </div>
  )
}

function ProfilePane({
  memberId,
  detail,
  members,
  agents,
  onClose,
  onMessage,
}: {
  memberId: string
  detail: ConversationDetail
  members: Member[]
  agents: Agent[]
  onClose: () => void
  onMessage: (id: string) => void
}) {
  const agent = agents.find((a) => a.member.id === memberId)
  const cm = detail.members.find((m) => m.member_id === memberId)
  const member = members.find((m) => m.id === memberId)
  const name = agent?.member.display_name ?? cm?.display_name ?? member?.display_name ?? '成員'
  const handle = agent?.member.handle ?? cm?.handle ?? member?.handle ?? ''
  const kind = agent ? 'agent' : (cm?.kind ?? member?.kind ?? 'human')
  return (
    <aside className="side-pane">
      <header className="pane-head">
        <div className="pane-title">個人資料</div>
        <button className="icon-btn" onClick={onClose}>
          ✕
        </button>
      </header>
      <div className="pane-body profile">
        <Avatar name={name} kind={kind} size={120} />
        <h2>
          {name} {kind === 'agent' && <span className="app-badge">AI</span>}
        </h2>
        <div className="muted">@{handle}</div>
        {agent && (
          <>
            <div className={`status-line ${agent.status}`}>● {agent.status === 'online' ? '在線' : agent.status}</div>
            <p className="profile-desc">{agent.description}</p>
            {agent.expertise.length > 0 && (
              <div className="tags">
                {agent.expertise.map((t) => (
                  <span className="tag" key={t}>
                    {t}
                  </span>
                ))}
              </div>
            )}
            <dl className="profile-facts">
              <dt>{agent.origin === 'builtin' ? '模型' : '類型'}</dt>
              <dd>
                <code>{agent.origin === 'builtin' ? agent.model : `外部 Agent（${agent.delivery}）`}</code>
              </dd>
              <dt>單次喚醒逾時</dt>
              <dd>{agent.max_wake_seconds} 秒</dd>
            </dl>
            <button className="primary wide" onClick={() => onMessage(agent.member.id)}>
              傳送私訊
            </button>
          </>
        )}
      </div>
    </aside>
  )
}

interface ThreadProps {
  me: Member
  root: MessageView
  detail: ConversationDetail
  wakes: ActiveWake[]
  nameOf: (handle: string) => string
  onClose: () => void
  onSend: (body: string) => Promise<boolean>
  onReact: (m: MessageView, emoji: string, add: boolean) => void
  onWhy: (m: MessageView) => void
  onDelete: (m: MessageView) => void
  onError: (e: unknown) => void
  onProfile: (id: string) => void
}

function ThreadPane({ me, root, detail, wakes, nameOf, onClose, onSend, onReact, onWhy, onDelete, onError, onProfile }: ThreadProps) {
  const [items, setItems] = useState<MessageView[]>([])
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    api.messages(detail.id, { thread_root_id: root.id, limit: 100 }).then(setItems).catch(onError)
  }, [detail.id, root.id, onError])

  useLayoutEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight
  }, [items.length, wakes.length])

  useRealtime((ev) => {
    switch (ev.type) {
      case 'message_created':
        if (ev.message.thread_root_id === root.id || ev.message.id === root.id) setItems((l) => upsert(l, ev.message))
        break
      case 'message_deleted':
        setItems((l) => l.filter((m) => m.id !== ev.message_id))
        break
      case 'reactions_changed':
        setItems((l) => l.map((m) => (m.id === ev.message_id ? { ...m, reactions: ev.reactions } : m)))
        break
    }
  })

  const head = items.find((m) => m.id === root.id) ?? root
  const replies = items.filter((m) => m.id !== root.id)

  return (
    <aside className="side-pane thread-pane">
      <header className="pane-head">
        <div>
          <div className="pane-title">討論串</div>
          <div className="pane-sub">{detail.kind === 'channel' ? `#${detail.name}` : '私訊'}</div>
        </div>
        <button className="icon-btn" onClick={onClose}>
          ✕
        </button>
      </header>
      <div className="messages" ref={ref}>
        <MessageList items={[head]} me={me} inThread onReact={onReact} onWhy={onWhy} onDelete={onDelete} onProfile={onProfile} />
        {replies.length > 0 && (
          <div className="reply-divider">
            <span>{replies.length} 則回覆</span>
          </div>
        )}
        <MessageList items={replies} me={me} inThread onReact={onReact} onWhy={onWhy} onDelete={onDelete} onProfile={onProfile} />
      </div>
      <Composer members={detail.members} placeholder="回覆…" onSend={onSend} working={wakes} nameOf={nameOf} />
    </aside>
  )
}
