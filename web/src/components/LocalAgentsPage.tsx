import { useCallback, useEffect, useState } from 'react'
import { localApi, type LocalAgentsInfo, type LocalInstallResult, type LocalPlatform, type LocalRole, type LocalSession } from '../api'

interface Props {
  onOpen: (conversationId: string) => void
  onError: (msg: string) => void
}

/** What the user typed for one coding agent's role; `expertise` is the comma-separated text. */
interface RoleDraft {
  description: string
  expertise: string
}

function splitExpertise(text: string): string[] {
  return text
    .split(/[,，、]/)
    .map((s) => s.trim())
    .filter(Boolean)
}

function currentRole(p: LocalPlatform): RoleDraft {
  return { description: p.agent?.description ?? '', expertise: (p.agent?.expertise ?? []).join(', ') }
}

function when(iso: string): string {
  const minutes = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 60000))
  if (minutes < 1) return '剛剛'
  if (minutes < 60) return `${minutes} 分鐘前`
  if (minutes < 1440) return `${Math.floor(minutes / 60)} 小時前`
  return `${Math.floor(minutes / 1440)} 天前`
}

/** Coding agents on the computer CrewConvo runs on: pick, install, done. */
export default function LocalAgentsPage({ onOpen, onError }: Props) {
  const [info, setInfo] = useState<LocalAgentsInfo | null>(null)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [group, setGroup] = useState('')
  const [channel, setChannel] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [result, setResult] = useState<LocalInstallResult | null>(null)
  // Only what the user has edited and not saved yet; everything else shows the stored value.
  const [drafts, setDrafts] = useState<Record<string, RoleDraft>>({})
  const [sessions, setSessions] = useState<LocalSession[]>([])
  const [reuse, setReuse] = useState(true)

  const load = useCallback(
    (first: boolean) => {
      localApi
        .sessions()
        .then((s) => {
          setSessions(s.sessions)
          setReuse(s.reuse)
        })
        .catch(() => {})
      return localApi
        .list()
        .then((i) => {
          setInfo(i)
          if (first) {
            setPicked(new Set(i.platforms.filter((p) => p.detected && !p.agent).map((p) => p.id)))
            setGroup(i.default_group)
            setChannel(i.default_channel)
          }
        })
        .catch((e) => onError(String(e.message ?? e)))
    },
    [onError],
  )

  useEffect(() => {
    load(true)
    const t = setInterval(() => load(false), 5000)
    return () => clearInterval(t)
  }, [load])

  if (!info) return <div className="page connect-page" />

  const toggle = (id: string) =>
    setPicked((s) => {
      const n = new Set(s)
      if (n.has(id)) n.delete(id)
      else n.add(id)
      return n
    })

  const install = async () => {
    setBusy(true)
    setError('')
    setResult(null)
    try {
      // Roles of agents that are already connected are saved with their own button.
      const roles: Record<string, LocalRole> = {}
      for (const p of info.platforms) {
        const d = drafts[p.id]
        if (!picked.has(p.id) || p.agent || !d) continue
        const role: LocalRole = {}
        if (d.description.trim()) role.description = d.description.trim()
        if (d.expertise.trim()) role.expertise = splitExpertise(d.expertise)
        if (role.description || role.expertise) roles[p.id] = role
      }
      const r = await localApi.install({ platforms: [...picked], group_name: group, channel_name: channel, roles })
      setResult(r)
      setPicked(new Set())
      setDrafts((d) => Object.fromEntries(Object.entries(d).filter(([id]) => !(id in roles))))
      await load(false)
    } catch (e) {
      setError(String((e as Error).message ?? e))
    } finally {
      setBusy(false)
    }
  }

  const remove = async (id: string, name: string) => {
    if (!confirm(`要中斷 ${name} 嗎？\n會移除寫進 ${name} 的 CrewConvo 設定，並刪除它在群聊裡的 Agent。`)) return
    setBusy(true)
    try {
      await localApi.remove(id)
      setResult(null)
      await load(false)
    } catch (e) {
      onError(String((e as Error).message ?? e))
    } finally {
      setBusy(false)
    }
  }

  const edit = (p: LocalPlatform, change: Partial<RoleDraft>) =>
    setDrafts((d) => ({ ...d, [p.id]: { ...(d[p.id] ?? currentRole(p)), ...change } }))

  const saveRole = async (p: LocalPlatform) => {
    const d = drafts[p.id]
    if (!d) return
    setBusy(true)
    try {
      await localApi.setRole(p.id, { description: d.description.trim(), expertise: splitExpertise(d.expertise) })
      await load(false)
      setDrafts((all) => Object.fromEntries(Object.entries(all).filter(([id]) => id !== p.id)))
    } catch (e) {
      onError(String((e as Error).message ?? e))
    } finally {
      setBusy(false)
    }
  }

  const forget = async (s: LocalSession) => {
    try {
      await localApi.forgetSession(s.id)
      setSessions((all) => all.filter((x) => x.id !== s.id))
    } catch (e) {
      onError(String((e as Error).message ?? e))
    }
  }

  const found = info.platforms.filter((p) => p.detected)
  const missing = info.platforms.filter((p) => !p.detected)

  return (
    <div className="page connect-page">
      <div className="connect-card wide">
        <h1>這台電腦上的 Coding Agent</h1>
        <p className="muted">
          勾選要加入群聊的 Coding Agent，按「安裝」。CrewConvo 會替每一個建立 Agent、把連線設定寫進它，並建立一個群組和一個頻道。之後在頻道裡 @ 它，它就會在這台電腦上被叫起來回覆。
        </p>

        {found.length === 0 && <p className="pill-note warn">這台電腦上沒有找到支援的 Coding Agent。安裝其中一個之後重新整理這一頁。</p>}

        {found.map((p) => {
          const stored = currentRole(p)
          const role = drafts[p.id] ?? stored
          const changed = role.description.trim() !== stored.description || splitExpertise(role.expertise).join(',') !== (p.agent?.expertise ?? []).join(',')
          const own = p.agent ? sessions.filter((s) => s.agent_id === p.agent!.agent_id) : []
          return (
            <div key={p.id} className="connect-agent">
              <div className="connect-agent-head">
                <label className="local-pick">
                  {!p.agent && <input type="checkbox" checked={picked.has(p.id)} disabled={busy} onChange={() => toggle(p.id)} />}
                  <b>{p.name}</b>
                </label>
                <span className="muted small">{p.description}</span>
                {p.agent ? (
                  <>
                    <span className={`status ${p.agent.online ? 'online' : 'offline'}`}>
                      @{p.agent.handle}・{p.agent.online ? '已連線' : '未連線'}
                    </span>
                    <button className="ghost small" disabled={busy} onClick={() => toggle(p.id)}>
                      {picked.has(p.id) ? '取消重新安裝' : '重新安裝'}
                    </button>
                    <button className="ghost small danger" disabled={busy} onClick={() => remove(p.id, p.name)}>
                      中斷
                    </button>
                  </>
                ) : (
                  <span className="status offline">尚未加入</span>
                )}
              </div>
              {!p.can_wake && (
                <p className="muted small">
                  找不到它的指令列工具，所以被 @ 時無法自動回覆；仍然可以在它裡面主動使用 CrewConvo。{p.note}
                </p>
              )}
              {p.can_wake && p.note && <p className="muted small">{p.note}</p>}
              {(p.agent || picked.has(p.id)) && (
                <div className="local-role">
                  <label className="connect-field">
                    <span>它負責什麼</span>
                    <textarea
                      rows={2}
                      value={role.description}
                      placeholder={`例如：負責前端：React、CSS、畫面調整（不填就是「${p.default_role}」）`}
                      onChange={(e) => edit(p, { description: e.target.value })}
                    />
                  </label>
                  <div className="local-role-row">
                    <label className="connect-field">
                      <span>專長（選填，用逗號分隔）</span>
                      <input value={role.expertise} placeholder="例如：frontend, React, CSS" onChange={(e) => edit(p, { expertise: e.target.value })} />
                    </label>
                    {p.agent && (
                      <button className="primary small" disabled={busy || !changed || !role.description.trim()} onClick={() => saveRole(p)}>
                        儲存
                      </button>
                    )}
                  </div>
                  <p className="muted small">
                    這段說明會寫進它每次被叫起來時的指示，其他成員和決策模型也是看這段說明來決定要找誰。{p.agent ? '儲存後從下一次被叫起來開始生效。' : ''}
                  </p>
                </div>
              )}
              {p.agent && p.keeps_sessions && (
                <details className="local-sessions">
                  <summary>
                    工作階段（{own.length}）{!reuse && <span className="muted">・目前設定為每次都開新的</span>}
                  </summary>
                  {own.length === 0 && <p className="muted small">還沒有紀錄。它被叫起來一次之後，這裡會出現那次開的工作階段；之後相關的喚醒會接續它，而不是從頭開始。</p>}
                  {own.map((s) => (
                    <div key={s.id} className="local-session">
                      <div className="local-session-text">
                        <div>{s.purpose}</div>
                        <div className="muted small">
                          {s.conversation_name}
                          {s.thread_root_id ? '（討論串）' : ''}・上次使用 {when(s.last_used_at)}・用了 {s.uses} 次
                        </div>
                      </div>
                      <button className="ghost small" title="不再接續這個工作階段；下次會開新的" onClick={() => forget(s)}>
                        忘記
                      </button>
                    </div>
                  ))}
                </details>
              )}
            </div>
          )
        })}

        {missing.length > 0 && <p className="muted small">沒有偵測到：{missing.map((p) => p.name).join('、')}</p>}

        {picked.size > 0 && (
          <div className="connect-two">
            <label className="connect-field">
              <span>Agent 群組（讓這些 Agent 互相找得到）</span>
              <input value={group} onChange={(e) => setGroup(e.target.value)} />
            </label>
            <label className="connect-field">
              <span>頻道（你和這些 Agent 的私有頻道）</span>
              <input value={channel} onChange={(e) => setChannel(e.target.value)} />
            </label>
          </div>
        )}

        {error && <p className="form-error">{error}</p>}

        {result && (
          <div className="local-result">
            {result.results.map((r) => (
              <div key={r.platform} className={`pill-note ${r.ok ? 'ok' : 'warn'}`}>
                <b>
                  {r.ok ? '✓' : '✗'} {r.name}（@{r.handle}）
                </b>
                {r.lines.map((l, i) => (
                  <div key={i} className="small">
                    {l}
                  </div>
                ))}
              </div>
            ))}
            {result.warnings.map((w, i) => (
              <p key={i} className="pill-note warn">
                {w}
              </p>
            ))}
            <p className="muted small">已經開著的 Coding Agent 要重新啟動，才會讀到新的設定。</p>
          </div>
        )}

        <div className="connect-actions">
          {result?.channel && (
            <button className="ghost" onClick={() => onOpen(result.channel!.id)}>
              前往 #{result.channel.name}
            </button>
          )}
          <button className="primary" disabled={busy || picked.size === 0} onClick={install}>
            {busy ? '安裝中…' : `安裝${picked.size > 0 ? `（${picked.size}）` : ''}`}
          </button>
        </div>
      </div>
    </div>
  )
}
