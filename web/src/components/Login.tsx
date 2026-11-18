import { useEffect, useState, type FormEvent } from 'react'
import { api, authInfo, type AuthInfo, type Member } from '../api'

export default function Login({ onLoggedIn }: { onLoggedIn: (m: Member) => void }) {
  const [info, setInfo] = useState<AuthInfo | null>(null)
  const [mode, setMode] = useState<'login' | 'register'>('login')
  const [handle, setHandle] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    authInfo()
      .then((i) => {
        setInfo(i)
        // First run: there is no account yet, so the only thing to do is create one.
        if (i.needs_setup) setMode('register')
      })
      .catch(() => setInfo({ edition: 'enterprise', needs_setup: false, registration_open: true }))
  }, [])

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const m =
        mode === 'login' ? await api.login(handle, password) : await api.register(handle, displayName, password)
      onLoggedIn(m)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  if (!info) return <div className="splash">載入中…</div>
  const personal = info.edition === 'personal'
  const setup = info.needs_setup

  return (
    <div className="login">
      <form className="login-card" onSubmit={submit}>
        <div className="brand">
          <span className="brand-mark">◆</span> CrewConvo
          <span className={`edition-badge ${info.edition}`}>{personal ? 'Personal' : 'Enterprise'}</span>
        </div>
        <p className="muted">
          {setup
            ? personal
              ? '第一次使用：建立你的帳號。Personal 版只有這一個帳號，資料都留在這台電腦。'
              : '第一次使用：建立管理員帳號。之後其他成員可以自行註冊，或由你關閉註冊。'
            : personal
              ? '你和你的 AI Agent 的聊天室'
              : '團隊與 AI Agent 一起工作的聊天室'}
        </p>
        {location.pathname === '/connect' && (
          <p className="pill-note">終端機上的 crewconvo 正在等待核准：登入後就會看到核准畫面。</p>
        )}
        <label>
          {mode === 'register' ? '帳號名稱（登入與被 @ 時使用，不能有空白）' : '帳號名稱'}
          <input value={handle} onChange={(e) => setHandle(e.target.value)} autoFocus autoComplete="username" />
        </label>
        {mode === 'register' && (
          <label>
            顯示名稱
            <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
          </label>
        )}
        <label>
          密碼{mode === 'register' && '（至少 8 個字元）'}
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete={mode === 'login' ? 'current-password' : 'new-password'}
          />
        </label>
        {error && <div className="form-error">{error}</div>}
        <button className="primary" disabled={busy || !handle || !password}>
          {mode === 'login' ? '登入' : setup ? '建立帳號並開始' : '建立帳號'}
        </button>
        {!setup && info.registration_open && (
          <button type="button" className="link" onClick={() => setMode(mode === 'login' ? 'register' : 'login')}>
            {mode === 'login' ? '還沒有帳號？建立一個' : '已有帳號？登入'}
          </button>
        )}
        {!setup && !info.registration_open && !personal && <p className="hint">這個站台沒有開放註冊，需要帳號請聯絡管理員。</p>}
      </form>
    </div>
  )
}
