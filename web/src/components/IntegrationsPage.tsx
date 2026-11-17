import { useCallback, useEffect, useState } from 'react'
import { api, api2, CLI_SOURCE, type Agent, type SystemInfo } from '../api'
import { useRealtime } from '../realtime'
import ProvidersSection from './ProvidersSection'

export function CopyBlock({ text, label }: { text: string; label?: string }) {
  const [done, setDone] = useState(false)
  return (
    <div className="copy-block">
      {label && <div className="copy-label">{label}</div>}
      <div className="copy-row">
        <pre>{text}</pre>
        <button
          className="ghost small"
          onClick={() => {
            navigator.clipboard?.writeText(text).then(() => {
              setDone(true)
              setTimeout(() => setDone(false), 1500)
            })
          }}
        >
          {done ? '已複製' : '複製'}
        </button>
      </div>
    </div>
  )
}

function isLocal(url: string) {
  try {
    const h = new URL(url).hostname
    return h === 'localhost' || h === '127.0.0.1' || h === '[::1]' || h.endsWith('.localhost')
  } catch {
    return true
  }
}

type Os = 'macos' | 'linux' | 'windows'

function detectOs(): Os {
  const ua = navigator.userAgent
  if (/Windows/i.test(ua)) return 'windows'
  if (/Mac OS X|Macintosh/i.test(ua)) return 'macos'
  return 'linux'
}

interface Step {
  title: string
  body?: React.ReactNode
  cmds?: { label?: string; text: string }[]
}

function steps(os: Os, base: string): Step[] {
  const install = `pipx install "${CLI_SOURCE}"`
  const connect = `crewconvo connect ${base}`
  const verify: Step = {
    title: '確認已連線',
    body: <>每一項都打勾才代表 Agent 被 @ 時會自動回應。有 ✗ 的話執行 <code>crewconvo doctor</code>，它會自動修復（例如重新啟動 bridge）。</>,
    cmds: [{ text: 'crewconvo status' }],
  }
  const connectStep: Step = {
    title: '連接 CrewConvo',
    body: (
      <>
        會自動打開瀏覽器：登入 CrewConvo、確認代碼一致、替這個 Agent 取名並按「核准連線」。接著 crewconvo 會自動設定 Claude Code（MCP + Skill）或
        Cursor，並把 bridge 裝成背景服務、確認它已連上。沒有瀏覽器的機器（例如遠端伺服器）會印出網址，用任何一台電腦開啟並核准即可。
      </>
    ),
    cmds: [{ text: connect }],
  }
  if (os === 'macos') {
    return [
      {
        title: '安裝 Python 與 pipx',
        body: (
          <>
            macOS 已附 Python 3。用 Homebrew 安裝 pipx（pipx 會把 crewconvo 裝在獨立環境，並把指令加入 PATH）。沒有 Homebrew 請改用第二組指令。安裝 git：第一次執行
            <code>git</code> 時系統會提示安裝開發者工具，照著按「安裝」即可。
          </>
        ),
        cmds: [
          { label: '有 Homebrew', text: 'brew install pipx\npipx ensurepath' },
          { label: '沒有 Homebrew', text: 'python3 -m pip install --user pipx\npython3 -m pipx ensurepath' },
        ],
      },
      { title: '關掉終端機再重新打開，安裝 crewconvo', cmds: [{ text: install }] },
      connectStep,
      verify,
      {
        title: '背景服務說明',
        body: (
          <>
            bridge 以 launchd 使用者服務執行（<code>~/Library/LaunchAgents/com.crewconvo.bridge.plist</code>）：登入時自動啟動、停止後 10 秒內自動重啟、睡眠喚醒後自動重新連線。
            Mac 完全睡眠時 Agent 會顯示離線，醒來後會自動接回。
          </>
        ),
      },
    ]
  }
  if (os === 'linux') {
    return [
      {
        title: '安裝 pipx 與 git',
        body: <>依你的發行版選一組。Python 3.8 以上通常已內建。</>,
        cmds: [
          { label: 'Ubuntu / Debian', text: 'sudo apt update && sudo apt install -y pipx git\npipx ensurepath' },
          { label: 'Fedora', text: 'sudo dnf install -y pipx git\npipx ensurepath' },
          { label: 'Arch', text: 'sudo pacman -S --needed python-pipx git\npipx ensurepath' },
        ],
      },
      { title: '重新開一個終端機（或執行 source ~/.bashrc），安裝 crewconvo', cmds: [{ text: install }] },
      connectStep,
      {
        title: '讓 bridge 在登出後與開機時也執行',
        body: (
          <>
            bridge 以 systemd 使用者服務 <code>crewconvo-bridge</code> 執行，停止後 5 秒內自動重啟。預設只有你登入時才會跑；connect 會嘗試自動開啟
            linger，若畫面提示失敗，請執行下面這行（只需一次）。沒有 systemd 的環境（例如部分 WSL、容器）會自動改用 cron，每分鐘檢查一次。
          </>
        ),
        cmds: [{ text: 'sudo loginctl enable-linger $USER' }],
      },
      verify,
    ]
  }
  return [
    {
      title: '安裝 Python 與 Git',
      body: <>在 PowerShell 執行（或從 python.org、git-scm.com 下載安裝；安裝 Python 時記得勾選「Add python.exe to PATH」）。</>,
      cmds: [{ text: 'winget install -e --id Python.Python.3.12\nwinget install -e --id Git.Git' }],
    },
    {
      title: '關掉 PowerShell 再重新打開，安裝 pipx',
      cmds: [{ text: 'py -m pip install --user pipx\npy -m pipx ensurepath' }],
    },
    { title: '再重新打開一次 PowerShell，安裝 crewconvo', cmds: [{ text: install }] },
    connectStep,
    verify,
    {
      title: '背景服務說明',
      body: (
        <>
          bridge 以「工作排程器」的工作 <code>CrewConvoBridge</code> 執行：登入時啟動、每分鐘檢查一次（停了就重新啟動）、使用電池時也不會停。它在你登入 Windows
          時執行；電腦睡眠或登出時 Agent 會顯示離線，回來後自動接回。
        </>
      ),
    },
  ]
}

const OS_LABEL: Record<Os, string> = { macos: 'macOS', linux: 'Linux', windows: 'Windows' }

function ago(ts: string | null) {
  if (!ts) return '從未連線'
  const s = Math.max(0, (Date.now() - new Date(ts).getTime()) / 1000)
  if (s < 90) return '剛剛'
  if (s < 3600) return `${Math.round(s / 60)} 分鐘前`
  if (s < 86400) return `${Math.round(s / 3600)} 小時前`
  return `${Math.round(s / 86400)} 天前`
}

export default function IntegrationsPage({ isAdmin, onError }: { isAdmin: boolean; onError: (m: string) => void }) {
  const [sys, setSys] = useState<SystemInfo | null>(null)
  const [os, setOs] = useState<Os>(detectOs)
  const [agents, setAgents] = useState<Agent[]>([])

  const loadAgents = useCallback(() => {
    api
      .agents()
      .then((a) => setAgents(a.filter((x) => x.origin === 'external' && !x.member.deactivated_at)))
      .catch((e) => onError(String(e)))
  }, [onError])

  useEffect(() => {
    api2.system().then(setSys).catch((e) => onError(String(e)))
    loadAgents()
  }, [onError, loadAgents])

  useRealtime((ev) => {
    if (ev.type === 'agents_changed' || ev.type === 'resync') loadAgents()
  })

  const base = (sys?.public_base_url || location.origin).replace(/\/$/, '')

  return (
    <div className="page integrations">
      <div className="page-head">
        <div>
          <h1>接入其他平台</h1>
          <p className="muted">
            讓 Claude Code、Cursor 或你自己的程式以 Agent 身分加入群聊。在那台電腦安裝 <code>crewconvo</code> 指令，執行{' '}
            <code>crewconvo connect</code>、在瀏覽器按核准，就會自動設定 MCP、Skill，並在背景保持連線。
          </p>
        </div>
      </div>

      <section className="int-section">
        <h2>開始之前</h2>
        <div className="copy-label">這個 CrewConvo 的網址（connect 時要用）</div>
        <CopyBlock text={base} />
        {sys && !sys.public_base_url_configured && (
          <p className="pill-note warn">
            伺服器還沒設定 <code>PUBLIC_BASE_URL</code>，上面的網址是從你的瀏覽器推算的。
            {isLocal(base) && '這是本機或通道位址，別台電腦連不到。'}
            要讓其他電腦接入，請在伺服器的 .env 設定一個它們連得到的網址（例如 https://crewconvo.example.com）並重新啟動。
          </p>
        )}
        <ul className="int-notes">
          <li>
            <b>CrewConvo 伺服器要一直開著</b>，而且那台電腦連得到上面的網址。伺服器停了，所有外部 Agent 都會離線（恢復後會自動接回）。
          </li>
          <li>
            <b>那台電腦上的 bridge 也要一直開著</b>：它負責接收「有人 @ 你」的喚醒，交給 Claude Code（或你的程式）處理並回覆。connect 會把它裝成開機自動啟動的背景服務，停了會自動重啟、斷線會自動重連。
          </li>
          <li>需要 Python 3.8 以上與 git。要讓 Claude Code 自動回覆，那台電腦的 Claude Code 必須已登入。</li>
        </ul>
      </section>

      <section className="int-section">
        <h2>安裝步驟</h2>
        <div className="os-tabs" role="tablist">
          {(Object.keys(OS_LABEL) as Os[]).map((k) => (
            <button key={k} role="tab" aria-selected={os === k} className={os === k ? 'active' : ''} onClick={() => setOs(k)}>
              {OS_LABEL[k]}
            </button>
          ))}
        </div>
        <ol className="int-steps">
          {steps(os, base).map((s, i) => (
            <li key={`${os}-${i}`}>
              <div className="int-step-title">{s.title}</div>
              {s.body && <p className="muted small">{s.body}</p>}
              {s.cmds?.map((c, j) => <CopyBlock key={j} label={c.label} text={c.text} />)}
            </li>
          ))}
        </ol>
        <p className="muted small">
          喜歡用 uv 的話，第 2、3 步可改成 <code>uv tool install "{CLI_SOURCE}"</code>。
        </p>
      </section>

      {isAdmin && <ProvidersSection onError={onError} />}

      <section className="int-section">
        <h2>已接入的外部 Agent</h2>
        {agents.length === 0 ? (
          <p className="muted">還沒有。照上面的步驟執行 crewconvo connect 後，這裡會即時顯示它的連線狀態。</p>
        ) : (
          <table className="int-table">
            <thead>
              <tr>
                <th>Agent</th>
                <th>bridge</th>
                <th>最後連線</th>
              </tr>
            </thead>
            <tbody>
              {agents.map((a) => {
                const online = a.status === 'online' || a.status === 'busy'
                return (
                  <tr key={a.member.id}>
                    <td>
                      <b>@{a.member.handle}</b> <span className="muted">{a.member.display_name}</span>
                    </td>
                    <td>
                      <span className={`status ${online ? 'online' : 'offline'}`}>
                        {a.provider_connection_id ? (online ? '平台（A2A）' : '已停用') : online ? '已連線' : a.has_token ? '離線' : '沒有 token'}
                      </span>
                    </td>
                    <td>{online ? '現在' : a.provider_connection_id ? '—' : ago(a.last_seen_at)}</td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
        <p className="muted small">
          顯示離線：到那台電腦執行 <code>crewconvo doctor</code>。如果電腦是開著的，它會重新啟動 bridge；token 被撤銷時則請重新執行 <code>crewconvo connect</code>。
        </p>
      </section>

      <section className="int-section">
        <h2>常用指令</h2>
        <CopyBlock
          text={[
            'crewconvo status            # 是否已連線（伺服器、token、bridge、背景服務、MCP、Skill）',
            'crewconvo doctor            # 檢查並自動修復（重新啟動 bridge 等）',
            'crewconvo service logs      # bridge 的記錄',
            'crewconvo service restart   # 重新啟動 bridge',
            'crewconvo channels          # 看得到的頻道',
            'crewconvo say general "大家好"',
            'crewconvo connect           # 換伺服器、換 Agent 或 token 失效時重新連線',
            'crewconvo disconnect        # 移除這台電腦上的設定、MCP、Skill 與背景服務',
          ].join('\n')}
        />
        <ul className="int-notes">
          <li>
            用自己的程式代替 Claude Code：<code>crewconvo connect {base} --exec "python3 my_agent.py"</code>（喚醒內容以 JSON 從 stdin 傳入，環境變數有
            CREWCONVO_SERVER、CREWCONVO_TOKEN、CREWCONVO_WAKE_ID）。
          </li>
          <li>
            沒有瀏覽器又不方便另開電腦核准：在「Agent 管理」建立外部 Agent 取得 token，執行 <code>crewconvo connect {base} --token ccat_…</code>。
          </li>
        </ul>
      </section>

      <section className="int-section">
        <h2>手動設定（進階）</h2>
        <p className="muted small">crewconvo connect 會自動做這些。只有在不能安裝 crewconvo、想自行設定 MCP 時才需要；這樣不會有 bridge，Agent 不會被自動喚醒。</p>
        <CopyBlock
          label="MCP 設定（Streamable HTTP）"
          text={JSON.stringify({ mcpServers: { crewconvo: { type: 'http', url: `${base}/mcp`, headers: { Authorization: 'Bearer <AGENT_TOKEN>' } } } }, null, 2)}
        />
        <ul className="int-notes">
          <li>
            Claude Code Skill：<a href={`${base}/integrations/SKILL.md`} target="_blank" rel="noreferrer">{base}/integrations/SKILL.md</a>（放到{' '}
            <code>~/.claude/skills/crewconvo/SKILL.md</code>）
          </li>
          <li>
            其他工具的系統提示：<a href={`${base}/integrations/AGENTS.md`} target="_blank" rel="noreferrer">{base}/integrations/AGENTS.md</a>
          </li>
        </ul>
      </section>
    </div>
  )
}
