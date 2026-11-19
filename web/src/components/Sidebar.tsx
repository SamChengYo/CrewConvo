import { useState } from 'react'
import type { Agent, ConversationItem, LocalProject, Member } from '../api'
import CodingSection, { type ProjectGroup } from './CodingSection'
import { Avatar } from './Avatar'

interface Props {
  me: Member
  edition: 'personal' | 'enterprise'
  convs: ConversationItem[]
  agents: Agent[]
  activeId: string | null
  page: 'agents' | 'deciders' | 'activity' | 'groups' | 'local' | null
  /** Present when the server can set up the coding agents on the computer it runs on. */
  onLocal?: () => void
  /** Conversations of coding agents with their project folders; they get their own section. */
  coding: LocalProject[]
  onNewProject: () => void
  onNewProjectConversation: (project: ProjectGroup) => void
  onRenameCoding: (row: LocalProject) => void
  onArchiveCoding: (row: LocalProject) => void
  onOpen: (id: string) => void
  onOpenAgent: (agentId: string) => void
  onAgents: () => void
  onDeciders: () => void
  onActivity: () => void
  onGroups: () => void
  onNewChannel: () => void
  onNewDm: () => void
  onBrowse: () => void
}

export function dmTitle(c: ConversationItem, me: Member) {
  const others = c.dm_members.filter((m) => m.member_id !== me.id)
  return others.map((m) => m.display_name).join('、') || '私訊'
}

function Section({ title, children, onAdd, addTitle }: { title: string; children: React.ReactNode; onAdd?: () => void; addTitle?: string }) {
  const [open, setOpen] = useState(true)
  return (
    <div className="sb-section">
      <div className="sb-section-head">
        <button className="sb-caret" onClick={() => setOpen(!open)}>
          <span className={`caret ${open ? 'open' : ''}`}>▸</span> {title}
        </button>
        {onAdd && (
          <button className="sb-icon" title={addTitle} onClick={onAdd}>
            ＋
          </button>
        )}
      </div>
      {open && children}
    </div>
  )
}

export default function Sidebar(p: Props) {
  // What is listed under "Coding Agent" is not listed again among the channels and direct messages.
  const inCoding = new Set(p.onLocal ? p.coding.map((r) => r.conversation_id) : [])
  const channels = p.convs.filter((c) => c.kind === 'channel' && !inCoding.has(c.id))
  const dms = p.convs.filter((c) => c.kind === 'dm' && !inCoding.has(c.id))
  const unread = (c: ConversationItem) => c.last_other_seq > c.last_read_seq && c.id !== p.activeId

  return (
    <aside className="sidebar">
      <div className="sb-header">
        <div className="sb-workspace">
          CrewConvo <span className={`edition-badge ${p.edition}`}>{p.edition === 'personal' ? 'Personal' : 'Enterprise'}</span>
        </div>
        <button className="sb-compose" title="新訊息" onClick={p.onNewDm}>
          ✎
        </button>
      </div>
      <nav className="sb-scroll">
        <div className="sb-group">
          <button className={`sb-item ${p.page === 'activity' ? 'active' : ''}`} onClick={p.onActivity}>
            <span className="sb-glyph">⚡</span>
            <span className="sb-label">Agent 動態</span>
          </button>
          <button className={`sb-item ${p.page === 'agents' ? 'active' : ''}`} onClick={p.onAgents}>
            <span className="sb-glyph">✦</span>
            <span className="sb-label">Agent 管理</span>
          </button>
          {p.onLocal && (
            <button className={`sb-item ${p.page === 'local' ? 'active' : ''}`} onClick={p.onLocal}>
              <span className="sb-glyph">⌘</span>
              <span className="sb-label">這台電腦的 Coding Agent</span>
            </button>
          )}
          <button className={`sb-item ${p.page === 'groups' ? 'active' : ''}`} onClick={p.onGroups}>
            <span className="sb-glyph">◎</span>
            <span className="sb-label">Agent 群組</span>
          </button>
          {p.me.is_admin && (
            <button className={`sb-item ${p.page === 'deciders' ? 'active' : ''}`} onClick={p.onDeciders}>
              <span className="sb-glyph">◇</span>
              <span className="sb-label">決策模型設定</span>
            </button>
          )}
          <button className="sb-item" onClick={p.onBrowse}>
            <span className="sb-glyph">⌕</span>
            <span className="sb-label">瀏覽頻道</span>
          </button>
        </div>

        {p.onLocal && (
          <CodingSection
            rows={p.coding}
            convs={p.convs}
            activeId={p.activeId}
            onOpen={p.onOpen}
            onNewProject={p.onNewProject}
            onNewConversation={p.onNewProjectConversation}
            onRename={p.onRenameCoding}
            onArchive={p.onArchiveCoding}
          />
        )}

        <Section title="頻道" onAdd={p.onNewChannel} addTitle="建立頻道">
          {channels.map((c) => (
            <button
              key={c.id}
              className={`sb-item ${c.id === p.activeId ? 'active' : ''} ${unread(c) ? 'unread' : ''}`}
              onClick={() => p.onOpen(c.id)}
            >
              <span className="sb-glyph">{c.visibility === 'private' ? '🔒' : '#'}</span>
              <span className="sb-label">{c.name}</span>
              {c.frozen_until_human && <span className="sb-badge warn">已停止</span>}
            </button>
          ))}
          <button className="sb-item muted" onClick={p.onNewChannel}>
            <span className="sb-glyph add">＋</span>
            <span className="sb-label">新增頻道</span>
          </button>
        </Section>

        <Section title="私訊" onAdd={p.onNewDm} addTitle="開新私訊">
          {dms.map((c) => {
            const others = c.dm_members.filter((m) => m.member_id !== p.me.id)
            const first = others[0]
            const presence = first?.kind === 'agent' ? (first.agent_status === 'online' ? 'online' : 'offline') : null
            return (
              <button
                key={c.id}
                className={`sb-item ${c.id === p.activeId ? 'active' : ''} ${unread(c) ? 'unread' : ''}`}
                onClick={() => p.onOpen(c.id)}
              >
                <span className="sb-glyph">
                  <Avatar name={first?.display_name ?? '?'} kind={first?.kind ?? 'human'} size={20} presence={presence} />
                </span>
                <span className="sb-label">{dmTitle(c, p.me)}</span>
              </button>
            )
          })}
        </Section>

        <Section title="Agent" onAdd={p.onAgents} addTitle="管理 Agent">
          {p.agents
            .filter((a) => !a.member.deactivated_at)
            .map((a) => (
              <button key={a.member.id} className="sb-item" onClick={() => p.onOpenAgent(a.member.id)} title={a.description}>
                <span className="sb-glyph">
                  <Avatar name={a.member.display_name} kind="agent" size={20} presence={a.status === 'online' ? 'online' : 'offline'} />
                </span>
                <span className="sb-label">{a.member.display_name}</span>
                {a.origin === 'external' && <span className="sb-badge">外部</span>}
              </button>
            ))}
        </Section>
      </nav>
    </aside>
  )
}
