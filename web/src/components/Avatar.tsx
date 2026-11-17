import type { MemberKind } from '../api'

const PALETTE = ['#e8912d', '#2bac76', '#1264a3', '#e01e5a', '#4a154b', '#36c5f0', '#9e5cba', '#d6503f', '#0b7a75', '#7c5a12']

function color(s: string) {
  let h = 0
  for (const ch of s) h = (h * 31 + ch.codePointAt(0)!) >>> 0
  return PALETTE[h % PALETTE.length]
}

export function Avatar({
  name,
  kind,
  size = 36,
  presence,
}: {
  name: string
  kind: MemberKind
  size?: number
  presence?: 'online' | 'offline' | null
}) {
  const initial = [...name.trim()][0]?.toUpperCase() ?? '?'
  const radius = Math.max(4, Math.round(size * 0.22))
  return (
    <span className="avatar-wrap" style={{ width: size, height: size }}>
      {kind === 'system' ? (
        <span className="avatar system" style={{ width: size, height: size, borderRadius: radius, fontSize: size * 0.5 }}>
          ⓘ
        </span>
      ) : (
        <span
          className={`avatar ${kind}`}
          style={{ width: size, height: size, borderRadius: radius, fontSize: size * 0.44, background: color(name) }}
        >
          {initial}
          {kind === 'agent' && size >= 28 && <span className="avatar-bot">✦</span>}
        </span>
      )}
      {presence && <span className={`presence ${presence}`} />}
    </span>
  )
}
