import { useCallback, useEffect, useState } from 'react'
import { providersApi, type ProviderConnection } from '../api'

/** Platform connections: one URL + access token brings in all of a platform workspace's agents. */
export default function ProvidersSection({ onError }: { onError: (m: string) => void }) {
  const [conns, setConns] = useState<ProviderConnection[]>([])
  const [url, setUrl] = useState('')
  const [token, setToken] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)

  const reload = useCallback(() => {
    providersApi.list().then(setConns).catch((e) => onError(String(e)))
  }, [onError])
  useEffect(reload, [reload])

  const run = async (key: string, fn: () => Promise<string | null>) => {
    setBusy(key)
    setNote(null)
    try {
      setNote(await fn())
      reload()
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(null)
    }
  }

  const connect = () =>
    run('new', async () => {
      const r = await providersApi.connect({ url: url.trim(), token: token.trim() || undefined })
      setUrl('')
      setToken('')
      return `已連上「${r.connection.external_workspace_name ?? r.connection.name}」：${r.agents} 個 Agent${
        r.created.length ? `（${r.created.map((h) => `@${h}`).join('、')}）` : ''
      }，並建立了同名的 Agent 群組。`
    })

  return (
    <section className="int-section">
      <h2>平台連線（A2A）</h2>
      <p className="muted small">
        給「一次託管很多 Agent」的平台用。填入平台提供的目錄網址與 Access Token，CrewConvo 就會讀出這個 Token 可用的 Agent，用 Agent
        的名稱建立成員、建立以平台工作區命名的 Agent 群組，之後每 10 分鐘自動同步（新增、改名、下架）。這些 Agent 用{' '}
        <a href="https://a2a-protocol.org" target="_blank" rel="noreferrer">
          A2A 協議
        </a>
        被呼叫，只有被 @ 或私訊時才會回應，也不會自動加入任何頻道（請自行邀請進頻道）。也可以直接填一個標準 A2A Agent Card 的網址。
      </p>
      <div className="provider-form">
        <label>
          <span>目錄網址或 Agent Card 網址</span>
          <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://platform.example.com/crewconvo/catalog" />
        </label>
        <label>
          <span>Access Token（平台核發，可留白）</span>
          <input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" />
        </label>
        <button className="primary" disabled={!url.trim() || busy !== null} onClick={connect}>
          {busy === 'new' ? '連線中…' : '連線並同步'}
        </button>
      </div>
      {note && <p className="pill-note ok">{note}</p>}
      {conns.length > 0 && (
        <table className="int-table">
          <thead>
            <tr>
              <th>平台工作區</th>
              <th>網址</th>
              <th>最後同步</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {conns.map((c) => (
              <tr key={c.id}>
                <td>
                  <b>{c.external_workspace_name ?? c.name}</b>
                  {c.external_workspace_id && <div className="muted small">{c.external_workspace_id}</div>}
                </td>
                <td className="provider-url">{c.catalog_url}</td>
                <td>
                  {c.last_error ? (
                    <span className="status offline" title={c.last_error}>
                      同步失敗
                    </span>
                  ) : c.last_sync_at ? (
                    new Date(c.last_sync_at).toLocaleString()
                  ) : (
                    '—'
                  )}
                  {c.last_error && <div className="muted small">{c.last_error}</div>}
                </td>
                <td className="provider-actions">
                  <button
                    className="ghost small"
                    disabled={busy !== null}
                    onClick={() =>
                      run(c.id, async () => {
                        const r = await providersApi.sync(c.id)
                        return `同步完成：${r.agents} 個 Agent（新增 ${r.created.length}、下架 ${r.removed}）。`
                      })
                    }
                  >
                    {busy === c.id ? '同步中…' : '立即同步'}
                  </button>
                  <button
                    className="ghost small"
                    disabled={busy !== null}
                    onClick={() => {
                      if (confirm(`移除「${c.external_workspace_name ?? c.name}」？它的 Agent 會停用並離開所有頻道，群組也會刪除。`))
                        run(c.id, async () => {
                          await providersApi.remove(c.id)
                          return '已移除連線。'
                        })
                    }}
                  >
                    移除
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  )
}
