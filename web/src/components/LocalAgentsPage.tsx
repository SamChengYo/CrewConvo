import { useCallback, useEffect, useState } from 'react'
import AgentTuning from './AgentTuning'
import {
  localApi,
  type LocalAccess,
  type LocalAgentsInfo,
  type LocalInstallResult,
  type LocalPlatform,
  type LocalProject,
  type LocalReadiness,
  type LocalRole,
  type LocalSession,
  type LocalWork,
} from '../api'

interface Props {
  onOpen: (conversationId: string) => void
  onError: (msg: string) => void
}

/** What the user typed for one coding agent's role; `expertise` is the comma-separated text. */
interface RoleDraft {
  description: string
  expertise: string
}

const STATES: Record<LocalReadiness, string> = {
  ready: '可以使用',
  needs_login: '需要登入',
  no_cli: '只能在它裡面使用',
  unknown: '無法確認',
}

const LEVELS: Record<LocalAccess, { label: string; hint: string }> = {
  read: { label: '唯讀', hint: '只能閱讀專案裡的檔案並回答，不能修改，也不能執行指令。' },
  edit: { label: '可修改檔案', hint: '可以在專案資料夾內建立與修改檔案。' },
  full: { label: '可修改檔案並執行指令', hint: '可以修改檔案，也可以執行指令（測試、建置、安裝套件，包含連網）。' },
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
  // The level chosen for a coding agent that is not connected yet (sent with install).
  const [levels, setLevels] = useState<Record<string, LocalAccess>>({})
  const [projects, setProjects] = useState<LocalProject[]>([])
  // Paths being typed, by conversation.
  const [paths, setPaths] = useState<Record<string, string>>({})
  const [work, setWork] = useState<LocalWork[]>([])
  const [workNote, setWorkNote] = useState<{ ok: boolean; text: string } | null>(null)
  const [checking, setChecking] = useState(false)

  // Folders and work change rarely and listing work runs git: not on every refresh.
  const loadWork = useCallback(() => {
    localApi
      .projects()
      .then((r) => setProjects(r.projects))
      .catch(() => {})
    localApi
      .work()
      .then((r) => setWork(r.work))
      .catch(() => {})
  }, [])

  const load = useCallback(
    (first: boolean, refresh = false) => {
      localApi
        .sessions()
        .then((s) => {
          setSessions(s.sessions)
          setReuse(s.reuse)
        })
        .catch(() => {})
      return localApi
        .list(refresh)
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
    loadWork()
    const t = setInterval(() => load(false), 5000)
    const w = setInterval(loadWork, 15000)
    return () => {
      clearInterval(t)
      clearInterval(w)
    }
  }, [load, loadWork])

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
        if (!picked.has(p.id) || p.agent) continue
        const role: LocalRole = {}
        if (d?.description.trim()) role.description = d.description.trim()
        if (d?.expertise.trim()) role.expertise = splitExpertise(d.expertise)
        if (levels[p.id] && p.limits_access) role.access = levels[p.id]
        if (role.description || role.expertise || role.access) roles[p.id] = role
      }
      const r = await localApi.install({ platforms: [...picked], group_name: group, channel_name: channel, roles })
      setResult(r)
      setPicked(new Set())
      setDrafts((d) => Object.fromEntries(Object.entries(d).filter(([id]) => !(id in roles))))
      await load(false)
      loadWork()
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

  const fail = (e: unknown) => onError(String((e as Error).message ?? e))

  const recheck = async () => {
    setChecking(true)
    await load(false, true)
    setChecking(false)
  }

  const setLevel = async (p: LocalPlatform, access: LocalAccess) => {
    if (!p.agent) {
      setLevels((l) => ({ ...l, [p.id]: access }))
      return
    }
    try {
      await localApi.setAccess(p.id, access)
      await load(false)
    } catch (e) {
      fail(e)
    }
  }

  // Work made from the folder a conversation is bound to now: it stays behind when the folder changes.
  const leftBehind = (c: LocalProject) => work.filter((w) => w.conversation_id === c.conversation_id && w.project === c.path)

  const confirmFolderChange = (c: LocalProject, to: string | null) => {
    if (!c.path) return true
    const left = leftBehind(c)
    const lines = [
      to ? `要把 ${c.name} 的專案資料夾改成別的資料夾嗎？` : `要移除 ${c.name} 的專案資料夾嗎？`,
      `原本：${c.path}`,
      ...(to ? [`改成：${to}`] : []),
      '',
      '・對話裡會出現一則變更通知；之前的訊息談的是原本的資料夾。',
      '・Coding Agent 在原本資料夾的工作階段不會帶到新的資料夾。',
    ]
    if (left.length > 0) lines.push(`・原本的資料夾還有 ${left.length} 份尚未合併的工作成果。它們會繼續列在「工作成果」，合併或捨棄仍然是針對原本的資料夾；建議先處理完再改。`)
    return confirm(lines.join('\n'))
  }

  const saveProject = async (c: LocalProject) => {
    const to = (paths[c.conversation_id] ?? '').trim()
    if (!confirmFolderChange(c, to)) return
    try {
      await localApi.setProject(c.conversation_id, paths[c.conversation_id] ?? '')
      setPaths((all) => Object.fromEntries(Object.entries(all).filter(([id]) => id !== c.conversation_id)))
      loadWork()
    } catch (e) {
      fail(e)
    }
  }

  const clearProject = async (c: LocalProject) => {
    if (!confirmFolderChange(c, null)) return
    try {
      await localApi.clearProject(c.conversation_id)
      setPaths((all) => Object.fromEntries(Object.entries(all).filter(([id]) => id !== c.conversation_id)))
      loadWork()
    } catch (e) {
      fail(e)
    }
  }

  const merge = async (w: LocalWork) => {
    setWorkNote(null)
    try {
      const r = await localApi.mergeWork(w.id)
      setWorkNote({ ok: true, text: `已把 ${r.branch} 合併進 ${r.into ?? '專案'}，工作副本和分支已經清掉。` })
    } catch (e) {
      // Refusals (uncommitted changes, another branch, a conflict) say what to do; keep them on screen.
      setWorkNote({ ok: false, text: String((e as Error).message ?? e) })
    }
    loadWork()
  }

  const discard = async (w: LocalWork) => {
    if (!confirm(`要捨棄 @${w.agent_handle} 在分支 ${w.branch} 上的工作嗎？\n${w.ahead} 個提交、工作副本和分支都會刪除，無法復原。`)) return
    setWorkNote(null)
    try {
      await localApi.discardWork(w.id)
    } catch (e) {
      setWorkNote({ ok: false, text: String((e as Error).message ?? e) })
    }
    loadWork()
    load(false)
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
          const level: LocalAccess = p.limits_access ? (p.agent?.access ?? levels[p.id] ?? 'read') : 'read'
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
              {p.state && (
                <div className="local-state">
                  <span className={`pill ${p.state === 'ready' ? 'good' : 'warn'}`}>{STATES[p.state]}</span>
                  <span className="muted small">
                    {p.explanation}
                    {p.state !== 'ready' && p.agent && ' 在可以使用之前，在頻道裡 @ 它不會得到回覆。'}
                  </span>
                  {p.login_command && <code className="local-command">{p.login_command}</code>}
                </div>
              )}
              {(p.agent || picked.has(p.id)) && (
                <label className="connect-field">
                  <span>可以對專案資料夾做什麼</span>
                  <select value={level} disabled={busy || !p.limits_access} onChange={(e) => setLevel(p, e.target.value as LocalAccess)}>
                    {(Object.keys(LEVELS) as LocalAccess[]).map((a) => (
                      <option key={a} value={a}>
                        {LEVELS[a].label}
                      </option>
                    ))}
                  </select>
                  <span className="local-hint">
                    {p.limits_access
                      ? `${LEVELS[level].hint}${level !== 'read' ? ' 只有你（它的擁有者）親自發的訊息才會用到這個權限，而且對話要先綁定專案資料夾；其他人或其他 Agent 叫它時一律唯讀。' : ''}${
                          p.id === 'codex' && level === 'edit' ? ' Codex 的沙箱無法只開放改檔：這一級它也能在專案資料夾內執行指令（沒有網路）。' : ''
                        }`
                      : `${p.name} 不支援權限設定：它照自己原本的設定執行，CrewConvo 只決定它在哪個資料夾工作。`}
                  </span>
                </label>
              )}
              {p.agent && <AgentTuning platform={p} onSaved={() => load(false)} onError={onError} />}
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

        <div className="local-recheck">
          {missing.length > 0 && <span className="muted small">沒有偵測到：{missing.map((p) => p.name).join('、')}</span>}
          <button className="ghost small" disabled={checking} onClick={recheck} title="重新確認每個 Coding Agent 是否已登入">
            {checking ? '檢查中…' : '重新檢查'}
          </button>
        </div>

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
                {r.ok && r.state !== 'ready' && (
                  <div className="small">
                    <b>⚠ {STATES[r.state]}：現在在頻道裡 @ 它還不會得到回覆。</b> {r.explanation}
                    {r.login_command && <code className="local-command">{r.login_command}</code>}
                  </div>
                )}
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

        {projects.length > 0 && (
          <section className="local-section">
            <h2>專案資料夾</h2>
            <p className="muted small">
              專案就是一個資料夾：綁定到同一個資料夾的對話屬於同一個專案，在側邊欄的「Coding Agent」底下列在一起。要開新專案或在專案裡多開一個對話，用側邊欄的「＋」比較快；這裡可以更改或移除每個對話的資料夾。沒有綁定的對話，Coding Agent 只能聊天（在自己的空白資料夾裡唯讀執行）。請填專案自己的資料夾的完整路徑，不能是整個磁碟或整個使用者資料夾。
            </p>
            {[...projects]
              .sort((a, b) => (a.path ?? '\uffff').localeCompare(b.path ?? '\uffff') || a.name.localeCompare(b.name))
              .map((c) => {
              const typed = paths[c.conversation_id]
              const value = typed ?? c.path ?? ''
              return (
                <div key={c.conversation_id} className="local-project">
                  <div className="local-project-name">
                    <b>{c.name}</b>
                    {c.project_name && <span className="tag" title={c.path ?? ''}>專案：{c.project_name}</span>}
                    <span className="muted small">{c.agents.map((a) => '@' + a).join('、') || '沒有這台電腦的 Coding Agent'}</span>
                    {c.path && <span className={`pill ${c.exists ? (c.is_git ? 'good' : '') : 'warn'}`}>{!c.exists ? '資料夾不存在' : c.is_git ? '是 git 專案' : '不是 git 專案'}</span>}
                  </div>
                  <div className="local-role-row">
                    <input
                      className="mono"
                      value={value}
                      placeholder="例如 C:\Users\me\projects\shop"
                      onChange={(e) => setPaths((all) => ({ ...all, [c.conversation_id]: e.target.value }))}
                    />
                    <button className="primary small" disabled={!value.trim() || value.trim() === (c.path ?? '')} onClick={() => saveProject(c)}>
                      儲存
                    </button>
                    <button className="ghost small" disabled={!c.path} onClick={() => clearProject(c)}>
                      清除
                    </button>
                  </div>
                  {c.path && c.exists && !c.is_git && (
                    <p className="muted small">不是 git 專案：可修改檔案的 Coding Agent 會直接改這個資料夾，沒有隔離，也無法列出改了什麼或還原。建議先在這個資料夾執行 git init 並提交一次。</p>
                  )}
                </div>
              )
            })}
          </section>
        )}

        {(work.length > 0 || workNote) && (
          <section className="local-section">
            <h2>工作成果</h2>
            <p className="muted small">
              可修改檔案的 Coding Agent 不會直接動你的專案：它在自己的工作副本（git worktree）和分支上修改並提交。看過之後按「合併」併入專案，或按「捨棄」整個丟掉。合併不會切換分支，也不會動你還沒提交的修改；有衝突時會自動取消。
            </p>
            {workNote && <p className={`pill-note ${workNote.ok ? 'ok' : 'warn'}`}>{workNote.text}</p>}
            {work.map((w) => (
              <div key={w.id} className="local-work">
                <div className="local-project-name">
                  <b>@{w.agent_handle}</b>
                  <span className="muted small">
                    {w.conversation_name}
                    {w.thread_root_id ? '（討論串）' : ''}・{when(w.last_used_at)}
                  </span>
                  <span className={`pill ${w.exists ? '' : 'warn'}`}>{!w.exists ? '工作副本不存在' : w.ahead > 0 ? `${w.ahead} 個提交` : '還沒有修改'}</span>
                  <span className="local-work-actions">
                    <button className="primary small" disabled={!w.exists || w.ahead === 0} onClick={() => merge(w)}>
                      合併
                    </button>
                    <button className="ghost small danger" onClick={() => discard(w)}>
                      捨棄
                    </button>
                  </span>
                </div>
                <div className="muted small mono">
                  {w.branch} → {w.base_branch ?? '（當時不在任何分支上）'}・{w.project}
                </div>
                {!w.current_folder && (
                  <p className="pill-note warn">這份工作屬於上面這個資料夾，它已經不是這個對話目前綁定的資料夾。合併會併入這個原本的資料夾，不是對話現在的資料夾。</p>
                )}
                {w.diffstat && <pre className="local-diffstat">{w.diffstat}</pre>}
              </div>
            ))}
          </section>
        )}
      </div>
    </div>
  )
}
