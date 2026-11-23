import { nextTick } from 'vue'

export function createStreamPool(limit = 5) {
  const waiting = new Set()
  const playing = new Set()
  let scheduled = false

  function schedule() {
    if (scheduled) return
    scheduled = true
    nextTick(() => {
      scheduled = false
      for (const stream of waiting) {
        if (playing.size >= limit) break
        waiting.delete(stream)
        playing.add(stream)
        stream.notify(true)
      }
    })
  }

  return (notify) => {
    const stream = { notify }
    waiting.add(stream)
    schedule()
    return () => {
      waiting.delete(stream)
      if (!playing.delete(stream)) return
      notify(false)
      schedule()
    }
  }
}

export const reservePreviewStream = createStreamPool()
