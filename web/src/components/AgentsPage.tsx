import { useCallback, useEffect, useState } from 'react'
import { api, api2, type Agent, type AgentCredentials, type ConversationItem, type Member, type SystemInfo } from '../api'
import { useRealtime } from '../realtime'
import { Avatar } from './Avatar'
import { Modal } from './Modals'
import { CopyBlock } from './CopyBlock'

const CHAT_TOOLS = ['list_agents', 'list_conversations', 'list_channels', 'join_channel', 'create_channel', 'open_dm', 'invite', 'leave', 'post_message', 'react', 'read_history', 'search_messages']
const SANDBOX_TOOLS = ['bash', 'read_file', 'write_file', 'list_dir']

interface Props {
  me: Member
  convs: ConversationItem[]
  onError: (msg: string) => void
  onOpenDm: (conversationId: string) => void
}

type Editing =
  | { kind: 'builtin'; agent: Agent | null }
  | { kind: 'external' }
  | { kind: 'sandbox'; agent: Agent }
  | { kind: 'delivery'; agent: Agent }
  | { kind: 'delete'; agent: Agent }
  | null

export default function AgentsPage({ me, convs, onError, onOpenDm }: Props) {
  const [agents, setAgents] = useState<Agent[]>([])
  const [models, setModels] = useState<{ providers: string[]; examples: string[] }>({ providers: [], examples: [] })
  const [system, setSystem] = useState<SystemInfo | null>(null)
  const [editing, setEditing] = useState<Editing>(null)
  const [creds, setCreds] = useState<AgentCredentials | null>(null)

  const reload = useCallback(() => {
    api.agents().then(setAgents).catch((e) => onError(String(e)))
  }, [onError])

  useEffect(() => {
    reload()
    api.models().then(setModels).catch(() => {})
    api2.system().then(setSystem).catch(() => {})
  }, [reload])

  useRealtime((ev) => {
    if (ev.type === 'agents_changed') reload()
  })

  const fail = (e: unknown) => onError(e instanceof Error ? e.message : String(e))
  const canManage = (a: Agent) => a.owner_id === me.id || me.is_admin
  const openDm = (a: Agent) => api.openDm([a.member.id]).then((c) => onOpenDm(c.id)).catch(fail)
  const usable = [...models.examples].sort(
    (a, b) => Number(models.providers.includes(b.split(':')[0])) - Number(models.providers.includes(a.split(':')[0])),
  )

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1>Agent 管理</h1>
          <p className="muted">
            內建 Agent 由平台代管執行；外部 Agent 是你自己的程式，透過 WebSocket / Webhook 收事件、用 REST 或 MCP 執行動作。兩者走同一套協定，發言都只能透過 <code>post_message</code>。
          </p>
        </div>
        <div className="head-actions">
          <button className="ghost" onClick={() => setEditing({ kind: 'external' })}>
            ＋ 外部 Agent
          </button>
          <button className="primary" onClick={() => setEditing({ kind: 'builtin', agent: null })}>
            ＋ 內建 Agent
          </button>
        </div>
      </header>
      <p className="muted small">
        模型供應商：{models.providers.length ? models.providers.join('、') : '（無）'}。沙箱：{system?.sandbox ? '可用' : '未啟用'}。對話摘要：
        {system?.summaries ? `啟用（${system.summary_model}）` : '未啟用'}。
      </p>
      <div className="agent-grid">
        {agents.map((a) => (
          <div key={a.member.id} className="agent-card">
            <div className="agent-card-head">
              <Avatar name={a.member.display_name} kind="agent" size={40} />
              <div>
                <div className="author">
                  {a.member.display_name} <span className="badge">{a.origin === 'builtin' ? 'AI' : '外部'}</span>
                </div>
                <div className="muted small">@{a.member.handle}</div>
              </div>
              <span className={`status ${a.status}`}>{a.status}</span>
            </div>
            <p>{a.description}</p>
            {a.expertise.length > 0 && (
              <div className="tags">
                {a.expertise.map((t) => (
                  <span key={t} className="tag">
                    {t}
                  </span>
                ))}
              </div>
            )}
            <div className="muted small">
              {a.origin === 'builtin' ? (
                <>
                  模型 <code>{a.model}</code>
                </>
              ) : (
                <>
                  {a.delivery === 'webhook' ? `Webhook → ${a.webhook_url}` : 'WebSocket'} · token{' '}
                  {a.has_token ? <code>{a.token_prefix}…</code> : '已撤銷'}
                  {a.last_seen_at ? ` · 最後連線 ${new Date(a.last_seen_at).toLocaleString()}` : ''}
                </>
              )}{' '}
              · 逾時 {a.max_wake_seconds}s{a.daily_token_budget ? ` · 每日 ${a.daily_token_budget.toLocaleString()} tokens` : ''}
            </div>
            <div className="card-actions">
              <button className="ghost" onClick={() => openDm(a)}>
                私訊
              </button>
              {canManage(a) && a.origin === 'builtin' && (
                <>
                  <button className="ghost" onClick={() => setEditing({ kind: 'builtin', agent: a })}>
                    編輯
                  </button>
                  {system?.sandbox && (
                    <button className="ghost" onClick={() => setEditing({ kind: 'sandbox', agent: a })}>
                      沙箱
                    </button>
                  )}
                </>
              )}
              {canManage(a) && a.origin === 'external' && (
                <>
                  <button className="ghost" onClick={() => api2.rotateToken(a.member.id).then(setCreds).catch(fail)}>
                    {a.has_token ? '輪替 token' : '產生 token'}
                  </button>
                  {a.has_token && (
                    <button
                      className="ghost"
                      onClick={() => {
                        if (confirm('撤銷後此 Agent 會立即斷線，確定？')) api2.revokeToken(a.member.id).then(reload).catch(fail)
                      }}
                    >
                      撤銷
                    </button>
                  )}
                  <button className="ghost" onClick={() => setEditing({ kind: 'delivery', agent: a })}>
                    傳送方式
                  </button>
                </>
              )}
              {canManage(a) && (
                <button className="ghost danger-text" onClick={() => setEditing({ kind: 'delete', agent: a })}>
                  刪除
                </button>
              )}
            </div>
          </div>
        ))}
        {agents.length === 0 && <div className="empty-hint">還沒有 Agent。建立一個，然後在頻道裡 @ 它。</div>}
      </div>

      {editing?.kind === 'builtin' && (
        <AgentForm
          agent={editing.agent}
          sandbox={!!system?.sandbox}
          convs={convs.filter((c) => c.kind === 'channel')}
          examples={usable}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null)
            reload()
          }}
        />
      )}
      {editing?.kind === 'external' && (
        <ExternalForm
          convs={convs.filter((c) => c.kind === 'channel')}
          onClose={() => setEditing(null)}
          onCreated={(c) => {
            setEditing(null)
            setCreds(c)
            reload()
          }}
        />
      )}
      {editing?.kind === 'sandbox' && (
        <SandboxForm agent={editing.agent} isAdmin={me.is_admin} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), reload())} />
      )}
      {editing?.kind === 'delivery' && (
        <DeliveryForm
          agent={editing.agent}
          onClose={() => setEditing(null)}
          onSaved={(c) => {
            setEditing(null)
            if (c.webhook_secret) setCreds(c)
            reload()
          }}
        />
      )}
      {editing?.kind === 'delete' && (
        <DeleteAgent agent={editing.agent} onClose={() => setEditing(null)} onDeleted={() => (setEditing(null), reload())} />
      )}
      {creds && <CredentialsModal creds={creds} onClose={() => setCreds(null)} />}
    </div>
  )
}

function Copy({ text }: { text: string }) {
  const [done, setDone] = useState(false)
  return (
    <div className="secret">
      <code>{text}</code>
      <button
        className="ghost small"
        onClick={() => {
          navigator.clipboard?.writeText(text).then(() => setDone(true))
        }}
      >
        {done ? '已複製' : '複製'}
      </button>
    </div>
  )
}

function CredentialsModal({ creds, onClose }: { creds: AgentCredentials; onClose: () => void }) {
  const [origin, setOrigin] = useState(location.origin)
  useEffect(() => {
    api2
      .system()
      .then((s) => s.public_base_url && setOrigin(s.public_base_url.replace(/\/$/, '')))
      .catch(() => {})
  }, [])
  return (
    <Modal title={`@${creds.agent.member.handle} 的憑證`} onClose={onClose}>
      <p className="form-error">只會顯示這一次，請現在保存。</p>
      {creds.token && (
        <>
          <div className="field-label">Agent token（Authorization: Bearer …）</div>
          <Copy text={creds.token} />
        </>
      )}
      {creds.webhook_secret && (
        <>
          <div className="field-label">Webhook 簽章密鑰（HMAC-SHA256）</div>
          <Copy text={creds.webhook_secret} />
        </>
      )}
      {creds.token && (
        <>
          <div className="field-label">在那台電腦上用這個 token 連線（一般建議直接執行 crewconvo setup，在瀏覽器核准即可，不用複製 token）</div>
          <CopyBlock text={`crewconvo setup ${origin} --token ${creds.token}`} />
          <p className="muted small">安裝 crewconvo 的方式請看專案 README 的「連接 Coding Agent」。</p>
        </>
      )}
      <div className="field-label">端點</div>
      <ul className="endpoints">
        <li>
          事件：<code>{origin.replace('http', 'ws')}/agent/v1/ws</code>
        </li>
        <li>
          MCP：<code>{origin}/mcp</code>
        </li>
        <li>
          REST 動作：<code>POST {origin}/agent/v1/tools/&lt;tool&gt;</code>
        </li>
        <li>
          回報：<code>POST {origin}/agent/v1/replies</code>
        </li>
      </ul>
      <p className="muted small">完整說明見 docs/external-agents.md。</p>
      <footer>
        <button className="primary" onClick={onClose}>
          我已保存
        </button>
      </footer>
    </Modal>
  )
}

function ExternalForm({ convs, onClose, onCreated }: { convs: ConversationItem[]; onClose: () => void; onCreated: (c: AgentCredentials) => void }) {
  const [handle, setHandle] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [description, setDescription] = useState('')
  const [expertise, setExpertise] = useState('')
  const [delivery, setDelivery] = useState<'websocket' | 'webhook'>('websocket')
  const [webhookUrl, setWebhookUrl] = useState('')
  const [join, setJoin] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)
  const create = async () => {
    try {
      onCreated(
        await api2.createExternalAgent({
          handle,
          display_name: displayName,
          description,
          expertise: expertise.split(/[,，、]/).map((s) => s.trim()).filter(Boolean),
          delivery,
          webhook_url: delivery === 'webhook' ? webhookUrl : null,
          join_conversations: join,
        }),
      )
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }
  return (
    <Modal title="建立外部 Agent" onClose={onClose}>
      <div className="form-grid">
        <label>
          Handle
          <input value={handle} onChange={(e) => setHandle(e.target.value)} autoFocus />
        </label>
        <label>
          顯示名稱
          <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
        </label>
        <label className="span2">
          描述（必填）
          <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        <label className="span2">
          專長關鍵字
          <input value={expertise} onChange={(e) => setExpertise(e.target.value)} />
        </label>
        <label>
          事件傳送方式
          <select value={delivery} onChange={(e) => setDelivery(e.target.value as 'websocket' | 'webhook')}>
            <option value="websocket">WebSocket（Agent 主動連線）</option>
            <option value="webhook">Webhook（平台 POST 到你的網址）</option>
          </select>
        </label>
        {delivery === 'webhook' && (
          <label>
            Webhook 網址（https）
            <input value={webhookUrl} onChange={(e) => setWebhookUrl(e.target.value)} className="mono" />
          </label>
        )}
        {convs.length > 0 && (
          <div className="span2">
            <div className="field-label">加入頻道</div>
            <div className="checks">
              {convs.map((c) => (
                <label key={c.id} className="row">
                  <input type="checkbox" checked={join.includes(c.id)} onChange={() => setJoin(join.includes(c.id) ? join.filter((x) => x !== c.id) : [...join, c.id])} /># {c.name}
                </label>
              ))}
            </div>
          </div>
        )}
      </div>
      {error && <div className="form-error">{error}</div>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" disabled={!handle || !description.trim()} onClick={create}>
          建立並產生 token
        </button>
      </footer>
    </Modal>
  )
}

function DeliveryForm({ agent, onClose, onSaved }: { agent: Agent; onClose: () => void; onSaved: (c: AgentCredentials) => void }) {
  const [delivery, setDelivery] = useState(agent.delivery ?? 'websocket')
  const [url, setUrl] = useState(agent.webhook_url ?? '')
  const [rotate, setRotate] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const save = async () => {
    try {
      onSaved(await api2.updateDelivery(agent.member.id, { delivery, webhook_url: delivery === 'webhook' ? url : null, rotate_secret: rotate }))
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }
  return (
    <Modal title={`@${agent.member.handle} 的傳送方式`} onClose={onClose}>
      <label>
        方式
        <select value={delivery} onChange={(e) => setDelivery(e.target.value)}>
          <option value="websocket">WebSocket</option>
          <option value="webhook">Webhook</option>
        </select>
      </label>
      {delivery === 'webhook' && (
        <>
          <label>
            Webhook 網址
            <input value={url} onChange={(e) => setUrl(e.target.value)} className="mono" />
          </label>
          <label className="row">
            <input type="checkbox" checked={rotate} onChange={(e) => setRotate(e.target.checked)} /> 產生新的簽章密鑰
          </label>
        </>
      )}
      {error && <div className="form-error">{error}</div>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" onClick={save}>
          儲存
        </button>
      </footer>
    </Modal>
  )
}

function SandboxForm({ agent, isAdmin, onClose, onSaved }: { agent: Agent; isAdmin: boolean; onClose: () => void; onSaved: () => void }) {
  const cfg = agent.sandbox_config as { cpus?: number; memory_mb?: number; pids?: number; disk_mb?: number; network?: string }
  const [cpus, setCpus] = useState(String(cfg.cpus ?? 1))
  const [mem, setMem] = useState(String(cfg.memory_mb ?? 512))
  const [pids, setPids] = useState(String(cfg.pids ?? 256))
  const [disk, setDisk] = useState(String(cfg.disk_mb ?? 1024))
  const [network, setNetwork] = useState(cfg.network ?? 'none')
  const [error, setError] = useState<string | null>(null)
  const save = async () => {
    try {
      await api2.updateSandbox(agent.member.id, { cpus: Number(cpus), memory_mb: Number(mem), pids: Number(pids), disk_mb: Number(disk), network })
      onSaved()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }
  return (
    <Modal title={`@${agent.member.handle} 的沙箱`} onClose={onClose}>
      <p className="muted small">
        每個內建 Agent 有自己的容器與 /workspace（非 root、唯讀根檔案系統、無特權）。變更在容器下次啟動時生效；/workspace 內容保留。
      </p>
      <div className="form-grid">
        <label>
          CPU（核）
          <input type="number" step="0.5" min={0.1} value={cpus} onChange={(e) => setCpus(e.target.value)} />
        </label>
        <label>
          記憶體（MB）
          <input type="number" min={64} value={mem} onChange={(e) => setMem(e.target.value)} />
        </label>
        <label>
          程序數上限
          <input type="number" min={16} value={pids} onChange={(e) => setPids(e.target.value)} />
        </label>
        <label>
          磁碟配額（MB）
          <input type="number" min={16} value={disk} onChange={(e) => setDisk(e.target.value)} />
        </label>
        <label className="span2">
          網路
          <select value={network} onChange={(e) => setNetwork(e.target.value)} disabled={!isAdmin && network === 'none'}>
            <option value="none">無對外連線（預設）</option>
            <option value="open" disabled={!isAdmin}>
              開放對外連線（僅管理員）
            </option>
          </select>
        </label>
      </div>
      {error && <div className="form-error">{error}</div>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" onClick={save}>
          儲存
        </button>
      </footer>
    </Modal>
  )
}

function DeleteAgent({ agent, onClose, onDeleted }: { agent: Agent; onClose: () => void; onDeleted: () => void }) {
  const [typed, setTyped] = useState('')
  const [error, setError] = useState<string | null>(null)
  return (
    <Modal title={`刪除 @${agent.member.handle}`} onClose={onClose}>
      <p>
        刪除後此 Agent 會離開所有對話、token 失效
        {agent.origin === 'builtin' && (
          <>
            ，並且<strong>永久刪除它的沙箱與 /workspace 資料（含長期筆記）</strong>
          </>
        )}
        。歷史訊息會保留。此動作無法復原。
      </p>
      <label>
        輸入 <code>{agent.member.handle}</code> 以確認
        <input value={typed} onChange={(e) => setTyped(e.target.value)} autoFocus />
      </label>
      {error && <div className="form-error">{error}</div>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button
          className="danger"
          disabled={typed !== agent.member.handle}
          onClick={() =>
            api2
              .deleteAgent(agent.member.id)
              .then(onDeleted)
              .catch((e) => setError(String(e)))
          }
        >
          永久刪除
        </button>
      </footer>
    </Modal>
  )
}

function AgentForm({
  agent,
  sandbox,
  convs,
  examples,
  onClose,
  onSaved,
}: {
  agent: Agent | null
  sandbox: boolean
  convs: ConversationItem[]
  examples: string[]
  onClose: () => void
  onSaved: () => void
}) {
  const [handle, setHandle] = useState(agent?.member.handle ?? '')
  const [displayName, setDisplayName] = useState(agent?.member.display_name ?? '')
  const [description, setDescription] = useState(agent?.description ?? '')
  const [expertise, setExpertise] = useState(agent?.expertise.join(', ') ?? '')
  const [model, setModel] = useState(agent?.model ?? examples[0] ?? '')
  const [prompt, setPrompt] = useState(agent?.system_prompt ?? '')
  const [tools, setTools] = useState<string[]>(agent?.tools ?? [])
  const [wakeSecs, setWakeSecs] = useState(String(agent?.max_wake_seconds ?? 300))
  const [budget, setBudget] = useState(agent?.daily_token_budget ? String(agent.daily_token_budget) : '')
  const [status, setStatus] = useState(agent?.status ?? 'online')
  const [join, setJoin] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)

  const save = async () => {
    const common = {
      display_name: displayName,
      description,
      expertise: expertise
        .split(/[,，、]/)
        .map((s) => s.trim())
        .filter(Boolean),
      model,
      system_prompt: prompt,
      tools,
      max_wake_seconds: Number(wakeSecs),
      daily_token_budget: budget ? Number(budget) : null,
    }
    try {
      if (agent) await api.updateAgent(agent.member.id, { ...common, status })
      else await api.createAgent({ ...common, handle, join_conversations: join })
      onSaved()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  const toggle = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v])
  const allTools = sandbox ? [...CHAT_TOOLS, ...SANDBOX_TOOLS] : CHAT_TOOLS

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className="modal wide" onMouseDown={(e) => e.stopPropagation()}>
        <header>
          <h2>{agent ? `編輯 @${agent.member.handle}` : '建立內建 Agent'}</h2>
          <button className="ghost" onClick={onClose}>
            ✕
          </button>
        </header>
        <div className="form-grid">
          {!agent && (
            <label>
              Handle
              <input value={handle} onChange={(e) => setHandle(e.target.value)} placeholder="例如 researcher" autoFocus />
            </label>
          )}
          <label>
            顯示名稱
            <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
          </label>
          <label className="span2">
            描述（必填，其他 Agent 與決策模型依此判斷誰該回應）
            <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
          </label>
          <label>
            專長關鍵字（逗號分隔）
            <input value={expertise} onChange={(e) => setExpertise(e.target.value)} />
          </label>
          <label>
            模型
            <input list="model-examples" value={model} onChange={(e) => setModel(e.target.value)} />
            <datalist id="model-examples">
              {examples.map((e) => (
                <option key={e} value={e} />
              ))}
            </datalist>
          </label>
          <label className="span2">
            System prompt
            <textarea rows={5} value={prompt} onChange={(e) => setPrompt(e.target.value)} />
          </label>
          <label>
            單次喚醒逾時（秒）
            <input type="number" min={10} max={1800} value={wakeSecs} onChange={(e) => setWakeSecs(e.target.value)} />
          </label>
          <label>
            每日 token 上限（空白＝不限）
            <input type="number" min={0} value={budget} onChange={(e) => setBudget(e.target.value)} />
          </label>
          {agent && (
            <label>
              狀態
              <select value={status} onChange={(e) => setStatus(e.target.value as Agent['status'])}>
                <option value="online">online</option>
                <option value="disabled">disabled</option>
              </select>
            </label>
          )}
          <div className="span2">
            <div className="field-label">可用工具（都不勾＝全部）</div>
            <div className="checks">
              {allTools.map((t) => (
                <label key={t} className="row">
                  <input type="checkbox" checked={tools.includes(t)} onChange={() => setTools(toggle(tools, t))} />
                  <code>{t}</code>
                </label>
              ))}
            </div>
          </div>
          {!agent && convs.length > 0 && (
            <div className="span2">
              <div className="field-label">加入頻道</div>
              <div className="checks">
                {convs.map((c) => (
                  <label key={c.id} className="row">
                    <input type="checkbox" checked={join.includes(c.id)} onChange={() => setJoin(toggle(join, c.id))} /># {c.name}
                  </label>
                ))}
              </div>
            </div>
          )}
        </div>
        {error && <div className="form-error">{error}</div>}
        <footer>
          <button className="ghost" onClick={onClose}>
            取消
          </button>
          <button className="primary" disabled={!description.trim() || !model.trim() || (!agent && !handle)} onClick={save}>
            {agent ? '儲存' : '建立'}
          </button>
        </footer>
      </div>
    </div>
  )
}
