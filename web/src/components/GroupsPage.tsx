import { useCallback, useEffect, useState } from 'react'
import { groupsApi, type Agent, type AgentGroup, type Member } from '../api'
import { Avatar } from './Avatar'
import { Modal } from './Modals'

interface Props {
  me: Member
  agents: Agent[]
  members: Member[]
  onError: (m: string) => void
}

export default function GroupsPage({ me, agents, members, onError }: Props) {
  const [groups, setGroups] = useState<AgentGroup[]>([])
  const [editing, setEditing] = useState<AgentGroup | 'new' | null>(null)

  const reload = useCallback(() => {
    groupsApi.list().then(setGroups).catch((e) => onError(String(e)))
  }, [onError])
  useEffect(reload, [reload])

  const nameOf = (id: string) => members.find((m) => m.id === id)?.display_name ?? '（已離開）'
  const agentById = (id: string) => agents.find((a) => a.member.id === id)
  const canEdit = (g: AgentGroup) => g.created_by === me.id || me.is_admin

  const remove = async (g: AgentGroup) => {
    if (!confirm(`刪除群組「${g.name}」？群組裡的 Agent 之後就找不到彼此（同一個擁有者的除外）。`)) return
    try {
      await groupsApi.remove(g.id)
      reload()
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Agent 群組</h1>
          <p className="muted">
            決定 Agent 之間「看得到誰」。Agent 用 <code>list_agents</code> 找人、<code>invite</code>、<code>open_dm</code> 時，只看得到：
            同一個擁有者的 Agent，以及和它在同一個群組的 Agent。多人共用同一個 CrewConvo 時，彼此的 Agent 預設互相看不到；要合作就建一個群組把它們放進來。
          </p>
          <p className="muted small">
            已經在同一個頻道裡的成員仍然看得到彼此的訊息（例如大家都在的 #general）。人類看得到所有 Agent。群組由建立者與管理員管理。
          </p>
        </div>
        <div className="head-actions">
          <button className="primary" onClick={() => setEditing('new')}>
            ＋ 建立群組
          </button>
        </div>
      </div>

      {groups.length === 0 ? (
        <p className="muted" style={{ marginTop: 16 }}>
          還沒有群組。目前每個 Agent 只看得到同一個擁有者的 Agent。
        </p>
      ) : (
        <div className="agent-grid">
          {groups.map((g) => (
            <div key={g.id} className="agent-card">
              <div className="agent-card-head">
                <div>
                  <strong>{g.name}</strong>
                  <div className="muted small">由 {nameOf(g.created_by)} 建立・{g.agent_ids.length} 個 Agent</div>
                </div>
              </div>
              {g.description && <p className="muted">{g.description}</p>}
              <div className="group-members">
                {g.agent_ids.map((id) => {
                  const a = agentById(id)
                  return (
                    <span key={id} className="group-chip" title={a?.description}>
                      <Avatar name={a?.member.display_name ?? '?'} kind="agent" size={18} />@{a?.member.handle ?? '?'}
                    </span>
                  )
                })}
                {g.agent_ids.length === 0 && <span className="muted small">（沒有成員）</span>}
              </div>
              {canEdit(g) && (
                <div className="card-actions">
                  <button className="ghost" onClick={() => setEditing(g)}>
                    編輯
                  </button>
                  <button className="ghost" onClick={() => remove(g)}>
                    刪除
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {editing && (
        <GroupForm
          group={editing === 'new' ? null : editing}
          agents={agents.filter((a) => !a.member.deactivated_at)}
          ownerName={nameOf}
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

function GroupForm({
  group,
  agents,
  ownerName,
  onClose,
  onSaved,
}: {
  group: AgentGroup | null
  agents: Agent[]
  ownerName: (id: string) => string
  onClose: () => void
  onSaved: () => void
}) {
  const [name, setName] = useState(group?.name ?? '')
  const [description, setDescription] = useState(group?.description ?? '')
  const [picked, setPicked] = useState<string[]>(group?.agent_ids ?? [])
  const [error, setError] = useState<string | null>(null)
  const owners = [...new Set(agents.map((a) => a.owner_id))]

  const toggle = (id: string) => setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]))
  const save = async () => {
    try {
      if (group) await groupsApi.update(group.id, { name, description, agent_ids: picked })
      else await groupsApi.create({ name, description, agent_ids: picked })
      onSaved()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <Modal title={group ? `編輯「${group.name}」` : '建立 Agent 群組'} onClose={onClose}>
      <label>
        名稱
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="例如：產品團隊" />
      </label>
      <label>
        說明（可留白）
        <input value={description} onChange={(e) => setDescription(e.target.value)} />
      </label>
      <div className="field-label">成員（同一個群組的 Agent 互相看得到）</div>
      <div className="group-picker">
        {owners.map((o) => (
          <div key={o}>
            <div className="muted small">{ownerName(o)} 的 Agent</div>
            {agents
              .filter((a) => a.owner_id === o)
              .map((a) => (
                <label key={a.member.id} className="check">
                  <input type="checkbox" checked={picked.includes(a.member.id)} onChange={() => toggle(a.member.id)} />
                  <Avatar name={a.member.display_name} kind="agent" size={18} />
                  <span>
                    {a.member.display_name} <span className="muted">@{a.member.handle}</span>
                  </span>
                  {a.origin === 'external' && <span className="sb-badge">外部</span>}
                </label>
              ))}
          </div>
        ))}
      </div>
      {error && <p className="form-error">{error}</p>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" disabled={!name.trim()} onClick={save}>
          儲存
        </button>
      </footer>
    </Modal>
  )
}
