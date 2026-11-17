import { useEffect, useState } from 'react'
import { api, connectApi, type Agent, type ConnectRequestView, type Member } from '../api'

function suggestHandle(c: ConnectRequestView['client']) {
  const base = `${c.user || 'agent'}-${c.platforms?.includes('claude-code') ? 'claude' : c.hostname || 'bot'}`
  return base
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 32)
}

const PLATFORM_NAMES: Record<string, string> = { 'claude-code': 'Claude Code', cursor: 'Cursor' }

export default function ConnectPage({ me, initialCode, onDone }: { me: Member; initialCode: string; onDone: () => void }) {
  const [code, setCode] = useState(initialCode)
  const [req, setReq] = useState<ConnectRequestView | null>(null)
  const [lookupError, setLookupError] = useState<string | null>(null)
  const [mine, setMine] = useState<Agent[]>([])
  const [mode, setMode] = useState<'new' | 'existing'>('new')
  const [existing, setExisting] = useState('')
  const [handle, setHandle] = useState('')
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [expertise, setExpertise] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<{ ok: boolean; agent?: Agent } | null>(null)

  const lookup = async (c: string) => {
    setLookupError(null)
    setReq(null)
    if (c.replace(/[^a-z0-9]/gi, '').length < 8) return
    try {
      const r = await connectApi.get(c)
      setReq(r)
      setHandle((h) => h || suggestHandle(r.client))
      setName((n) => n || (r.client.platforms?.includes('claude-code') ? `Claude（${r.client.hostname ?? ''}）` : r.client.hostname ?? ''))
    } catch (e) {
      setLookupError(e instanceof Error ? e.message : String(e))
    }
  }

  useEffect(() => {
    lookup(initialCode)
    api
      .agents()
      .then((all) => {
        const ext = all.filter((a) => a.origin === 'external' && !a.member.deactivated_at && (a.owner_id === me.id || me.is_admin))
        setMine(ext)
        if (ext[0]) setExisting(ext[0].member.id)
      })
      .catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const approve = async () => {
    if (!req) return
    setBusy(true)
    setError(null)
    try {
      const agent = await connectApi.approve(
        req.user_code,
        mode === 'existing'
          ? { agent_id: existing }
          : {
              handle: handle.trim(),
              display_name: name.trim() || undefined,
              description: description.trim(),
              expertise: expertise.split(/[,，、]/).map((s) => s.trim()).filter(Boolean),
            },
      )
      setResult({ ok: true, agent })
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
      setResult({ ok: false })
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  if (result) {
    return (
      <div className="page connect-page">
        <div className="connect-card">
          {result.ok ? (
            <>
              <div className="connect-icon ok">✓</div>
              <h1>已核准</h1>
              <p>
                <b>@{result.agent?.member.handle}</b> 已經接上。回到終端機，crewconvo 會自動完成設定並啟動背景 bridge，完成後這個 Agent 就會顯示為上線。
              </p>
            </>
          ) : (
            <>
              <div className="connect-icon">✕</div>
              <h1>已拒絕</h1>
              <p>這台電腦不會取得任何權限。</p>
            </>
          )}
          <button className="primary" onClick={onDone}>
            回到 CrewConvo
          </button>
        </div>
      </div>
    )
  }

  const pending = req?.status === 'pending'
  const canApprove = pending && (mode === 'existing' ? !!existing : handle.trim() && description.trim())

  return (
    <div className="page connect-page">
      <div className="connect-card">
        <h1>連接一台電腦</h1>
        <p className="muted">
          有人在終端機執行了 <code>crewconvo connect</code>。核准後，那台電腦上的 Agent（例如 Claude Code）就能以下面這個身分加入群聊，被 @ 時自動回應。
        </p>

        <label className="connect-code">
          <span>終端機上顯示的代碼</span>
          <input
            value={code}
            placeholder="ABCD-EFGH"
            onChange={(e) => {
              setCode(e.target.value.toUpperCase())
              lookup(e.target.value)
            }}
          />
        </label>
        {lookupError && <p className="form-error">{lookupError}</p>}

        {req && (
          <div className="connect-client">
            <div>
              <b>電腦</b>：{req.client.hostname ?? '（未提供）'}
              {req.client.user ? `（使用者 ${req.client.user}）` : ''}
            </div>
            <div>
              <b>系統</b>：{req.client.os ?? '—'}
            </div>
            <div>
              <b>偵測到的平台</b>：
              {req.client.platforms?.length ? req.client.platforms.map((p) => PLATFORM_NAMES[p] ?? p).join('、') : '沒有（會使用通用 MCP 設定）'}
            </div>
            {!pending && (
              <p className="pill-note warn">
                {req.status === 'expired' ? '代碼已過期，請在終端機重新執行 crewconvo connect。' : '這個代碼已經處理過了。'}
              </p>
            )}
            {pending && <p className="muted small">請確認終端機上的代碼和上面一致，而且是你自己（或你信任的人）剛剛執行的。</p>}
          </div>
        )}

        {pending && (
          <>
            <div className="connect-modes">
              <label>
                <input type="radio" checked={mode === 'new'} onChange={() => setMode('new')} /> 建立新的 Agent
              </label>
              <label className={mine.length ? '' : 'disabled'}>
                <input type="radio" checked={mode === 'existing'} disabled={!mine.length} onChange={() => setMode('existing')} /> 重新連接既有的外部 Agent
                {!mine.length && <span className="muted small">（你還沒有外部 Agent）</span>}
              </label>
            </div>

            {mode === 'new' ? (
              <div className="connect-form">
                <label>
                  <span>Handle（群聊裡用 @ 呼叫的名稱，可以用中文，不能有空白，不能和別人重複）</span>
                  <input value={handle} onChange={(e) => setHandle(e.target.value)} placeholder="sam-claude" />
                </label>
                <label>
                  <span>顯示名稱</span>
                  <input value={name} onChange={(e) => setName(e.target.value)} />
                </label>
                <label>
                  <span>它負責什麼（其他 Agent 與決策模型靠這段判斷要不要找它）</span>
                  <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Sam 筆電上的 Claude Code，熟悉前端專案與部署" />
                </label>
                <label>
                  <span>專長（逗號分隔，可留白）</span>
                  <input value={expertise} onChange={(e) => setExpertise(e.target.value)} placeholder="React, 部署" />
                </label>
                <p className="muted small">新 Agent 一開始不在任何頻道。核准後，到頻道的成員清單把它邀請進來，或直接私訊它。</p>
              </div>
            ) : (
              <div className="connect-form">
                <label>
                  <span>要連接的 Agent（它原本的 token 會失效）</span>
                  <select value={existing} onChange={(e) => setExisting(e.target.value)}>
                    {mine.map((a) => (
                      <option key={a.member.id} value={a.member.id}>
                        @{a.member.handle}（{a.member.display_name}）{a.status === 'online' ? '・上線中' : ''}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
            )}

            {error && <p className="form-error">{error}</p>}
            <div className="connect-actions">
              <button className="ghost" disabled={busy} onClick={deny}>
                拒絕
              </button>
              <button className="primary" disabled={busy || !canApprove} onClick={approve}>
                核准連線
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
