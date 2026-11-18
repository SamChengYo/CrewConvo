import { useEffect, useState } from 'react'
import { api, connectApi, type Agent, type ApproveResult, type ConnectClientAgent, type ConnectRequestView, type Member } from '../api'

interface Row {
  platform: string
  name: string
  wakeable: boolean
  /** Reconnect this existing agent instead of creating a new one. */
  existing: Agent | null
  reuse: boolean
  handle: string
  displayName: string
  description: string
}

function rowsFrom(req: ConnectRequestView, mine: Agent[]): Row[] {
  const c = req.client
  const list: ConnectClientAgent[] = c.agents?.length
    ? c.agents
    : [{ platform: 'custom', name: 'Agent', wakeable: true, suggested_handle: `${c.hostname ?? 'agent'}`.toLowerCase() }]
  return list.map((a) => {
    const existing = mine.find((m) => m.member.id === a.existing_agent_id) ?? null
    return {
      platform: a.platform,
      name: a.name,
      wakeable: a.wakeable,
      existing,
      reuse: !!existing,
      handle: a.suggested_handle,
      displayName: `${a.name}（${c.hostname ?? ''}）`,
      description: `${c.user ? `${c.user} 的` : ''}${c.hostname ?? '電腦'}上的 ${a.name}`,
    }
  })
}

export default function ConnectPage({
  me,
  initialCode,
  onDone,
}: {
  me: Member
  initialCode: string
  onDone: (channelId?: string) => void
}) {
  const [code, setCode] = useState(initialCode)
  const [req, setReq] = useState<ConnectRequestView | null>(null)
  const [lookupError, setLookupError] = useState<string | null>(null)
  const [mine, setMine] = useState<Agent[] | null>(null)
  const [rows, setRows] = useState<Row[]>([])
  const [group, setGroup] = useState('')
  const [channel, setChannel] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<ApproveResult | 'denied' | null>(null)

  useEffect(() => {
    api
      .agents()
      .then((all) =>
        setMine(all.filter((a) => a.origin === 'external' && !a.provider_connection_id && !a.member.deactivated_at && (a.owner_id === me.id || me.is_admin))),
      )
      .catch(() => setMine([]))
  }, [me])

  const lookup = async (c: string, agents: Agent[]) => {
    setLookupError(null)
    setReq(null)
    if (c.replace(/[^a-z0-9]/gi, '').length < 8) return
    try {
      const r = await connectApi.get(c)
      setReq(r)
      setRows(rowsFrom(r, agents))
      setGroup(r.client.suggested_group ?? '')
      setChannel(r.client.suggested_channel ?? '')
    } catch (e) {
      setLookupError(e instanceof Error ? e.message : String(e))
    }
  }

  useEffect(() => {
    if (mine) lookup(initialCode, mine)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mine])

  const edit = (i: number, patch: Partial<Row>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)))

  const approve = async () => {
    if (!req) return
    setBusy(true)
    setError(null)
    try {
      setResult(
        await connectApi.approve(req.user_code, {
          agents: rows.map((r) =>
            r.reuse && r.existing
              ? { platform: r.platform, agent_id: r.existing.member.id }
              : { platform: r.platform, handle: r.handle.trim(), display_name: r.displayName.trim() || undefined, description: r.description.trim() },
          ),
          group_name: group.trim() || undefined,
          channel_name: channel.trim() || undefined,
        }),
      )
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  const deny = async () => {
    if (!req) return
    setBusy(true)
    try {
      await connectApi.deny(req.user_code)
      setResult('denied')
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  if (result) {
    const okRes = result === 'denied' ? null : result
    return (
      <div className="page connect-page">
        <div className="connect-card">
          {okRes ? (
            <>
              <div className="connect-icon ok">✓</div>
              <h1>已核准</h1>
              <p>
                {okRes.agents.map((a) => `@${a.member.handle}`).join('、')} 已經建立。回到終端機，crewconvo 會把它們裝進對應的 Coding Agent 並啟動背景
                bridge。
              </p>
              {okRes.group && <p className="muted">群組「{okRes.group.name}」：這些 Agent 互相找得到。</p>}
              {okRes.channel && <p className="muted">頻道 #{okRes.channel.name}：你和這些 Agent 都在裡面，直接 @ 它們就可以開始。</p>}
              {okRes.warnings.map((w) => (
                <p key={w} className="pill-note warn">
                  {w}
                </p>
              ))}
            </>
          ) : (
            <>
              <div className="connect-icon">✕</div>
              <h1>已拒絕</h1>
              <p>那台電腦不會取得任何權限。</p>
            </>
          )}
          <button className="primary" onClick={() => onDone(okRes?.channel?.id)}>
            {okRes?.channel ? `前往 #${okRes.channel.name}` : '回到 CrewConvo'}
          </button>
        </div>
      </div>
    )
  }

  const pending = req?.status === 'pending'
  const valid = rows.length > 0 && rows.every((r) => (r.reuse && r.existing) || (r.handle.trim() && r.description.trim()))

  return (
    <div className="page connect-page">
      <div className="connect-card wide">
        <h1>連接這台電腦上的 Coding Agent</h1>
        <p className="muted">
          有人在終端機執行了 <code>crewconvo setup</code>。核准後，下面每個 Coding Agent 都會成為群聊裡的一個 Agent，被 @ 時自動回覆。
        </p>

        <label className="connect-code">
          <span>終端機上顯示的代碼</span>
          <input
            value={code}
            placeholder="ABCD-EFGH"
            onChange={(e) => {
              setCode(e.target.value.toUpperCase())
              lookup(e.target.value, mine ?? [])
            }}
          />
        </label>
        {lookupError && <p className="form-error">{lookupError}</p>}

        {req && (
          <div className="connect-client">
            <div>
              <b>電腦</b>：{req.client.hostname ?? '（未提供）'}
              {req.client.user ? `（使用者 ${req.client.user}）` : ''}　<b>系統</b>：{req.client.os ?? '—'}
            </div>
            {!pending && (
              <p className="pill-note warn">
                {req.status === 'expired' ? '代碼已過期，請在終端機重新執行 crewconvo setup。' : '這個代碼已經處理過了。'}
              </p>
            )}
            {pending && <p className="muted small">請確認終端機上的代碼和上面一致，而且是你自己（或你信任的人）剛剛執行的。</p>}
          </div>
        )}

        {pending && (
          <>
            {rows.map((r, i) => (
              <div key={r.platform} className="connect-agent">
                <div className="connect-agent-head">
                  <b>{r.name}</b>
                  <span className={`status ${r.wakeable ? 'online' : 'offline'}`}>{r.wakeable ? '被 @ 時會自動回覆' : '只能主動使用'}</span>
                  {r.existing && (
                    <label className="connect-reuse">
                      <input type="checkbox" checked={r.reuse} onChange={(e) => edit(i, { reuse: e.target.checked })} /> 沿用既有的 @
                      {r.existing.member.handle}
                    </label>
                  )}
                </div>
                {!(r.reuse && r.existing) && (
                  <div className="connect-form">
                    <div className="connect-two">
                      <label>
                        <span>名稱（用 @ 呼叫，可以用中文，不能有空白）</span>
                        <input value={r.handle} onChange={(e) => edit(i, { handle: e.target.value })} />
                      </label>
                      <label>
                        <span>顯示名稱</span>
                        <input value={r.displayName} onChange={(e) => edit(i, { displayName: e.target.value })} />
                      </label>
                    </div>
                    <label>
                      <span>它負責什麼（其他成員與 Agent 靠這段判斷要不要找它）</span>
                      <input value={r.description} onChange={(e) => edit(i, { description: e.target.value })} />
                    </label>
                  </div>
                )}
              </div>
            ))}

            <div className="connect-two">
              <label className="connect-field">
                <span>Agent 群組（讓這些 Agent 互相找得到；留白就不建立）</span>
                <input value={group} onChange={(e) => setGroup(e.target.value)} />
              </label>
              <label className="connect-field">
                <span>頻道（你和這些 Agent 的私有頻道；留白就不建立）</span>
                <input value={channel} onChange={(e) => setChannel(e.target.value)} />
              </label>
            </div>

            {error && <p className="form-error">{error}</p>}
            <div className="connect-actions">
              <button className="ghost" disabled={busy} onClick={deny}>
                拒絕
              </button>
              <button className="primary" disabled={busy || !valid} onClick={approve}>
                核准並建立
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
