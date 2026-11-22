import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import Dashboard from '../src/views/Dashboard.vue'
import { adminToken } from '../src/api'

function mockSystem(cameras) {
  const fetch = vi.fn(async url => {
    const data = url === '/api/system'
      ? { version: '0.6.0', backend: 'rust', providers: ['DmlExecutionProvider'], cameras }
      : {}
    return { ok: true, json: async () => data }
  })
  vi.stubGlobal('fetch', fetch)
  return fetch
}

enableAutoUnmount(afterEach)
beforeEach(() => adminToken.set('adm'))
afterEach(() => {
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('首页监控状态与更新入口', () => {
  it('显示实时摄像头的运行、连接、离线和停止状态', async () => {
    mockSystem([
      { id: 1, type: 'rtsp', status: 'running' },
      { id: 2, type: 'rtsp', status: 'starting' },
      { id: 3, type: 'usb', status: 'error' },
      { id: 4, type: 'usb', status: 'stopped' },
      { id: 5, type: 'folder', status: 'running' },
      null,
    ])
    const view = mount(Dashboard)
    await flushPromises()
    const card = view.get('.monitor-card')
    expect(card.get('.num').text()).toContain('1 / 4')
    expect(card.get('.stat-detail').text()).toBe('1 路连接中 · 1 路离线 · 1 路停止')
    expect(card.get('a').attributes('href')).toBe('#/sources/cams')
    expect(view.text()).not.toContain('Rust')
    expect(view.text()).not.toContain('DirectML')
  })

  it('空实例提供接入入口，异常响应显示状态不可用', async () => {
    mockSystem([])
    const empty = mount(Dashboard)
    await flushPromises()
    expect(empty.get('.monitor-card a').text()).toBe('添加摄像头')
    mockSystem({ state: 'unknown' })
    const unavailable = mount(Dashboard)
    await flushPromises()
    expect(unavailable.get('.monitor-card').text()).toContain('暂时无法读取摄像头状态')
  })

  it('显示本地版本，查看更新直接前往正式 Release', async () => {
    const fetch = mockSystem([])
    const view = mount(Dashboard)
    await flushPromises()
    const link = view.get('a[target="_blank"]')
    expect(view.get('.version').text()).toBe('0.6.0')
    expect(link.attributes('href')).toBe('https://github.com/windyxyz/LabSentinel/releases/latest')
    expect(link.attributes('rel')).toBe('noopener noreferrer')
    expect(fetch.mock.calls.some(([url]) => url.includes('/update/check'))).toBe(false)
  })
})
