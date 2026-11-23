import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, it, vi } from 'vitest'
import CameraPreview from '../src/components/CameraPreview.vue'
import VideoOverlayCanvas from '../src/components/VideoOverlayCanvas.vue'

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

it('the preview only polls in view and reuses its snapshot after a 304', async () => {
  vi.useFakeTimers()
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false)
  const disconnect = vi.fn()
  let visibility
  vi.stubGlobal('IntersectionObserver', class {
    constructor(callback) { visibility = callback }
    observe() {}
    disconnect() { disconnect() }
  })
  const PreviousURL = URL
  vi.stubGlobal('URL', class extends PreviousURL {
    static createObjectURL() { return 'blob:frame' }
    static revokeObjectURL() {}
  })
  const first = {
    ok: true,
    headers: new Headers({ etag: '"frame-a"', 'x-ls-frame-time-ms': '100' }),
    blob: async () => new Blob(['frame']),
  }
  const fetch = vi.fn()
    .mockResolvedValueOnce(first)
    .mockResolvedValue({ ok: false, status: 304 })
  vi.stubGlobal('fetch', fetch)
  const component = mount(CameraPreview, { props: { cameraId: 9, cameraType: 'usb', name: 'USB' } })
  try {
    await vi.advanceTimersByTimeAsync(6000)
    expect(fetch).not.toHaveBeenCalled()
    visibility([{ isIntersecting: true }])
    await vi.advanceTimersByTimeAsync(0)
    expect(component.get('img').attributes('src')).toBe('blob:frame')
    await vi.advanceTimersByTimeAsync(1500)
    expect(fetch.mock.calls[1][1].headers['If-None-Match']).toBe('"frame-a"')
    expect(component.get('img').attributes('src')).toBe('blob:frame')
    visibility([{ isIntersecting: false }])
    await vi.advanceTimersByTimeAsync(6000)
    expect(fetch).toHaveBeenCalledTimes(2)
  } finally {
    component.unmount()
  }
  expect(disconnect).toHaveBeenCalledOnce()
})

it('the overlay coalesces incoming frames and closes its socket in a hidden tab', async () => {
  const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(false)
  const raf = vi.fn().mockReturnValue(1)
  vi.stubGlobal('requestAnimationFrame', raf)
  vi.stubGlobal('cancelAnimationFrame', vi.fn())
  vi.stubGlobal('IntersectionObserver', undefined)
  vi.stubGlobal('ResizeObserver', undefined)
  let socket
  vi.stubGlobal('WebSocket', class {
    constructor() { socket = this }
    close = vi.fn()
  })
  const component = mount(VideoOverlayCanvas, { props: { cameraId: 9 } })
  try {
    expect(raf).not.toHaveBeenCalled()
    for (let i = 0; i < 3; i++) {
      socket.onmessage({ data: '{"camera_id":9,"tracks":[]}' })
    }
    expect(raf).toHaveBeenCalledOnce()
    hidden.mockReturnValue(true)
    document.dispatchEvent(new Event('visibilitychange'))
    expect(socket.close).toHaveBeenCalledOnce()
    await flushPromises()
    expect(component.emitted('tracks').at(-1)).toEqual([0])
  } finally {
    component.unmount()
  }
})
