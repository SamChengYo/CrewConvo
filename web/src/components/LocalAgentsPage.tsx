import { useCallback, useEffect, useState } from 'react'
import { localApi, type LocalAgentsInfo, type LocalInstallResult } from '../api'

interface Props {
  onOpen: (conversationId: string) => void
  onError: (msg: string) => void
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

  const load = useCallback(
    (first: boolean) =>
      localApi
        .list()
        .then((i) => {
          setInfo(i)
          if (first) {
            setPicked(new Set(i.platforms.filter((p) => p.detected && !p.agent).map((p) => p.id)))
            setGroup(i.default_group)
            setChannel(i.default_channel)
          }
        })
        .catch((e) => onError(String(e.message ?? e))),
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
      const r = await localApi.install({ platforms: [...picked], group_name: group, channel_name: channel })
      setResult(r)
      setPicked(new Set())
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

        {found.map((p) => (
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
          </div>
        ))}

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
