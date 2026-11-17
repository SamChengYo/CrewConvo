import type { ReactNode } from 'react'

/**
 * Small, safe Markdown renderer for chat messages (no HTML injection):
 * fenced code blocks, inline code, **bold**, *italic*, ~~strike~~, links, @mentions,
 * headings, bullet / numbered lists, block quotes.
 */

const URL_RE = /(https?:\/\/[^\s<>()]+[^\s<>().,;:!?，。！？）])/

function inline(text: string, mentions: Set<string>, key: string): ReactNode[] {
  const out: ReactNode[] = []
  // Tokenize: code, bold, italic, strike, url, mention
  const re = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*)|(~~[^~\n]+~~)|(\*[^*\n]+\*)|(https?:\/\/[^\s<>()]+[^\s<>().,;:!?，。！？）])|(@[\p{L}\p{N}_.-]+)/gu
  let last = 0
  let m: RegExpExecArray | null
  let i = 0
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index))
    const t = m[0]
    const k = `${key}-${i++}`
    if (m[1]) out.push(<code key={k} className="md-code">{t.slice(1, -1)}</code>)
    else if (m[2]) out.push(<strong key={k}>{inline(t.slice(2, -2), mentions, k)}</strong>)
    else if (m[3]) out.push(<s key={k}>{t.slice(2, -2)}</s>)
    else if (m[4]) out.push(<em key={k}>{t.slice(1, -1)}</em>)
    else if (m[5] && URL_RE.test(t))
      out.push(
        <a key={k} href={t} target="_blank" rel="noreferrer noopener">
          {t}
        </a>,
      )
    else if (m[6]) {
      // A name can be glued to the text after it (@客服助理請幫我): highlight the longest known
      // handle the token starts with, as the server does.
      const h = t.slice(1).replace(/\.+$/, '').toLowerCase()
      let hit = mentions.has(h) || h === 'all' ? h : ''
      if (!hit)
        for (const cand of mentions) {
          const next = h.codePointAt(cand.length)
          if (cand.length > hit.length && h.startsWith(cand) && next !== undefined && next > 127) hit = cand
        }
      if (hit) {
        out.push(
          <span key={k} className="mention">
            {t.slice(0, hit.length + 1)}
          </span>,
        )
        if (t.length > hit.length + 1) out.push(t.slice(hit.length + 1))
      } else out.push(t)
    } else out.push(t)
    last = m.index + t.length
  }
  if (last < text.length) out.push(text.slice(last))
  return out
}

export default function Markdown({ text, mentions = [] }: { text: string; mentions?: string[] }) {
  const ms = new Set(mentions.map((m) => m.toLowerCase()))
  const lines = text.replace(/\r\n/g, '\n').split('\n')
  const blocks: ReactNode[] = []
  let i = 0
  let para: string[] = []
  const flush = () => {
    if (para.length) {
      const k = `p${blocks.length}`
      blocks.push(
        <p key={k} className="md-p">
          {para.map((l, j) => (
            <span key={j}>
              {j > 0 && <br />}
              {inline(l, ms, `${k}-${j}`)}
            </span>
          ))}
        </p>,
      )
      para = []
    }
  }
  while (i < lines.length) {
    const line = lines[i]
    const fence = line.match(/^\s*```(\w*)\s*$/)
    if (fence) {
      flush()
      const code: string[] = []
      i++
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) code.push(lines[i++])
      i++
      blocks.push(
        <pre key={`c${blocks.length}`} className="md-pre">
          <code>{code.join('\n')}</code>
        </pre>,
      )
      continue
    }
    const heading = line.match(/^\s*#{1,6}\s+(.*)$/)
    if (heading) {
      flush()
      blocks.push(
        <p key={`h${blocks.length}`} className="md-h">
          <strong>{inline(heading[1], ms, `h${blocks.length}`)}</strong>
        </p>,
      )
      i++
      continue
    }
    if (/^\s*([-*•]|\d+[.)])\s+/.test(line)) {
      flush()
      const ordered = /^\s*\d+[.)]/.test(line)
      const items: string[] = []
      while (i < lines.length && /^\s*([-*•]|\d+[.)])\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*([-*•]|\d+[.)])\s+/, ''))
        i++
      }
      const k = `l${blocks.length}`
      const lis = items.map((it, j) => <li key={j}>{inline(it, ms, `${k}-${j}`)}</li>)
      blocks.push(ordered ? <ol key={k} className="md-list">{lis}</ol> : <ul key={k} className="md-list">{lis}</ul>)
      continue
    }
    if (/^\s*>\s?/.test(line)) {
      flush()
      const q: string[] = []
      while (i < lines.length && /^\s*>\s?/.test(lines[i])) q.push(lines[i++].replace(/^\s*>\s?/, ''))
      blocks.push(
        <blockquote key={`q${blocks.length}`} className="md-quote">
          {q.map((l, j) => (
            <div key={j}>{inline(l, ms, `q${blocks.length}-${j}`)}</div>
          ))}
        </blockquote>,
      )
      continue
    }
    if (line.trim() === '') {
      flush()
      i++
      continue
    }
    para.push(line)
    i++
  }
  flush()
  return <div className="md">{blocks}</div>
}
