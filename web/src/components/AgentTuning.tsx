import { useState } from 'react'
import { localApi, type LocalPlatform } from '../api'

/** The tools' own words for how hard to think, with what they mean. The raw value stays visible. */
const DEPTHS: Record<string, string> = {
  none: '不思考',
  minimal: '極低',
  low: '低',
  medium: '中',
  high: '高',
  xhigh: '很高',
  max: '最高',
  ultra: '極致',
}

export function depthLabel(value: string): string {
  return DEPTHS[value] ? `${DEPTHS[value]}（${value}）` : value
}

/**
 * The thinking depths to offer: exactly what the platform accepts. For a tool that lists its
 * models (Codex) that is the chosen model's own list; otherwise the tool's fixed choices.
 * A stored value that is not among them is kept visible, so it is never changed behind the user's back.
 */
export function depthChoices(p: LocalPlatform, model: string, current: string): string[] {
  const listed = p.models.find((m) => m.id === model.trim())
  const base = listed && listed.efforts.length > 0 ? listed.efforts : p.efforts
  return current && !base.includes(current) ? [...base, current] : base
}

interface Props {
  platform: LocalPlatform
  onSaved: () => void
  onError: (msg: string) => void
}

/** Model and thinking depth of one connected coding agent. Empty means the tool's own default. */
export default function AgentTuning({ platform: p, onSaved, onError }: Props) {
  const agent = p.agent!
  const stored = { model: agent.model ?? '', effort: agent.effort ?? '' }
  // Only what is being edited; without a draft the stored values are shown.
  const [draft, setDraft] = useState<{ model: string; effort: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const value = draft ?? stored
  const changed = value.model.trim() !== stored.model || value.effort !== stored.effort

  if (!p.tunable) {
    return <p className="muted small">模型與思考深度：{p.name} 的指令列選項還沒有經過驗證，所以這裡不提供設定；它用自己原本的設定執行。</p>
  }

  const listed = p.models.find((m) => m.id === value.model.trim())
  const choices = depthChoices(p, value.model, value.effort)
  const save = async () => {
    setBusy(true)
    try {
      await localApi.setTuning(p.id, { model: value.model.trim(), effort: value.effort })
      setDraft(null)
      onSaved()
    } catch (e) {
      onError(String((e as Error).message ?? e))
    } finally {
      setBusy(false)
    }
  }
  const listId = `models-${p.id}`
  return (
    <div className="local-role">
      <div className="local-role-row">
        <label className="connect-field">
          <span>模型</span>
          <input
            className="mono"
            list={listId}
            value={value.model}
            placeholder={`預設（由 ${p.name} 自己決定）`}
            onChange={(e) => setDraft({ ...value, model: e.target.value })}
          />
          <datalist id={listId}>
            {p.models.length > 0
              ? p.models.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))
              : p.model_hints.map((m) => <option key={m} value={m} />)}
          </datalist>
        </label>
        <label className="connect-field local-depth">
          <span>思考深度</span>
          <select value={value.effort} onChange={(e) => setDraft({ ...value, effort: e.target.value })}>
            <option value="">預設{listed?.default_effort ? `（這個模型是 ${listed.default_effort}）` : ''}</option>
            {choices.map((d) => (
              <option key={d} value={d}>
                {depthLabel(d)}
              </option>
            ))}
          </select>
        </label>
        <button className="primary small" disabled={busy || !changed} onClick={save}>
          儲存
        </button>
      </div>
      <p className="muted small">
        {p.models.length > 0
          ? `可以從 ${p.name} 列出的模型裡選，也可以自己輸入。`
          : `${p.name} 沒有辦法列出可用的模型：請自己輸入名稱${p.model_hints.length > 0 ? `（例如 ${p.model_hints.join('、')}，或完整的模型名稱）` : ''}。`}
        兩個都留空就是用 {p.name} 自己的預設。填的值由 {p.name} 判斷能不能用——不能用的話，它被叫起來時會在對話裡說明要改哪一項。儲存後從下一次執行開始生效，接續中的工作階段也會套用。
        {agent.used_model && (
          <>
            {' '}
            上一次實際執行：<span className="mono">{agent.used_model}</span>
            {agent.used_effort ? `，思考深度 ${depthLabel(agent.used_effort)}` : ''}。
          </>
        )}
      </p>
    </div>
  )
}
