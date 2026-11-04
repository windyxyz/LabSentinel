// 任务115：浏览器桌面通知（原生 Notification，不引依赖）。
// 原则：不主动请求权限（必须用户点按钮）；页面隐藏时才弹；L1 不通知；同 tag 10s 去重；
// 开关存 localStorage（LS_DESKTOP_NOTIFY=0 关闭）。
const TAG_INTERVAL_MS = 10_000
const lastShown = new Map() // tag -> ts

export function notifyEnabled() {
  return localStorage.getItem('LS_DESKTOP_NOTIFY') !== '0'
}

export function setNotifyEnabled(on) {
  localStorage.setItem('LS_DESKTOP_NOTIFY', on ? '1' : '0')
}

export function permissionState() {
  return typeof Notification === 'undefined' ? 'unsupported' : Notification.permission
}

export async function ensurePermission() {
  if (typeof Notification === 'undefined') return 'unsupported'
  if (Notification.permission === 'granted' || Notification.permission === 'denied') {
    return Notification.permission
  }
  try {
    return await Notification.requestPermission()
  } catch {
    return 'denied'
  }
}

export function notifyEvent(ev) {
  // ev: {id, camera_id, type, level, detail?, ts}
  if (!notifyEnabled()) return
  if (typeof Notification === 'undefined' || Notification.permission !== 'granted') return
  if (!document.hidden) return // 页面可见时不打扰
  const level = Number(ev.level || 0)
  if (level < 2) return // 只 L2/L3
  const tag = `cam-${ev.camera_id}-${ev.type}`
  const now = Date.now()
  const last = lastShown.get(tag) || 0
  if (now - last < TAG_INTERVAL_MS) return
  lastShown.set(tag, now)
  const title = `L${level} ${ev.type} · 摄像头 #${ev.camera_id}`
  try {
    const n = new Notification(title, {
      body: String(ev.detail || '').slice(0, 120) || '检测到事件，点击查看',
      tag,
      renotify: true,
    })
    n.onclick = () => {
      window.focus()
      n.close()
    }
  } catch {
    /* 通知失败静默（部分环境禁用） */
  }
}
