import { useState } from 'react'
import { localApi, type LocalProjectCheck } from '../api'
import { Modal } from './Modals'
import type { ProjectGroup } from './CodingSection'

const message = (e: unknown) => (e instanceof Error ? e.message : String(e))

/** "New project": a folder becomes a project, and gets its first conversation. */
export function NewProjectModal({ onClose, onCreated }: { onClose: () => void; onCreated: (conversationId: string) => void }) {
  const [path, setPath] = useState('')
  const [name, setName] = useState('')
  const [conversation, setConversation] = useState('')
  // What the server said about the folder that is in the field right now.
  const [checked, setChecked] = useState<LocalProjectCheck | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const edit = (v: string) => {
    setPath(v)
    setChecked(null)
    setError(null)
  }

  const submit = async () => {
    setBusy(true)
    setError(null)
    try {
      // First press: look at the folder. Warnings are shown and need a second press.
      if (!checked) {
        const c = await localApi.checkProject(path)
        setChecked(c)
        // Also when the folder is a project already: say so before adding a conversation to it.
        if (c.warnings.length > 0 || c.conversations > 0) return
      }
      const r = await localApi.createProject({ path, name: name.trim() || undefined, conversation_name: conversation.trim() || undefined })
      onCreated(r.conversation.id)
    } catch (e) {
      setError(message(e))
    } finally {
      setBusy(false)
    }
  }

  const joining = checked && checked.conversations > 0
  return (
    <Modal title="新增專案" onClose={onClose}>
      <p className="muted small">
        專案就是這台電腦上的一個資料夾。建立後會有一個私有對話，所有已加入的 Coding Agent 都在裡面，並在這個資料夾裡工作。同一個專案可以再開更多對話，各自有獨立的工作階段。
      </p>
      <label>
        資料夾的完整路徑
        <input className="mono" value={path} onChange={(e) => edit(e.target.value)} placeholder="例如 C:\Users\me\projects\shop" autoFocus />
      </label>
      <label>
        專案名稱（選填，預設用資料夾的名稱）
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder={checked?.project_name ?? ''} />
      </label>
      <label>
        對話名稱（選填，預設用專案名稱）
        <input value={conversation} onChange={(e) => setConversation(e.target.value)} placeholder="不能有空白、@ 或 #" />
      </label>
      {joining && (
        <p className="pill-note ok">
          這個資料夾已經是專案「{checked.project_name}」（{checked.conversations} 個對話），會在裡面再開一個對話。
        </p>
      )}
      {checked?.warnings.map((w, i) => (
        <p key={i} className="pill-note warn">
          {w}
        </p>
      ))}
      {error && <div className="form-error">{error}</div>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" disabled={busy || !path.trim()} onClick={submit}>
          {checked && checked.warnings.length > 0 ? '我知道了，仍要建立' : '建立'}
        </button>
      </footer>
    </Modal>
  )
}

/** Another conversation in a project that exists: same folder, same coding agents. */
export function NewProjectConversationModal({
  project,
  onClose,
  onCreated,
}: {
  project: ProjectGroup
  onClose: () => void
  onCreated: (conversationId: string) => void
}) {
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const create = async () => {
    setBusy(true)
    try {
      const r = await localApi.createProject({ path: project.path, conversation_name: name.trim() || undefined })
      onCreated(r.conversation.id)
    } catch (e) {
      setError(message(e))
      setBusy(false)
    }
  }
  return (
    <Modal title={`在「${project.name}」開新對話`} onClose={onClose}>
      <p className="muted small">
        新對話和這個專案的其他對話用同一個資料夾（<span className="mono">{project.path}</span>）、同樣的 Coding Agent，但有自己的工作階段
        {project.isGit ? '和自己的工作副本' : ''}，適合用來做另一件事。
      </p>
      <label>
        對話名稱（選填）
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="例如 fix-login；不填會自動取名" autoFocus />
      </label>
      {error && <div className="form-error">{error}</div>}
      <footer>
        <button className="ghost" onClick={onClose}>
          取消
        </button>
        <button className="primary" disabled={busy} onClick={create}>
          建立
        </button>
      </footer>
    </Modal>
  )
}
