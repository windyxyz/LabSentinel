import { afterEach, expect, it, vi } from 'vitest'
import { connectEvents } from '../src/api'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

it('resumes at the last event after a disconnect without replaying it to the page', async () => {
  vi.useFakeTimers()
  const sockets = []
  vi.stubGlobal('WebSocket', class {
    constructor(url) {
      this.url = url
      sockets.push(this)
    }
    close() {}
  })
  const onEvent = vi.fn()
  const stop = connectEvents(onEvent)
  const send = (socket, value) => socket.onmessage({ data: JSON.stringify(value) })
  try {
    expect(new URL(sockets[0].url).searchParams.get('since')).toBe('latest')
    send(sockets[0], { stream_cursor: 12 })
    send(sockets[0], { id: 13, detail: 'first' })
    sockets[0].onclose()
    await vi.advanceTimersByTimeAsync(3000)
    expect(new URL(sockets[1].url).searchParams.get('since')).toBe('13')
    send(sockets[1], { stream_cursor: 13 })
    send(sockets[1], { id: 13, detail: 'duplicate' })
    send(sockets[1], { id: 14, detail: 'replayed' })
    expect(onEvent.mock.calls.map(([event]) => event.id)).toEqual([13, 14])
  } finally {
    stop()
  }
})

it('opens from the history cursor and reports an expired history without disrupting replay', () => {
  let socket
  vi.stubGlobal('WebSocket', class {
    constructor(url) { this.url = url; socket = this }
    close() {}
  })
  const onEvent = vi.fn()
  const onHistoryReset = vi.fn()
  const stop = connectEvents(onEvent, null, null, { since: 42, onHistoryReset })
  const send = value => socket.onmessage({ data: JSON.stringify(value) })
  try {
    expect(new URL(socket.url).searchParams.get('since')).toBe('42')
    send({ stream_cursor: 42, stream_history_reset: true })
    send({ id: 45 })
    send({ id: 45 })
    expect(onHistoryReset).toHaveBeenCalledTimes(1)
    expect(onEvent.mock.calls.map(([event]) => event.id)).toEqual([45])
  } finally {
    stop()
  }
})
