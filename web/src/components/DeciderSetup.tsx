import { useCallback, useEffect, useState } from 'react'
import type { Member } from '../api'
import DecidersPage from './DecidersPage'

interface Props {
  me: Member
  /** A decision model that can be called exists now. */
  ready: boolean
  /** Ask the server again whether one exists. */
  onChanged: () => void
  onContinue: () => void
  onLogout: () => void
}

/** The only screen until the workspace has a decision model: without one the app cannot be used. */
export default function DeciderSetup({ me, ready, onChanged, onContinue, onLogout }: Props) {
  const [error, setError] = useState<string | null>(null)
  const showError = useCallback((m: string) => setError(m), [])

  useEffect(() => {
    if (me.is_admin) return
    // Someone else has to set it up: look again now and then, so this screen goes away by itself.
    const t = setInterval(onChanged, 10000)
    return () => clearInterval(t)
  }, [me.is_admin, onChanged])

  return (
    <div className="setup-screen">
      <div className="setup-card">
        <header className="setup-head">
          <div>
            <div className="setup-step">開始使用前</div>
            <h1>先設定決策模型</h1>
          </div>
          <button className="ghost" onClick={onLogout}>
            登出
          </button>
        </header>
        <p>
          在頻道裡發訊息但沒有 @ 任何人時，由「決策模型」判斷這則訊息該由哪一個 Agent 來回答。它也用來決定 Coding Agent 被叫起來時要接續哪一個工作階段。沒有決策模型，CrewConvo 無法使用。
        </p>
        {me.is_admin ? (
          <>
            <p className="muted">
              你需要一把 API 金鑰：Gemini、OpenAI，或任何 OpenAI 相容端點（例如自己架的模型）都可以。金鑰只存在這台伺服器上，加密保存。
            </p>
            {error && (
              <p className="form-error" onClick={() => setError(null)}>
                {error}
              </p>
            )}
            <DecidersPage embedded onError={showError} onChanged={onChanged} />
            <div className="setup-actions">
              <span className={ready ? 'pill-note ok' : 'pill-note warn'}>
                {ready ? '✓ 已經有可以使用的決策模型' : '還沒有可以使用的決策模型：至少要有一筆啟用的設定（Decisions API 還要填入金鑰）'}
              </span>
              <button className="primary" disabled={!ready} onClick={onContinue}>
                開始使用
              </button>
            </div>
          </>
        ) : (
          <p className="pill-note warn">這個站台還沒有設定決策模型。請聯絡管理員完成設定，設定好之後這一頁會自動消失。</p>
        )}
      </div>
    </div>
  )
}
