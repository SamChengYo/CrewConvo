import { useEffect, useRef, useState } from 'react'
import type { Member } from '../api'
import { Avatar } from './Avatar'

export default function TopBar({ me, onSearch, onLogout }: { me: Member; onSearch: (q: string) => void; onLogout: () => void }) {
  const [q, setQ] = useState('')
  const [menu, setMenu] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setMenu(false)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [])
  return (
    <header className="topbar">
      <div className="tb-left" />
      <form
        className="tb-search"
        onSubmit={(e) => {
          e.preventDefault()
          if (q.trim()) onSearch(q.trim())
        }}
      >
        <span className="tb-search-icon">⌕</span>
        <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="搜尋 CrewConvo 的訊息" />
      </form>
      <div className="tb-right" ref={ref}>
        <button className="tb-me" onClick={() => setMenu(!menu)} title={me.display_name}>
          <Avatar name={me.display_name} kind="human" size={26} presence="online" />
        </button>
        {menu && (
          <div className="tb-menu">
            <div className="tb-menu-head">
              <Avatar name={me.display_name} kind="human" size={36} />
              <div>
                <div className="tb-menu-name">{me.display_name}</div>
                <div className="muted small">
                  @{me.handle}
                  {me.is_admin ? ' · 管理員' : ''}
                </div>
              </div>
            </div>
            <button className="tb-menu-item" onClick={onLogout}>
              登出 CrewConvo
            </button>
          </div>
        )}
      </div>
    </header>
  )
}
