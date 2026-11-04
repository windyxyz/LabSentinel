// 任务133：notify.js 契约测试（mock Notification/localStorage/document，零真弹窗）。
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

let notifications = []
let permission = 'default'
let hidden = true

vi.mock('../src/notify', async () => {
  const mod = await vi.importActual('../src/notify')
  return mod
})

function installNotification(permissionValue) {
  permission = permissionValue
  class FakeNotification {
    constructor(title, opts = {}) {
      this.title = title
      this.opts = opts
      notifications.push(this)
      this.onclose = null
    }

    close() {
      this.closed = true
    }
  }
  FakeNotification.permission = permissionValue
  FakeNotification.requestPermission = vi.fn().mockResolvedValue(permissionValue)
  vi.stubGlobal('Notification', FakeNotification)
}

beforeEach(() => {
  notifications = []
  hidden = true
  localStorage.clear()
  vi.stubGlobal('document', { ...document, hidden })
})

afterEach(() => {
  vi.unstubAllGlobals()
})

// document.hidden 是 getter，不能直接 stub 属性——动态 mock
async function loadModule() {
  const mod = await import('../src/notify')
  return mod
}

describe('notify（任务115/133）', () => {
  it('权限未授予（default）不弹', async () => {
    installNotification('default')
    const { notifyEvent } = await loadModule()
    notifyEvent({ id: 1, camera_id: 1, type: 'enter_no_ppe', level: 3 })
    expect(notifications).toHaveLength(0)
  })

  it('Notification 不支持的环境不弹不抛', async () => {
    vi.stubGlobal('Notification', undefined)
    const { notifyEvent } = await loadModule()
    expect(() => notifyEvent({ id: 1, camera_id: 1, type: 'enter', level: 3 })).not.toThrow()
  })

  it('L1 不弹；L2/L3 弹且 tag=cam-<id>-<type>', async () => {
    installNotification('granted')
    const { notifyEvent } = await loadModule()
    notifyEvent({ id: 1, camera_id: 1, type: 'enter', level: 1 })
    expect(notifications).toHaveLength(0)
    notifyEvent({ id: 2, camera_id: 2, type: 'enter_no_ppe', level: 3, detail: 'd' })
    expect(notifications).toHaveLength(1)
    expect(notifications[0].opts.tag).toBe('cam-2-enter_no_ppe')
  })

  it('页面可见（hidden=false）不弹', async () => {
    installNotification('granted')
    hidden = false
    vi.stubGlobal('document', { ...document, get hidden() { return hidden } })
    const { notifyEvent } = await loadModule()
    notifyEvent({ id: 3, camera_id: 1, type: 'enter', level: 3 })
    expect(notifications).toHaveLength(0)
  })

  it('同 tag 10s 内去重（vi.useFakeTimers）', async () => {
    installNotification('granted')
    vi.useFakeTimers()
    const { notifyEvent } = await loadModule()
    const ev = { id: 4, camera_id: 3, type: 'enter', level: 2 }
    notifyEvent(ev)
    notifyEvent(ev)
    expect(notifications).toHaveLength(1)
    vi.advanceTimersByTime(10_001)
    notifyEvent(ev)
    expect(notifications).toHaveLength(2)
    vi.useRealTimers()
  })

  it('开关关闭（localStorage=0）不弹', async () => {
    installNotification('granted')
    const { notifyEvent, setNotifyEnabled } = await loadModule()
    setNotifyEnabled(false)
    notifyEvent({ id: 5, camera_id: 1, type: 'enter', level: 3 })
    expect(notifications).toHaveLength(0)
    setNotifyEnabled(true)
    notifyEvent({ id: 6, camera_id: 1, type: 'enter', level: 3 })
    expect(notifications).toHaveLength(1)
  })

  it('ensurePermission：granted 时不再请求；default 时请求并返回', async () => {
    installNotification('granted')
    const { ensurePermission } = await loadModule()
    expect(await ensurePermission()).toBe('granted')
    installNotification('default')
    const mod = await loadModule()
    expect(await mod.ensurePermission()).toBe('default')
    expect(globalThis.Notification.requestPermission).toHaveBeenCalledTimes(1)
  })
})
