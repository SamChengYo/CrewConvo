import { useState, type FormEvent } from 'react'
import { api, type Member } from '../api'

export default function Login({ onLoggedIn }: { onLoggedIn: (m: Member) => void }) {
  const [mode, setMode] = useState<'login' | 'register'>('login')
  const [handle, setHandle] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

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

  return (
    <div className="login">
      <form className="login-card" onSubmit={submit}>
        <div className="brand">
          <span className="brand-mark">◆</span> CrewConvo
        </div>
        <p className="muted">人類與 AI Agent 一起工作的聊天室</p>
        {location.pathname === '/connect' && (
          <p className="pill-note">終端機上的 crewconvo 正在等待核准：登入後就會看到核准畫面。</p>
        )}
        <label>
          Handle
          <input value={handle} onChange={(e) => setHandle(e.target.value)} autoFocus autoComplete="username" />
        </label>
        {mode === 'register' && (
          <label>
            顯示名稱
            <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
          </label>
        )}
        <label>
          密碼
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete={mode === 'login' ? 'current-password' : 'new-password'}
          />
        </label>
        {error && <div className="form-error">{error}</div>}
        <button className="primary" disabled={busy || !handle || !password}>
          {mode === 'login' ? '登入' : '建立帳號'}
        </button>
        <button type="button" className="link" onClick={() => setMode(mode === 'login' ? 'register' : 'login')}>
          {mode === 'login' ? '還沒有帳號？建立一個' : '已有帳號？登入'}
        </button>
        {mode === 'register' && <p className="hint">第一個註冊的帳號會成為管理員。</p>}
      </form>
    </div>
  )
}
