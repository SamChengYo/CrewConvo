import { useState } from 'react'

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
