// Typed client for the CrewConvo human API.

export type MemberKind = 'human' | 'agent' | 'system'
export type AgentStatus = 'online' | 'offline' | 'busy' | 'disabled'
export type WakeStatus = 'pending' | 'delivered' | 'running' | 'done' | 'timeout' | 'failed' | 'cancelled'

export interface Member {
  id: string
  workspace_id: string
  kind: MemberKind
  handle: string
  display_name: string
  is_admin: boolean
  deactivated_at?: string | null
  created_at: string
}

export interface Agent {
  member: Member
  origin: 'builtin' | 'external'
  owner_id: string
  description: string
  expertise: string[]
  status: AgentStatus
  model: string | null
  system_prompt: string | null
  tools: string[]
  max_wake_seconds: number
  daily_token_budget: number | null
  sandbox_config: Record<string, unknown>
  delivery: string | null
  webhook_url: string | null
  has_token: boolean
  token_prefix: string | null
  last_seen_at: string | null
  provider_connection_id?: string | null
  mention_only?: boolean
}

export interface ConvMember {
  member_id: string
  handle: string
  display_name: string
  kind: MemberKind
  description: string | null
  agent_status: AgentStatus | null
  last_read_seq: number
}

export interface Conversation {
  id: string
  kind: 'channel' | 'dm'
  name: string | null
  topic: string | null
  visibility: 'public' | 'private'
  created_by: string
  frozen_until_human: boolean
  next_seq: number
}

export interface ConversationItem extends Conversation {
  last_read_seq: number
  last_other_seq: number
  dm_members: ConvMember[]
}

export interface Wake {
  id: string
  agent_id: string
  conversation_id: string
  thread_root_id: string | null
  trigger_message_id: string
  reason: string
  mode: string
  status: WakeStatus
  depth: number
  posted_count: number
  tokens_used: number
  private_log: string | null
  created_at: string
  started_at: string | null
  finished_at: string | null
}

export interface ConversationDetail extends Conversation {
  members: ConvMember[]
  is_member: boolean
  active_wakes: Wake[]
}

export interface Reaction {
  emoji: string
  count: number
  handles: string[]
}

export interface MessageView {
  id: string
  conversation_id: string
  seq: number
  thread_root_id: string | null
  author_id: string
  author_handle: string
  author_display_name: string
  author_kind: MemberKind
  body: string
  mentions: string[]
  reactions: Reaction[]
  reply_count: number
  depth: number
  chain_root_id: string
  wake_id: string | null
  created_at: string
}

export interface WakeWithAgent extends Wake {
  agent_handle: string
}

export type RealtimeEvent =
  | { type: 'message_created'; message: MessageView }
  | { type: 'message_deleted'; conversation_id: string; message_id: string }
  | { type: 'reactions_changed'; conversation_id: string; message_id: string; reactions: Reaction[] }
  | {
      type: 'wake_status'
      conversation_id: string
      wake_id: string
      agent_id: string
      agent_handle: string
      thread_root_id: string | null
      reason: string
      mode: string
      status: WakeStatus
    }
  | { type: 'conversation_updated'; conversation: Conversation }
  | { type: 'membership_changed'; conversation_id: string; member_id: string; joined: boolean }
  | { type: 'agents_changed'; workspace_id: string }
  | { type: 'resync' }

export class ApiError extends Error {
  status: number
  code: string
  constructor(status: number, code: string, message: string) {
    super(message)
    this.status = status
    this.code = code
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    credentials: 'same-origin',
    headers: body !== undefined ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })
  const text = await res.text()
  const data = text ? JSON.parse(text) : null
  if (!res.ok) {
    throw new ApiError(res.status, data?.error?.code ?? 'error', data?.error?.message ?? res.statusText)
  }
  return data as T
}

export const api = {
  register: (email: string, name: string, password: string) =>
    request<Member>('POST', '/api/auth/register', { email, name, password }),
  /** `handle` is the e-mail address, or the name of an account created before e-mail sign-up. */
  login: (handle: string, password: string) => request<Member>('POST', '/api/auth/login', { handle, password }),
  logout: () => request<unknown>('POST', '/api/auth/logout'),
  me: () => request<Member>('GET', '/api/me'),
  members: () => request<Member[]>('GET', '/api/members'),
  models: () => request<{ providers: string[]; examples: string[] }>('GET', '/api/models'),
  agents: () => request<Agent[]>('GET', '/api/agents'),
  createAgent: (input: Record<string, unknown>) => request<Agent>('POST', '/api/agents', input),
  updateAgent: (id: string, input: Record<string, unknown>) => request<Agent>('PATCH', `/api/agents/${id}`, input),
  conversations: () => request<ConversationItem[]>('GET', '/api/conversations'),
  publicChannels: () => request<Conversation[]>('GET', '/api/channels/public'),
  createChannel: (name: string, topic: string, visibility: string, invite: string[]) =>
    request<Conversation>('POST', '/api/conversations', { name, topic: topic || null, visibility, invite }),
  openDm: (member_ids: string[]) => request<Conversation>('POST', '/api/dms', { member_ids }),
  conversation: (id: string) => request<ConversationDetail>('GET', `/api/conversations/${id}`),
  join: (id: string) => request<unknown>('POST', `/api/conversations/${id}/join`),
  leave: (id: string) => request<unknown>('POST', `/api/conversations/${id}/leave`),
  invite: (id: string, member_ids: string[]) =>
    request<{ added: string[] }>('POST', `/api/conversations/${id}/invite`, { member_ids }),
  stop: (id: string) => request<{ cancelled: number }>('POST', `/api/conversations/${id}/stop`),
  markRead: (id: string, seq: number) => request<unknown>('POST', `/api/conversations/${id}/read`, { seq }),
  messages: (id: string, opts: { thread_root_id?: string; before_seq?: number; limit?: number } = {}) => {
    const q = new URLSearchParams()
    if (opts.thread_root_id) q.set('thread_root_id', opts.thread_root_id)
    if (opts.before_seq !== undefined) q.set('before_seq', String(opts.before_seq))
    q.set('limit', String(opts.limit ?? 50))
    return request<MessageView[]>('GET', `/api/conversations/${id}/messages?${q}`)
  },
  post: (id: string, body: string, thread_root_id?: string | null) =>
    request<{ message: MessageView; ignored_mentions: string[] }>('POST', `/api/conversations/${id}/messages`, {
      body,
      thread_root_id: thread_root_id ?? null,
    }),
  deleteMessage: (id: string) => request<unknown>('DELETE', `/api/messages/${id}`),
  react: (id: string, emoji: string, add: boolean) =>
    request<MessageView>(add ? 'POST' : 'DELETE', `/api/messages/${id}/reactions`, { emoji }),
  wakes: (id: string) => request<{ message_id: string; wakes: WakeWithAgent[] }>('GET', `/api/messages/${id}/wakes`),
}

// ---- phases 2–4 ----

export interface SearchHit {
  message: MessageView
  conversation: { id: string; kind: 'channel' | 'dm'; name: string | null }
  before: MessageView | null
  after: MessageView | null
  score: number
}

export interface SearchResults {
  total: number
  offset: number
  hits: SearchHit[]
}

export interface DeciderConfig {
  id: string
  name: string
  protocol: 'decisions_api' | 'llm_structured'
  endpoint_url: string
  has_api_key: boolean
  api_key_last4: string | null
  auth_header: string
  auth_prefix: string
  model: string
  input_field: string
  timeout_ms: number
  t_wake: number
  t_discuss: number
  max_wake: number
  extra_headers: Record<string, unknown>
  extra_body: Record<string, unknown>
  enabled: boolean
  priority: number
  updated_at: string
  stats?: { calls: number; failures: number; failure_rate: number; avg_latency_ms: number; p95_latency_ms: number }
}

export interface DeciderTestResult {
  ok: boolean
  latency_ms: number
  error: string | null
  error_kind: string | null
  message: string
  scores: [string, number][]
  interaction: string | null
  interaction_confidence: number | null
  woken: string[]
  mode: string | null
}

export interface DecisionRecord {
  id: string
  decider_config_id: string | null
  source: 'rule' | 'decider' | 'none'
  reason: string | null
  attempts: { config_id: string; name: string; ok: boolean; latency_ms: number; error?: string; error_kind?: string; skipped?: string }[]
  scores: Record<string, number> | null
  interaction: string | null
  interaction_confidence: number | null
  woken: string[]
  mode: string | null
  latency_ms: number | null
  created_at: string
}

export interface WakeExplanation {
  message_id: string
  wakes: WakeWithAgent[]
  decisions: DecisionRecord[]
  produced_by: WakeWithAgent | null
  agent_handles: Record<string, string>
}

export interface AgentCredentials {
  agent: Agent
  token: string | null
  webhook_secret: string | null
}

export interface SystemInfo {
  edition: 'personal' | 'enterprise'
  registration_open: boolean
  llm_providers: string[]
  sandbox: boolean
  summaries: boolean
  summary_model: string | null
  mcp_endpoint: string
  agent_ws_endpoint: string
  public_base_url: string
  public_base_url_configured: boolean
  /** The server can set up the coding agents on the computer it runs on. */
  local_agents?: boolean
  /** The workspace has a decision model that can be called; until then the app shows only its setup. */
  decider_ready?: boolean
  /** The release the server was built from (e.g. `v0.6.0`); null for a development build. */
  version?: string | null
}

export const api2 = {
  system: () => request<SystemInfo>('GET', '/api/system'),
  search: (query: string, opts: { conversation_id?: string; limit?: number; offset?: number } = {}) => {
    const q = new URLSearchParams({ query })
    if (opts.conversation_id) q.set('conversation_id', opts.conversation_id)
    q.set('limit', String(opts.limit ?? 20))
    if (opts.offset) q.set('offset', String(opts.offset))
    return request<SearchResults>('GET', `/api/search?${q}`)
  },
  deciders: () => request<DeciderConfig[]>('GET', '/api/decider-configs'),
  createDecider: (input: Record<string, unknown>) => request<DeciderConfig>('POST', '/api/decider-configs', input),
  updateDecider: (id: string, input: Record<string, unknown>) => request<DeciderConfig>('PATCH', `/api/decider-configs/${id}`, input),
  deleteDecider: (id: string) => request<unknown>('DELETE', `/api/decider-configs/${id}`),
  reorderDeciders: (ids: string[]) => request<DeciderConfig[]>('POST', '/api/decider-configs/reorder', { ids }),
  testDecider: (id: string) => request<DeciderTestResult>('POST', `/api/decider-configs/${id}/test`),
  explain: (messageId: string) => request<WakeExplanation>('GET', `/api/messages/${messageId}/wakes`),
  createExternalAgent: (input: Record<string, unknown>) => request<AgentCredentials>('POST', '/api/agents/external', input),
  rotateToken: (id: string) => request<AgentCredentials>('POST', `/api/agents/${id}/token`),
  revokeToken: (id: string) => request<Agent>('DELETE', `/api/agents/${id}/token`),
  updateDelivery: (id: string, input: Record<string, unknown>) => request<AgentCredentials>('PUT', `/api/agents/${id}/delivery`, input),
  updateSandbox: (id: string, cfg: Record<string, unknown>) => request<Agent>('PUT', `/api/agents/${id}/sandbox`, cfg),
  deleteAgent: (id: string) => request<unknown>('DELETE', `/api/agents/${id}?confirm=true`),
}

export interface WakeActivity extends Wake {
  agent_handle: string
  agent_display_name: string
  conversation_kind: 'channel' | 'dm'
  conversation_name: string | null
  trigger_excerpt: string | null
  trigger_author_handle: string | null
}

export const api3 = {
  activity: (before?: string) =>
    request<WakeActivity[]>('GET', `/api/activity?limit=60${before ? `&before=${encodeURIComponent(before)}` : ''}`),
  wakePayload: (wakeId: string) => request<Record<string, unknown>>('GET', `/api/wakes/${wakeId}/payload`),
}

export interface AgentGroup {
  id: string
  name: string
  description: string
  created_by: string
  created_at: string
  agent_ids: string[]
}

export const groupsApi = {
  list: () => request<AgentGroup[]>('GET', '/api/agent-groups'),
  create: (body: { name: string; description?: string; agent_ids: string[] }) => request<AgentGroup>('POST', '/api/agent-groups', body),
  update: (id: string, body: { name?: string; description?: string; agent_ids?: string[] }) =>
    request<AgentGroup>('PATCH', `/api/agent-groups/${id}`, body),
  remove: (id: string) => request<unknown>('DELETE', `/api/agent-groups/${id}`),
}


export interface AuthInfo {
  edition: 'personal' | 'enterprise'
  /** No account exists yet: the first one becomes the owner / admin. */
  needs_setup: boolean
  registration_open: boolean
}

export const authInfo = () => request<AuthInfo>('GET', '/api/auth/info')

/** A coding agent the CLI wants to connect (from `crewconvo setup`). */
export interface ConnectClientAgent {
  platform: string
  name: string
  wakeable: boolean
  suggested_handle: string
  existing_agent_id?: string | null
}

export interface ConnectRequestView {
  user_code: string
  client: {
    hostname?: string
    os?: string
    user?: string
    cli?: string
    agents?: ConnectClientAgent[]
    suggested_group?: string
    suggested_channel?: string
  }
  status: 'pending' | 'approved' | 'denied' | 'consumed' | 'expired'
  expires_at: string
}

export interface ConnectApproval {
  agents: { platform?: string; agent_id?: string; handle?: string; display_name?: string; description?: string }[]
  group_name?: string
  channel_name?: string
}

export interface ApproveResult {
  agents: Agent[]
  group: AgentGroup | null
  channel: Conversation | null
  warnings: string[]
}

export const connectApi = {
  get: (code: string) => request<ConnectRequestView>('GET', `/api/connect/${encodeURIComponent(code)}`),
  approve: (code: string, body: ConnectApproval) => request<ApproveResult>('POST', `/api/connect/${encodeURIComponent(code)}/approve`, body),
  deny: (code: string) => request<unknown>('POST', `/api/connect/${encodeURIComponent(code)}/deny`),
}

export interface LocalPlatform {
  id: string
  name: string
  description: string
  /** Found on the computer CrewConvo runs on. */
  detected: boolean
  /** Has a command-line program, so it can answer when mentioned. */
  can_wake: boolean
  /** Whether it can actually be woken; null when it is not on this computer. */
  state: LocalReadiness | null
  explanation: string
  /** For `needs_login`: what to run in a terminal. */
  login_command: string | null
  /** What it may do to files can be limited (otherwise the access level does not apply). */
  limits_access: boolean
  /** The description a new agent gets when none is written. */
  default_role: string
  /** A wake can continue an earlier session of this tool. */
  keeps_sessions: boolean
  /** The model and the thinking depth of this tool can be chosen. */
  tunable: boolean
  /** The thinking depths the tool accepts, weakest first (the tool's own words). */
  efforts: string[]
  /** The models the tool itself lists; empty when it cannot list any. */
  models: { id: string; name: string; efforts: string[]; default_effort: string | null }[]
  /** Names to suggest when there is no list. */
  model_hints: string[]
  agent: {
    agent_id: string
    handle: string
    online: boolean
    description: string
    expertise: string[]
    access: LocalAccess
    /** null: the tool's own default. */
    model: string | null
    effort: string | null
    /** What the tool last said it actually ran on. */
    used_model: string | null
    used_effort: string | null
  } | null
}

/** ready: signed in. needs_login: not signed in. no_cli: only usable from inside the app. unknown: cannot tell. */
export type LocalReadiness = 'ready' | 'needs_login' | 'no_cli' | 'unknown'

/** What a coding agent may do to the project's files when its owner asks. */
export type LocalAccess = 'read' | 'edit' | 'full'

/** What one coding agent is responsible for. */
export interface LocalRole {
  description?: string
  expertise?: string[]
  access?: LocalAccess
}

/** A conversation with local coding agents in it, and the folder they work in. */
export interface LocalProject {
  conversation_id: string
  name: string
  is_channel: boolean
  /** What the project (the folder) is called; null when the conversation has no folder. */
  project_name: string | null
  /** The folder is one of the user's catch-all folders (Downloads, Desktop…) itself. */
  catch_all: boolean
  agents: string[]
  path: string | null
  exists: boolean
  is_git: boolean
}

export interface LocalProjectCheck {
  path: string
  project_name: string
  /** How many conversations this folder already has as a project. */
  conversations: number
  warnings: string[]
}

/** What an agent did in its own working copy (a git worktree) that is not merged yet. */
export interface LocalWork {
  id: string
  agent_handle: string
  conversation_id: string
  /** False when the conversation has been bound to another folder since; the work still belongs to `project`. */
  current_folder: boolean
  conversation_name: string
  thread_root_id: string | null
  project: string
  worktree: string
  branch: string
  base_branch: string | null
  last_used_at: string
  /** Commits on its branch that the project does not have. */
  ahead: number
  diffstat: string
  /** False when the worktree or the project folder is gone. */
  exists: boolean
}

/** A session of a coding agent's command-line tool that CrewConvo started, and what it is for. */
export interface LocalSession {
  id: string
  platform: string
  agent_id: string
  agent_handle: string | null
  conversation_id: string
  conversation_name: string
  thread_root_id: string | null
  purpose: string
  created_at: string
  last_used_at: string
  uses: number
}

export interface LocalSessionsInfo {
  /** False when LOCAL_SESSIONS=new: every wake starts a new session. */
  reuse: boolean
  sessions: LocalSession[]
}

export interface LocalAgentsInfo {
  platforms: LocalPlatform[]
  server: string
  default_group: string
  default_channel: string
}

export interface LocalInstallResult {
  results: {
    platform: string
    name: string
    handle: string
    ok: boolean
    lines: string[]
    can_wake: boolean
    state: LocalReadiness
    explanation: string
    login_command: string | null
  }[]
  group: string | null
  channel: { id: string; name: string } | null
  warnings: string[]
}

export const localApi = {
  /** `refresh` checks again whether each coding agent is signed in instead of using the last answer. */
  list: (refresh = false) => request<LocalAgentsInfo>('GET', refresh ? '/api/local-agents?refresh=true' : '/api/local-agents'),
  setAccess: (platform: string, access: LocalAccess) => request<{ access: LocalAccess }>('PUT', `/api/local-agents/${platform}/access`, { access }),
  projects: () => request<{ projects: LocalProject[] }>('GET', '/api/local-agents/projects'),
  /** Looks at a folder before making it a project: how it is written, its name, warnings. Changes nothing. */
  checkProject: (path: string) => request<LocalProjectCheck>('POST', '/api/local-agents/projects/check', { path }),
  /** A new conversation in the project that is this folder (creating the project if it is new). */
  createProject: (body: { path: string; name?: string; conversation_name?: string }) =>
    request<{ conversation: { id: string; name: string }; path: string; project_name: string; warnings: string[] }>('POST', '/api/local-agents/projects', body),
  renameProject: (path: string, name: string) => request<{ path: string; project_name: string }>('PUT', '/api/local-agents/projects/name', { path, name }),
  renameConversation: (id: string, name: string) => request<{ id: string; name: string }>('PATCH', `/api/local-agents/conversations/${id}`, { name }),
  archiveConversation: (id: string) => request<unknown>('POST', `/api/local-agents/conversations/${id}/archive`),
  setProject: (conversationId: string, path: string) =>
    request<{ path: string; exists: boolean; is_git: boolean }>('PUT', `/api/local-agents/projects/${conversationId}`, { path }),
  clearProject: (conversationId: string) => request<unknown>('DELETE', `/api/local-agents/projects/${conversationId}`),
  work: () => request<{ work: LocalWork[] }>('GET', '/api/local-agents/work'),
  mergeWork: (id: string) => request<{ ok: boolean; branch: string; into: string | null }>('POST', `/api/local-agents/work/${id}/merge`),
  discardWork: (id: string) => request<unknown>('POST', `/api/local-agents/work/${id}/discard`),
  install: (body: { platforms: string[]; group_name?: string; channel_name?: string; roles?: Record<string, LocalRole> }) =>
    request<LocalInstallResult>('POST', '/api/local-agents/install', body),
  remove: (platform: string) => request<unknown>('DELETE', `/api/local-agents/${platform}`),
  /** Model and thinking depth; an empty string goes back to the tool's default. */
  setTuning: (platform: string, body: { model: string; effort: string }) =>
    request<{ model: string | null; effort: string | null }>('PUT', `/api/local-agents/${platform}/model`, body),
  setRole: (platform: string, role: LocalRole) =>
    request<{ description: string; expertise: string[] }>('PUT', `/api/local-agents/${platform}/role`, role),
  sessions: () => request<LocalSessionsInfo>('GET', '/api/local-agents/sessions'),
  forgetSession: (id: string) => request<unknown>('DELETE', `/api/local-agents/sessions/${encodeURIComponent(id)}`),
}
