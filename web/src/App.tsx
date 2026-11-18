import { useCallback, useEffect, useState } from 'react'
import { api, api2, ApiError, localApi, type Agent, type ConversationItem, type Member } from './api'
import { realtime, useRealtime } from './realtime'
import Login from './components/Login'
import Sidebar from './components/Sidebar'
import TopBar from './components/TopBar'
import ConversationView from './components/ConversationView'
import AgentsPage from './components/AgentsPage'
import DecidersPage from './components/DecidersPage'
import SearchPanel from './components/SearchPanel'
import ActivityPage from './components/ActivityPage'
import ConnectPage from './components/ConnectPage'
import GroupsPage from './components/GroupsPage'
import LocalAgentsPage from './components/LocalAgentsPage'
import { NewChannelModal, NewDmModal, BrowseChannelsModal } from './components/Modals'

type View = { kind: 'conversation'; id: string } | { kind: 'agents' } | { kind: 'deciders' } | { kind: 'activity' } | { kind: 'groups' } | { kind: 'local' } | { kind: 'connect'; code: string } | { kind: 'empty' }
type ModalKind = 'channel' | 'dm' | 'browse' | null

export default function App() {
  const [me, setMe] = useState<Member | null | undefined>(undefined)
  const loggedOut = useCallback(() => setMe(null), [])

  useEffect(() => {
    api.me().then(setMe).catch(() => setMe(null))
  }, [])

  if (me === undefined) return <div className="splash">載入中…</div>
  if (me === null) return <Login onLoggedIn={setMe} />
  return <Workspace me={me} onLogout={loggedOut} />
}

function storageGet(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function storageSet(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    /* storage unavailable */
  }
}

function Workspace({ me, onLogout }: { me: Member; onLogout: () => void }) {
  const [convs, setConvs] = useState<ConversationItem[]>([])
  const [members, setMembers] = useState<Member[]>([])
  const [agents, setAgents] = useState<Agent[]>([])
  const [view, setView] = useState<View>(() =>
    location.pathname === '/connect'
      ? { kind: 'connect', code: new URLSearchParams(location.search).get('code') ?? '' }
      : { kind: 'empty' },
  )
  const [modal, setModal] = useState<ModalKind>(null)
  const [error, setError] = useState<string | null>(null)
  const [search, setSearch] = useState<string | null>(null)
  const [edition, setEdition] = useState<'personal' | 'enterprise'>('enterprise')
  const [localAgents, setLocalAgents] = useState(false)

  useEffect(() => {
    api2
      .system()
      .then((s) => {
        setEdition(s.edition)
        setLocalAgents(!!s.local_agents)
        // Nothing connected yet: start on the page that sets the coding agents up.
        if (s.local_agents)
          localApi
            .list()
            .then((i) => {
              if (i.platforms.some((p) => p.detected) && !i.platforms.some((p) => p.agent)) setView((v) => (v.kind === 'connect' ? v : { kind: 'local' }))
            })
            .catch(() => {})
      })
      .catch(() => {})
  }, [])

  const reload = useCallback(async () => {
    try {
      const [c, m, a] = await Promise.all([api.conversations(), api.members(), api.agents()])
      setConvs(c)
      setMembers(m)
      setAgents(a)
      return c
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) onLogout()
      return []
    }
  }, [onLogout])

  useEffect(() => {
    realtime.start()
    reload().then((c) => {
      const saved = storageGet('cc.lastConversation')
      const pick = c.find((x) => x.id === saved) ?? c.find((x) => x.name === 'general') ?? c[0]
      if (pick) setView((v) => (v.kind === 'connect' ? v : { kind: 'conversation', id: pick.id }))
    })
    return () => realtime.stop()
  }, [reload])

  useEffect(() => {
    if (view.kind === 'conversation') storageSet('cc.lastConversation', view.id)
  }, [view])

  useEffect(() => {
    if (!error) return
    const t = setTimeout(() => setError(null), 6000)
    return () => clearTimeout(t)
  }, [error])

  useRealtime((ev) => {
    switch (ev.type) {
      case 'membership_changed':
      case 'agents_changed':
      case 'resync':
        reload()
        break
      case 'conversation_updated':
        setConvs((cs) => cs.map((c) => (c.id === ev.conversation.id ? { ...c, ...ev.conversation } : c)))
        break
      case 'message_created':
        if (ev.message.thread_root_id === null && ev.message.author_id !== me.id) {
          setConvs((cs) =>
            cs.map((c) =>
              c.id === ev.message.conversation_id ? { ...c, last_other_seq: Math.max(c.last_other_seq, ev.message.seq) } : c,
            ),
          )
        }
        break
    }
  })

  const markRead = useCallback((id: string, seq: number) => {
    setConvs((cs) => cs.map((c) => (c.id === id && c.last_read_seq < seq ? { ...c, last_read_seq: seq } : c)))
  }, [])

  const openConversation = useCallback(
    async (id: string) => {
      if (!convs.some((c) => c.id === id)) await reload()
      setView({ kind: 'conversation', id })
    },
    [convs, reload],
  )

  const openAgent = useCallback(
    async (agentId: string) => {
      try {
        const c = await api.openDm([agentId])
        await reload()
        setView({ kind: 'conversation', id: c.id })
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e))
      }
    },
    [reload],
  )

  const logout = async () => {
    await api.logout().catch(() => {})
    onLogout()
  }

  return (
    <div className="app">
      <TopBar me={me} onSearch={setSearch} onLogout={logout} />
      <div className="app-body">
        <Sidebar
          me={me}
          edition={edition}
          convs={convs}
          agents={agents}
          activeId={view.kind === 'conversation' ? view.id : null}
          page={view.kind === 'agents' || view.kind === 'deciders' || view.kind === 'activity' || view.kind === 'groups' || view.kind === 'local' ? view.kind : null}
          onOpen={(id) => setView({ kind: 'conversation', id })}
          onOpenAgent={openAgent}
          onAgents={() => setView({ kind: 'agents' })}
          onDeciders={() => setView({ kind: 'deciders' })}
          onActivity={() => setView({ kind: 'activity' })}
          onGroups={() => setView({ kind: 'groups' })}
          onLocal={localAgents ? () => setView({ kind: 'local' }) : undefined}
          onNewChannel={() => setModal('channel')}
          onNewDm={() => setModal('dm')}
          onBrowse={() => setModal('browse')}
        />
        <main className="main">
          {error && (
            <div className="toast" onClick={() => setError(null)}>
              {error}
            </div>
          )}
          {view.kind === 'conversation' && (
            <ConversationView
              key={view.id}
              me={me}
              conversationId={view.id}
              members={members}
              agents={agents}
              onRead={markRead}
              onError={setError}
              onOpenAgent={openAgent}
              onLeft={() => {
                reload()
                setView({ kind: 'empty' })
              }}
            />
          )}
          {view.kind === 'agents' && <AgentsPage me={me} convs={convs} onError={setError} onOpenDm={openConversation} />}
          {view.kind === 'deciders' && <DecidersPage onError={setError} />}
          {view.kind === 'activity' && <ActivityPage onOpen={openConversation} onError={setError} />}
          {view.kind === 'groups' && <GroupsPage me={me} agents={agents} members={members} onError={setError} />}
          {view.kind === 'local' && <LocalAgentsPage onOpen={openConversation} onError={setError} />}
          {view.kind === 'connect' && (
            <ConnectPage
              me={me}
              initialCode={view.code}
              onDone={async (channelId) => {
                history.replaceState(null, '', '/')
                const list = await reload()
                const g = list.find((c) => c.id === channelId) ?? list.find((c) => c.name === 'general') ?? list[0]
                setView(g ? { kind: 'conversation', id: g.id } : { kind: 'empty' })
              }}
            />
          )}
          {view.kind === 'empty' && <div className="empty-state">從左邊選一個頻道或私訊開始對話。</div>}
        </main>
        {search !== null && (
          <SearchPanel
            key={search}
            initialQuery={search}
            onClose={() => setSearch(null)}
            onOpen={(id) => {
              openConversation(id)
            }}
          />
        )}
      </div>
      {modal === 'channel' && (
        <NewChannelModal
          members={members.filter((m) => m.id !== me.id)}
          onClose={() => setModal(null)}
          onCreated={async (c) => {
            setModal(null)
            await reload()
            setView({ kind: 'conversation', id: c.id })
          }}
        />
      )}
      {modal === 'dm' && (
        <NewDmModal
          members={members.filter((m) => m.id !== me.id)}
          onClose={() => setModal(null)}
          onOpened={async (c) => {
            setModal(null)
            await reload()
            setView({ kind: 'conversation', id: c.id })
          }}
        />
      )}
      {modal === 'browse' && (
        <BrowseChannelsModal
          joinedIds={new Set(convs.map((c) => c.id))}
          onClose={() => setModal(null)}
          onJoined={async (id) => {
            setModal(null)
            await reload()
            setView({ kind: 'conversation', id })
          }}
        />
      )}
    </div>
  )
}
