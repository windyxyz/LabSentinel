// LS-051：VideoOverlayCanvas 坐标缩放 / 合规态颜色 + Wall 宫格与静音开关。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { mapBox, stateColor } from '../src/overlay.js'

describe('VideoOverlayCanvas 纯函数', () => {
  it('mapBox：等比缩放并居中（letterbox）', () => {
    // 1280x720 源 → 640x360 容器：scale=0.5，无偏移
    const r = mapBox([100, 50, 300, 250], 1280, 720, 640, 360)
    expect(r.scale).toBeCloseTo(0.5, 5)
    expect(r.x).toBeCloseTo(50, 5)
    expect(r.y).toBeCloseTo(25, 5)
    expect(r.w).toBeCloseTo(100, 5)
    expect(r.h).toBeCloseTo(100, 5)
  })

  it('mapBox：容器更宽时左右留边', () => {
    // 1280x720 → 1280x720 容器高度受限：scale=0.5, offX=(1280-640)/2=320
    const r = mapBox([0, 0, 1280, 720], 1280, 720, 1280, 360)
    expect(r.scale).toBeCloseTo(0.5, 5)
    expect(r.offX).toBeCloseTo(320, 5)
    expect(r.x).toBeCloseTo(320, 5)
  })

  it('mapBox：非法输入返回 null', () => {
    expect(mapBox(null, 1280, 720, 640, 360)).toBeNull()
    expect(mapBox([1, 2, 3], 1280, 720, 640, 360)).toBeNull()
    expect(mapBox([1, 2, 3, 4], 0, 720, 640, 360)).toBeNull()
    expect(mapBox([NaN, 2, 3, 4], 1280, 720, 640, 360)).toBeNull()
  })

  it('stateColor：绿/红/灰三态', () => {
    expect(stateColor('compliant')).toBe('#22c55e')
    expect(stateColor('violation')).toBe('#ef4444')
    expect(stateColor('pending')).toBe('#9ca3af')
    expect(stateColor(undefined)).toBe('#9ca3af')
  })
})

function mockFetch(responses) {
  const fn = vi.fn().mockImplementation((url, opts = {}) => {
    const key = `${opts.method || 'GET'} ${url.split('?')[0]}`
    const body = responses[key]
    if (body === undefined) return Promise.resolve({ ok: false, status: 404, json: async () => ({}) })
    return Promise.resolve({ ok: true, status: 200, json: async () => body })
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

afterEach(() => { vi.unstubAllGlobals() })

describe('Wall.vue（LS-051 全屏值守）', () => {
  it('默认 9 宫格 + 检测框显隐 + 静音开关', async () => {
    vi.stubGlobal('WebSocket', class {
      constructor() { this.readyState = 0 }
      close() {}
      set onmessage(_) {}
    })
    vi.stubGlobal('requestAnimationFrame', () => 0)
    vi.stubGlobal('cancelAnimationFrame', () => {})
    mockFetch({
      'GET /api/cameras': [],
      'GET /api/events': [],
      'GET /api/stats/summary': { today_total: 10, today_compliant: 9 },
    })
    const Wall = (await import('../src/views/Wall.vue')).default
    const w = mount(Wall)
    await flushPromises()
    expect(w.text()).toContain('进入全屏值守大屏')
    expect(w.text()).toContain('隐藏检测框')
    expect(w.text()).toContain('开启提示音')
    // 默认 9 宫格
    expect(w.find('select[aria-label="宫格切换"]').element.value).toBe('9')
    // 切到 4 宫格
    await w.find('select[aria-label="宫格切换"]').setValue('4')
    expect(w.find('select[aria-label="宫格切换"]').element.value).toBe('4')
    // 静音 → 开启提示音
    const muteBtn = w.findAll('button').find((b) => b.text() === '开启提示音')
    await muteBtn.trigger('click')
    expect(w.text()).toContain('静音')
  })
})
