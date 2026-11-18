import { useCallback, useEffect, useRef, useState } from 'react'
import { api2, type DeciderConfig, type DeciderTestResult } from '../api'

const PROTOCOLS = {
  decisions_api: { label: 'Decisions API', header: 'x-goog-api-key', prefix: '', url: 'https://generativelanguage.googleapis.com/v1beta/decisions', model: 'gemini-decision-1' },
  llm_structured: { label: 'LLM 結構化輸出', header: 'Authorization', prefix: 'Bearer ', url: 'https://api.openai.com/v1/chat/completions', model: 'gpt-6-luna' },
} as const

type Protocol = keyof typeof PROTOCOLS

interface Props {
  onError: (m: string) => void
  /** Inside the first-run setup screen: no page chrome, the caller explains what this is. */
  embedded?: boolean
  /** Called after the list was (re)loaded, so the caller can check whether a usable one exists. */
  onChanged?: () => void
}

export default function DecidersPage({ onError, embedded, onChanged }: Props) {
  const [configs, setConfigs] = useState<DeciderConfig[]>([])
  const [editing, setEditing] = useState<DeciderConfig | 'new' | null>(null)
  const [tests, setTests] = useState<Record<string, DeciderTestResult | 'running'>>({})
  // Kept in a ref: a caller passing a new function each render must not restart the loading.
  const changed = useRef(onChanged)
  changed.current = onChanged

  const reload = useCallback(() => {
    api2
      .deciders()
      .then((list) => {
        setConfigs(list)
        changed.current?.()
      })
      .catch((e) => onError(String(e)))
  }, [onError])
  useEffect(reload, [reload])

  const move = async (i: number, d: number) => {
    const ids = configs.map((c) => c.id)
    const j = i + d
    if (j < 0 || j >= ids.length) return
    ;[ids[i], ids[j]] = [ids[j], ids[i]]
    try {
      await api2.reorderDeciders(ids)
      reload()
    } catch (e) {
      onError(String(e))
    }
  }

  const test = async (c: DeciderConfig) => {
    setTests((t) => ({ ...t, [c.id]: 'running' }))
    try {
      const r = await api2.testDecider(c.id)
      setTests((t) => ({ ...t, [c.id]: r }))
      reload()
    } catch (e) {
      onError(String(e))
      setTests((t) => {
        const n = { ...t }
        delete n[c.id]
        return n
      })
    }
  }

  return (
    <div className={embedded ? 'deciders-embedded' : 'page'}>
      <header className="page-head">
        <div>
          {!embedded && <h1>決策模型設定</h1>}
          <p className="muted">
            {embedded
              ? '依順序嘗試，前一個逾時或失敗就換下一個。按「編輯」填入 API 金鑰，或按「新增設定」加入別的模型；儲存後可以按「測試」確認連得上。'
              : '沒有被 tag 的人類訊息由決策模型判斷要喚醒誰。依順序嘗試，前一個逾時或失敗就換下一個；全部失敗時不喚醒任何人，訊息照常送出。變更立即生效。'}
          </p>
        </div>
        <button className={embedded ? 'ghost' : 'primary'} onClick={() => setEditing('new')}>
          ＋ 新增設定
        </button>
      </header>
      <div className="decider-list">
        {configs.map((c, i) => {
          const t = tests[c.id]
          return (
            <div key={c.id} className={`decider-card ${c.enabled ? '' : 'disabled'}`}>
              <div className="decider-order">
                <button className="ghost small" disabled={i === 0} onClick={() => move(i, -1)} title="上移">
                  ▲
                </button>
                <span className="order-num">{i + 1}</span>
                <button className="ghost small" disabled={i === configs.length - 1} onClick={() => move(i, 1)} title="下移">
                  ▼
                </button>
              </div>
              <div className="decider-main">
                <div className="decider-title">
                  <strong>{c.name}</strong>
                  <span className="tag">{PROTOCOLS[c.protocol].label}</span>
                  {!c.enabled && <span className="pill warn">停用</span>}
                  {!c.has_api_key && <span className="pill warn">未設定金鑰</span>}
                </div>
                <div className="muted small mono">{c.endpoint_url}</div>
                <div className="decider-meta">
                  <span>模型 <code>{c.model}</code></span>
                  <span>金鑰 {c.has_api_key ? <code>••••{c.api_key_last4}</code> : '—'}</span>
                  <span>
                    T_WAKE <code>{c.t_wake.toFixed(2)}</code> · T_DISCUSS <code>{c.t_discuss.toFixed(2)}</code> · MAX_WAKE <code>{c.max_wake}</code>
                  </span>
                  <span>逾時 {c.timeout_ms} ms</span>
                </div>
                {c.stats && c.stats.calls > 0 && (
                  <div className="decider-meta muted small">
                    最近 {c.stats.calls} 次呼叫 · 失敗率 {(c.stats.failure_rate * 100).toFixed(1)}% · 平均 {Math.round(c.stats.avg_latency_ms)} ms · p95{' '}
                    {Math.round(c.stats.p95_latency_ms)} ms
                  </div>
                )}
                {t && t !== 'running' && <TestResult r={t} />}
              </div>
              <div className="decider-actions">
                <button className="ghost" disabled={t === 'running'} onClick={() => test(c)}>
                  {t === 'running' ? '測試中…' : '測試'}
                </button>
                <button className="ghost" onClick={() => setEditing(c)}>
                  編輯
                </button>
                <button
                  className="ghost"
                  onClick={() => api2.updateDecider(c.id, { enabled: !c.enabled }).then(reload).catch((e) => onError(String(e)))}
                >
                  {c.enabled ? '停用' : '啟用'}
                </button>
                <button
                  className="ghost danger-text"
                  onClick={() => {
                    if (confirm(`刪除「${c.name}」？`)) api2.deleteDecider(c.id).then(reload).catch((e) => onError(String(e)))
                  }}
                >
                  刪除
                </button>
              </div>
            </div>
          )
        })}
        {configs.length === 0 && (
          <div className="empty-hint">
            {embedded ? '還沒有任何設定。按「新增設定」加入一個決策模型。' : '沒有任何設定：決策模型停用，只靠規則（tag、私訊、thread）喚醒。'}
          </div>
        )}
      </div>
      {editing && (
        <DeciderForm
          config={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null)
            reload()
          }}
        />
      )}
    </div>
  )
}

function TestResult({ r }: { r: DeciderTestResult }) {
  if (!r.ok) {
    return (
      <div className="test-result bad">
        ✕ 失敗（{r.latency_ms} ms，{r.error_kind}）：{r.error}
      </div>
    )
  }
  return (
    <div className="test-result good">
      <div>
        ✓ 成功 · {r.latency_ms} ms · 範例：「{r.message}」
      </div>
      <div className="score-bars">
        {r.scores.map(([h, s]) => (
          <div key={h} className="score-row">
            <span className="score-name">@{h}</span>
            <span className="score-bar">
              <i style={{ width: `${Math.round(s * 100)}%` }} />
            </span>
            <span className="score-val">{s.toFixed(2)}</span>
          </div>
        ))}
      </div>
      <div className="muted small">
        互動判斷 {r.interaction}（{r.interaction_confidence?.toFixed(2)}）→ 喚醒 {r.woken.length ? r.woken.map((w) => '@' + w).join('、') : '無'}，模式 {r.mode ?? 'none'}
      </div>
    </div>
  )
}

function DeciderForm({ config, onClose, onSaved }: { config: DeciderConfig | null; onClose: () => void; onSaved: () => void }) {
  const [protocol, setProtocol] = useState<Protocol>(config?.protocol ?? 'decisions_api')
  const [name, setName] = useState(config?.name ?? '')
  const [url, setUrl] = useState(config?.endpoint_url ?? PROTOCOLS.decisions_api.url)
  const [apiKey, setApiKey] = useState('')
  const [clearKey, setClearKey] = useState(false)
  const [header, setHeader] = useState(config?.auth_header ?? PROTOCOLS.decisions_api.header)
  const [prefix, setPrefix] = useState(config?.auth_prefix ?? PROTOCOLS.decisions_api.prefix)
  const [model, setModel] = useState(config?.model ?? PROTOCOLS.decisions_api.model)
  const [inputField, setInputField] = useState(config?.input_field ?? 'input')
  const [timeout, setTimeoutMs] = useState(String(config?.timeout_ms ?? 1500))
  const [tWake, setTWake] = useState(String(config?.t_wake ?? 0.6))
  const [tDiscuss, setTDiscuss] = useState(String(config?.t_discuss ?? 0.75))
  const [maxWake, setMaxWake] = useState(String(config?.max_wake ?? 4))
  const [extraHeaders, setExtraHeaders] = useState(JSON.stringify(config?.extra_headers ?? {}, null, 2))
  const [extraBody, setExtraBody] = useState(JSON.stringify(config?.extra_body ?? {}, null, 2))
  const [error, setError] = useState<string | null>(null)

  const switchProtocol = (p: Protocol) => {
    setProtocol(p)
    if (!config) {
      setUrl(PROTOCOLS[p].url)
      setHeader(PROTOCOLS[p].header)
      setPrefix(PROTOCOLS[p].prefix)
      setModel(PROTOCOLS[p].model)
      setTimeoutMs(p === 'decisions_api' ? '1500' : '8000')
      setExtraBody(p === 'llm_structured' ? '{\n  "reasoning_effort": "none"\n}' : '{}')
    }
  }

  const save = async () => {
    let eh: unknown, eb: unknown
    try {
      eh = JSON.parse(extraHeaders || '{}')
      eb = JSON.parse(extraBody || '{}')
    } catch {
      setError('額外標頭／請求欄位必須是 JSON 物件')
      return
    }
    const input: Record<string, unknown> = {
      name,
      protocol,
      endpoint_url: url,
      auth_header: header,
      auth_prefix: prefix,
      model,
      input_field: inputField,
      timeout_ms: Number(timeout),
      t_wake: Number(tWake),
      t_discuss: Number(tDiscuss),
      max_wake: Number(maxWake),
      extra_headers: eh,
      extra_body: eb,
    }
    if (apiKey.trim()) input.api_key = apiKey.trim()
    if (clearKey) input.clear_api_key = true
    try {
      if (config) await api2.updateDecider(config.id, input)
      else await api2.createDecider(input)
      onSaved()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <div className="modal-backdrop" onMouseDown={onClose}>
      <div className="modal wide" onMouseDown={(e) => e.stopPropagation()}>
        <header>
          <h2>{config ? `編輯「${config.name}」` : '新增決策模型設定'}</h2>
          <button className="ghost" onClick={onClose}>
            ✕
          </button>
        </header>
        <div className="form-grid">
          <label>
            名稱
            <input value={name} onChange={(e) => setName(e.target.value)} autoFocus />
          </label>
          <label>
            協定
            <select value={protocol} onChange={(e) => switchProtocol(e.target.value as Protocol)}>
              <option value="decisions_api">decisions_api（Gemini Decisions API 相容）</option>
              <option value="llm_structured">llm_structured（OpenAI 相容 chat completions）</option>
            </select>
          </label>
          <label className="span2">
            端點網址（只允許 https）
            <input value={url} onChange={(e) => setUrl(e.target.value)} className="mono" />
          </label>
          <label>
            API 金鑰（只能寫入）
            <input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={config?.has_api_key ? `已設定 ••••${config.api_key_last4}（留空不變）` : '尚未設定'}
              autoComplete="off"
            />
          </label>
          <label>
            模型 ID
            <input value={model} onChange={(e) => setModel(e.target.value)} className="mono" />
          </label>
          <label>
            金鑰標頭
            <input value={header} onChange={(e) => setHeader(e.target.value)} className="mono" />
          </label>
          <label>
            金鑰前綴
            <input value={prefix} onChange={(e) => setPrefix(e.target.value)} className="mono" placeholder="例如 Bearer（含空格）" />
          </label>
          <label>
            T_WAKE（≤ 0.99）
            <input type="number" step="0.05" min={0} max={0.99} value={tWake} onChange={(e) => setTWake(e.target.value)} />
          </label>
          <label>
            T_DISCUSS
            <input type="number" step="0.05" min={0} max={1} value={tDiscuss} onChange={(e) => setTDiscuss(e.target.value)} />
          </label>
          <label>
            MAX_WAKE
            <input type="number" min={1} max={16} value={maxWake} onChange={(e) => setMaxWake(e.target.value)} />
          </label>
          <label>
            逾時（ms）
            <input type="number" min={100} max={60000} value={timeout} onChange={(e) => setTimeoutMs(e.target.value)} />
          </label>
          {protocol === 'decisions_api' && (
            <label>
              共用語境欄位名（input_field）
              <input value={inputField} onChange={(e) => setInputField(e.target.value)} className="mono" />
            </label>
          )}
          <label className="span2">
            額外標頭（JSON）
            <textarea rows={2} value={extraHeaders} onChange={(e) => setExtraHeaders(e.target.value)} className="mono" />
          </label>
          <label className="span2">
            額外請求欄位（JSON）
            <textarea rows={3} value={extraBody} onChange={(e) => setExtraBody(e.target.value)} className="mono" />
          </label>
          {config?.has_api_key && (
            <label className="row span2">
              <input type="checkbox" checked={clearKey} onChange={(e) => setClearKey(e.target.checked)} /> 清除已儲存的金鑰
            </label>
          )}
        </div>
        <p className="muted small">門檻建議用 <code>ac-cli decider eval</code> 以實際中文樣本校準。</p>
        {error && <div className="form-error">{error}</div>}
        <footer>
          <button className="ghost" onClick={onClose}>
            取消
          </button>
          <button className="primary" disabled={!name.trim() || !url.trim() || !model.trim()} onClick={save}>
            儲存
          </button>
        </footer>
      </div>
    </div>
  )
}
