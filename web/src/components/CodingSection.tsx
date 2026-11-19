import { useState, type ReactNode } from 'react'
import type { ConversationItem, LocalProject } from '../api'

/** One project: a folder, and the conversations bound to it. */
export interface ProjectGroup {
  path: string
  name: string
  exists: boolean
  isGit: boolean
  catchAll: boolean
  conversations: LocalProject[]
}

/**
 * Projects are not stored anywhere as such: conversations with the same folder are one
 * project. Conversations of coding agents without a folder come back separately.
 */
export function groupProjects(rows: LocalProject[]): { projects: ProjectGroup[]; unassigned: LocalProject[] } {
  const byPath = new Map<string, ProjectGroup>()
  const unassigned: LocalProject[] = []
  for (const r of rows) {
    if (!r.path) {
      unassigned.push(r)
      continue
    }
    let g = byPath.get(r.path)
    if (!g) {
      g = { path: r.path, name: r.project_name ?? r.path, exists: r.exists, isGit: r.is_git, catchAll: r.catch_all, conversations: [] }
      byPath.set(r.path, g)
    }
    g.conversations.push(r)
  }
  const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name)
  const projects = [...byPath.values()].sort(byName)
  for (const g of projects) g.conversations.sort(byName)
  return { projects, unassigned: unassigned.sort(byName) }
}

function remembered(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key)
    return v === null ? fallback : v === '1'
  } catch {
    return fallback
  }
}

function remember(key: string, open: boolean) {
  try {
    localStorage.setItem(key, open ? '1' : '0')
  } catch {
    /* storage unavailable: the section just opens again next time */
  }
}

/** A collapsible block whose open/closed state is kept for this viewer. */
function Fold({
  storeKey,
  head,
  title,
  level,
  onAdd,
  addTitle,
  children,
}: {
  storeKey: string
  head: ReactNode
  title?: string
  level: 'section' | 'project'
  onAdd?: () => void
  addTitle?: string
  children: ReactNode
}) {
  const [open, setOpen] = useState(() => remembered(storeKey, true))
  const toggle = () => {
    remember(storeKey, !open)
    setOpen(!open)
  }
  return (
    <div className={level === 'section' ? 'sb-section' : 'sb-project'}>
      <div className="sb-section-head">
        <button className="sb-caret" onClick={toggle} title={title} aria-expanded={open}>
          <span className={`caret ${open ? 'open' : ''}`}>▸</span> {head}
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

interface Props {
  rows: LocalProject[]
  convs: ConversationItem[]
  activeId: string | null
  onOpen: (id: string) => void
  onNewProject: () => void
  onNewConversation: (project: ProjectGroup) => void
  onRename: (row: LocalProject) => void
  onArchive: (row: LocalProject) => void
}

/** The "Coding Agent" part of the sidebar: projects, and under each its conversations. */
export default function CodingSection(p: Props) {
  const { projects, unassigned } = groupProjects(p.rows)
  const conv = (id: string) => p.convs.find((c) => c.id === id)
  const item = (r: LocalProject) => {
    const c = conv(r.conversation_id)
    const unread = !!c && c.last_other_seq > c.last_read_seq && c.id !== p.activeId
    return (
      <div key={r.conversation_id} className={`sb-item sb-child ${r.conversation_id === p.activeId ? 'active' : ''} ${unread ? 'unread' : ''}`}>
        <button className="sb-child-open" onClick={() => p.onOpen(r.conversation_id)} title={r.agents.map((a) => '@' + a).join('、')}>
          <span className="sb-glyph">{r.is_channel ? '#' : '@'}</span>
          <span className="sb-label">{r.name.replace(/^#/, '')}</span>
          {c?.frozen_until_human && <span className="sb-badge warn">已停止</span>}
        </button>
        {r.is_channel && (
          <span className="sb-child-actions">
            <button title="重新命名" onClick={() => p.onRename(r)}>
              ✎
            </button>
            <button title="封存（從側邊欄移除，不會刪除內容）" onClick={() => p.onArchive(r)}>
              ✕
            </button>
          </span>
        )}
      </div>
    )
  }
  return (
    <Fold storeKey="cc.sb.coding" level="section" head="Coding Agent" onAdd={p.onNewProject} addTitle="新增專案">
      {projects.map((g) => (
        <Fold
          key={g.path}
          storeKey={`cc.sb.project.${g.path}`}
          level="project"
          title={g.path}
          head={
            <>
              <span className="sb-project-name">{g.name}</span>
              {!g.exists && <span className="sb-badge warn">找不到資料夾</span>}
            </>
          }
          onAdd={() => p.onNewConversation(g)}
          addTitle={`在「${g.name}」開新對話`}
        >
          {g.conversations.map(item)}
        </Fold>
      ))}
      {unassigned.length > 0 && (
        <Fold storeKey="cc.sb.project." level="project" head={<span className="sb-project-name unassigned">未指定專案</span>} title="還沒有綁定資料夾的對話：Coding Agent 在這裡只能聊天">
          {unassigned.map(item)}
        </Fold>
      )}
      <button className="sb-item muted" onClick={p.onNewProject}>
        <span className="sb-glyph add">＋</span>
        <span className="sb-label">新增專案</span>
      </button>
    </Fold>
  )
}
