import { useEffect, useState } from 'react'
import { api2, type MessageView, type SearchResults } from '../api'

function highlight(text: string, query: string) {
  const terms = query
    .replace(/["“”「」]/g, ' ')
    .split(/\s+/)
    .filter((t) => t.length > 0)
  if (!terms.length) return text
  const re = new RegExp(`(${terms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})`, 'gi')
  return text.split(re).map((part, i) => (re.test(part) ? <mark key={i}>{part}</mark> : <span key={i}>{part}</span>))
}

function Ctx({ m }: { m: MessageView | null }) {
  if (!m) return null
  return (
    <div className="hit-ctx">
      <span className="muted">@{m.author_handle}</span> {m.body.slice(0, 120)}
    </div>
  )
}

export default function SearchPanel({
  initialQuery,
  onClose,
  onOpen,
}: {
  initialQuery: string
  onClose: () => void
  onOpen: (conversationId: string) => void
}) {
  const [query, setQuery] = useState(initialQuery)
  const [results, setResults] = useState<SearchResults | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  const run = async (q: string, offset = 0) => {
    if (!q.trim()) return
    setLoading(true)
    setError(null)
    try {
      const r = await api2.search(q, { offset, limit: 20 })
      setResults((prev) => (offset > 0 && prev ? { ...r, hits: [...prev.hits, ...r.hits] } : r))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    run(initialQuery)
  }, [initialQuery])

  return (
    <aside className="side-pane search-pane">
      <header className="pane-head">
        <form
          className="search-form"
          onSubmit={(e) => {
            e.preventDefault()
            run(query)
          }}
        >
          <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder='搜尋訊息（"片語"）' autoFocus />
        </form>
        <button className="icon-btn" onClick={onClose}>
          ✕
        </button>
      </header>
      <div className="messages">
        {error && <div className="form-error pad">{error}</div>}
        {results && (
          <div className="muted small pad">
            共 {results.total} 筆結果{loading ? '…' : ''}
          </div>
        )}
        {results?.hits.map((h) => (
          <button key={h.message.id} className="hit" onClick={() => onOpen(h.conversation.id)}>
            <div className="hit-head">
              <strong>{h.conversation.kind === 'channel' ? `# ${h.conversation.name}` : '私訊'}</strong>
              <span className="muted small">{new Date(h.message.created_at).toLocaleString()}</span>
            </div>
            <Ctx m={h.before} />
            <div className="hit-body">
              <span className="author">{h.message.author_display_name}</span>
              {h.message.author_kind === 'agent' && <span className="badge">AI</span>}：{highlight(h.message.body, query)}
            </div>
            <Ctx m={h.after} />
          </button>
        ))}
        {results && results.hits.length < results.total && (
          <button className="load-older" disabled={loading} onClick={() => run(query, results.hits.length)}>
            載入更多
          </button>
        )}
        {results && results.total === 0 && <div className="empty-hint">沒有找到。只會搜尋你所在的對話。</div>}
      </div>
    </aside>
  )
}
