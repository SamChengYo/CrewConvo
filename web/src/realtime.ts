import { useEffect, useRef } from 'react'
import type { RealtimeEvent } from './api'

type Listener = (ev: RealtimeEvent) => void

/** Single reconnecting WebSocket shared by the whole app. */
class Realtime {
  private ws: WebSocket | null = null
  private listeners = new Set<Listener>()
  private retry = 0
  private stopped = true

  start() {
    if (!this.stopped) return
    this.stopped = false
    this.connect()
  }

  stop() {
    this.stopped = true
    this.ws?.close()
    this.ws = null
  }

  private connect() {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws'
    const ws = new WebSocket(`${proto}://${location.host}/api/ws`)
    this.ws = ws
    ws.onopen = () => {
      if (this.retry > 0) this.emit({ type: 'resync' })
      this.retry = 0
    }
    ws.onmessage = (e) => {
      try {
        this.emit(JSON.parse(e.data) as RealtimeEvent)
      } catch {
        /* ignore malformed frames */
      }
    }
    ws.onclose = () => {
      if (this.stopped) return
      this.retry = Math.min(this.retry + 1, 6)
      setTimeout(() => this.connect(), 500 * 2 ** this.retry)
    }
  }

  private emit(ev: RealtimeEvent) {
    this.listeners.forEach((l) => l(ev))
  }

  subscribe(l: Listener) {
    this.listeners.add(l)
    return () => {
      this.listeners.delete(l)
    }
  }
}

export const realtime = new Realtime()

/** Subscribes to realtime events; the latest handler is always used. */
export function useRealtime(handler: Listener) {
  const ref = useRef(handler)
  ref.current = handler
  useEffect(() => realtime.subscribe((ev) => ref.current(ev)), [])
}
