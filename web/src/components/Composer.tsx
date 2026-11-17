import { useRef, useState, type KeyboardEvent } from 'react'
import type { ConvMember, WakeStatus } from '../api'
import { Avatar } from './Avatar'

interface Working {
  agent_handle: string
  status: WakeStatus
}

interface Props {
  members: ConvMember[]
  placeholder: string
  disabled?: boolean
  working?: Working[]
  nameOf?: (handle: string) => string
  onSend: (body: string) => Promise<boolean>
}

const EMOJI = ['👍', '🙏', '✅', '🎉', '👀', '🔥', '😄', '🤔']

/** Message box with `@` autocomplete. Enter sends, Shift+Enter inserts a newline. */
export default function Composer({ members, placeholder, disabled, working = [], nameOf = (h) => h, onSend }: Props) {
  const [text, setText] = useState('')
  const [query, setQuery] = useState<string | null>(null)
  const [sel, setSel] = useState(0)
  const [sending, setSending] = useState(false)
  const [emoji, setEmoji] = useState(false)
  const ref = useRef<HTMLTextAreaElement>(null)

  const everyone: ConvMember = {
    member_id: '@all',
    handle: 'all',
    display_name: '所有人',
    kind: 'system',
    description: '通知這個對話的每個成員，所有 Agent 都會被喚醒',
    agent_status: null,
    last_read_seq: 0,
  }
  const candidates =
    query === null
      ? []
      : [...members.filter((m) => m.kind !== 'system'), everyone]
          .filter(
            (m) =>
              m.handle.toLowerCase().startsWith(query.toLowerCase()) ||
              m.display_name.toLowerCase().includes(query.toLowerCase()),
          )
          .slice(0, 8)

  const updateQuery = (value: string, caret: number) => {
    const m = /(^|[^A-Za-z0-9_.-])@([\p{L}\p{N}_.-]*)$/u.exec(value.slice(0, caret))
    setQuery(m ? m[2] : null)
    setSel(0)
  }

  const insert = (s: string) => {
    const el = ref.current
    const caret = el?.selectionStart ?? text.length
    const next = text.slice(0, caret) + s + text.slice(caret)
    setText(next)
    requestAnimationFrame(() => {
      el?.focus()
      el?.setSelectionRange(caret + s.length, caret + s.length)
      if (el) updateQuery(next, caret + s.length)
    })
  }

  const pick = (handle: string) => {
    const el = ref.current
    if (!el) return
    const caret = el.selectionStart
    const before = text.slice(0, caret).replace(/@([\p{L}\p{N}_.-]*)$/u, `@${handle} `)
    setText(before + text.slice(caret))
    setQuery(null)
    requestAnimationFrame(() => {
      el.focus()
      el.setSelectionRange(before.length, before.length)
    })
  }

  const send = async () => {
    const body = text.trim()
    if (!body || sending) return
    setSending(true)
    const ok = await onSend(body)
    setSending(false)
    if (ok) setText('')
    ref.current?.focus()
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (candidates.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setSel((s) => (s + 1) % candidates.length)
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setSel((s) => (s - 1 + candidates.length) % candidates.length)
        return
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        pick(candidates[sel].handle)
        return
      }
      if (e.key === 'Escape') {
        setQuery(null)
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault()
      send()
    }
  }

  const label = (s: WakeStatus) => (s === 'pending' ? '排隊中' : s === 'delivered' ? '已收到' : '正在處理')

  return (
    <div className="composer-wrap">
      <div className="composer">
        {candidates.length > 0 && (
          <ul className="mention-list">
            {candidates.map((m, i) => (
              <li
                key={m.member_id}
                className={i === sel ? 'selected' : ''}
                onMouseDown={(e) => {
                  e.preventDefault()
                  pick(m.handle)
                }}
              >
                <Avatar name={m.display_name} kind={m.kind} size={20} />
                <strong>{m.display_name}</strong> <span className="muted">@{m.handle}</span>
                {m.member_id === '@all' && <span className="muted small">（每個 Agent 都會被喚醒）</span>}
                {m.kind === 'agent' && <span className="app-badge">AI</span>}
              </li>
            ))}
          </ul>
        )}
        <textarea
          ref={ref}
          rows={Math.min(10, Math.max(1, text.split('\n').length))}
          value={text}
          placeholder={placeholder}
          disabled={disabled}
          onChange={(e) => {
            setText(e.target.value)
            updateQuery(e.target.value, e.target.selectionStart)
          }}
          onKeyDown={onKeyDown}
          onClick={(e) => updateQuery(text, e.currentTarget.selectionStart)}
        />
        <div className="composer-bar">
          <div className="composer-tools">
            <button title="提及成員" onClick={() => insert('@')}>
              @
            </button>
            <div className="menu-wrap">
              <button title="表情" onClick={() => setEmoji(!emoji)}>
                ☺
              </button>
              {emoji && (
                <div className="emoji-pop" onMouseLeave={() => setEmoji(false)}>
                  {EMOJI.map((e) => (
                    <button
                      key={e}
                      onClick={() => {
                        insert(e)
                        setEmoji(false)
                      }}
                    >
                      {e}
                    </button>
                  ))}
                </div>
              )}
            </div>
            <button title="程式碼區塊" onClick={() => insert('\n```\n\n```')}>
              {'</>'}
            </button>
          </div>
          <button className={`send-btn ${text.trim() ? 'ready' : ''}`} disabled={!text.trim() || sending || disabled} onClick={send} title="送出（Enter）">
            ➤
          </button>
        </div>
      </div>
      <div className="composer-foot">
        {working.length > 0 ? (
          <span className="working">
            <span className="dots">
              <i />
              <i />
              <i />
            </span>
            {working.map((w, i) => (
              <span key={i}>
                <strong>{nameOf(w.agent_handle)}</strong> {label(w.status)}
                {i < working.length - 1 ? '，' : ''}
              </span>
            ))}
            …
          </span>
        ) : (
          <span className="hint">
            <strong>Enter</strong> 送出 · <strong>Shift + Enter</strong> 換行 · 支援 **粗體**、`程式碼`
          </span>
        )}
      </div>
    </div>
  )
}
