import type { Member, MessageView } from '../api'
import { Avatar } from './Avatar'
import Markdown from './Markdown'

const QUICK_REACTIONS = ['👍', '✅', '👀', '🙏']

export function formatTime(iso: string) {
  return new Date(iso).toLocaleTimeString('zh-TW', { hour: 'numeric', minute: '2-digit' })
}

export function formatDay(iso: string) {
  const d = new Date(iso)
  const today = new Date()
  const y = new Date(today)
  y.setDate(today.getDate() - 1)
  if (d.toDateString() === today.toDateString()) return '今天'
  if (d.toDateString() === y.toDateString()) return '昨天'
  return d.toLocaleDateString('zh-TW', { month: 'long', day: 'numeric', weekday: 'long' })
}

interface Props {
  me: Member
  msg: MessageView
  compact: boolean
  inThread?: boolean
  onReply?: () => void
  onReact: (emoji: string, add: boolean) => void
  onWhy: () => void
  onDelete: () => void
  onProfile?: (memberId: string) => void
}

export default function MessageItem({ me, msg, compact, inThread, onReply, onReact, onWhy, onDelete, onProfile }: Props) {
  if (msg.author_kind === 'system') {
    return (
      <div className="msg system">
        <div className="msg-gutter">
          <span className="sys-icon">ⓘ</span>
        </div>
        <div className="msg-content">
          <span className="sys-text">{msg.body}</span>
          <span className="msg-time">{formatTime(msg.created_at)}</span>
        </div>
      </div>
    )
  }
  return (
    <div className={`msg ${compact ? 'compact' : ''}`}>
      <div className="msg-gutter">
        {compact ? (
          <span className="msg-time hover-only">{formatTime(msg.created_at)}</span>
        ) : (
          <button className="avatar-btn" onClick={() => onProfile?.(msg.author_id)}>
            <Avatar name={msg.author_display_name} kind={msg.author_kind} />
          </button>
        )}
      </div>
      <div className="msg-content">
        {!compact && (
          <div className="msg-meta">
            <button className="msg-author" onClick={() => onProfile?.(msg.author_id)}>
              {msg.author_display_name}
            </button>
            {msg.author_kind === 'agent' && <span className="app-badge">AI</span>}
            <span className="msg-time" title={new Date(msg.created_at).toLocaleString('zh-TW')}>
              {formatTime(msg.created_at)}
            </span>
            {msg.depth > 0 && (
              <span className="msg-depth" title="因果深度：由 Agent 回覆鏈產生">
                深度 {msg.depth}
              </span>
            )}
          </div>
        )}
        <div className="msg-body">
          <Markdown text={msg.body} mentions={msg.mentions} />
        </div>
        {msg.reactions.length > 0 && (
          <div className="reactions">
            {msg.reactions.map((r) => {
              const mine = r.handles.includes(me.handle)
              return (
                <button
                  key={r.emoji}
                  className={`reaction ${mine ? 'mine' : ''}`}
                  title={r.handles.map((h) => '@' + h).join('、') + ' 回應了 ' + r.emoji}
                  onClick={() => onReact(r.emoji, !mine)}
                >
                  <span>{r.emoji}</span> <span className="count">{r.count}</span>
                </button>
              )
            })}
            <button className="reaction add" title="加上表情" onClick={() => onReact('👍', true)}>
              ☺︎+
            </button>
          </div>
        )}
        {!inThread && msg.reply_count > 0 && onReply && (
          <button className="thread-summary" onClick={onReply}>
            <span className="thread-count">{msg.reply_count} 則回覆</span>
            <span className="thread-view">查看討論串</span>
          </button>
        )}
      </div>
      <div className="msg-actions">
        {QUICK_REACTIONS.map((e) => (
          <button key={e} title={`回應 ${e}`} onClick={() => onReact(e, true)}>
            {e}
          </button>
        ))}
        {onReply && !inThread && (
          <button title="在討論串中回覆" onClick={onReply}>
            💬
          </button>
        )}
        <button title="為什麼喚醒了這些 Agent？" onClick={onWhy}>
          ⓘ
        </button>
        {(msg.author_id === me.id || me.is_admin) && (
          <button title="刪除訊息" onClick={onDelete}>
            🗑
          </button>
        )}
      </div>
    </div>
  )
}
