const BASE = ''

// 管理令牌（任务16）：localStorage 持久化，启用鉴权的服务端要求写操作携带 X-Admin-Token
export const adminToken = {
  get: () => localStorage.getItem('admin_token') || '',
  set: (v) => v ? localStorage.setItem('admin_token', v) : localStorage.removeItem('admin_token'),
}

// 105 回归补丁 P0-2：读保护开启时 <img>/<video>/<a download> 裸 URL 不带请求头会 401——
// 媒体/下载地址统一走这里，有令牌时追加 ?token=（URL token 会进历史/代理日志，服务端文档已注明）
export function mediaUrl(path) {
  const t = adminToken.get()
  if (!t) return path
  return path + (path.includes('?') ? '&' : '?') + 'token=' + encodeURIComponent(t)
}

async function req(method, path, body) {
  const isForm = body instanceof FormData
  const headers = body && !isForm ? { 'Content-Type': 'application/json' } : {}
  const token = adminToken.get()
  if (token) headers['X-Admin-Token'] = token
  const r = await fetch(BASE + path, {
    method,
    headers,
    body: body ? (isForm ? body : JSON.stringify(body)) : undefined,
  })
  if (!r.ok) {
    let msg = `${method} ${path} -> ${r.status}`
    try { msg = (await r.json()).detail || msg } catch {}
    if (r.status === 401) msg = '需要管理令牌：请在「摄像头」页顶部填写管理令牌后重试。' + msg
    throw new Error(msg)
  }
  return r.json()
}

export const api = {
  system: () => req('GET', '/api/system'),
  cameras: () => req('GET', '/api/cameras'),
  usbDevices: (refresh = false) => req('GET', '/api/cameras/usb-devices' + (refresh ? '?refresh=1' : '')),
  addCamera: (d) => req('POST', '/api/cameras', d),
  startCamera: (id) => req('POST', `/api/cameras/${id}/start`),
  delCamera: (id) => req('DELETE', `/api/cameras/${id}`),
  zones: (cid) => req('GET', `/api/zones?camera_id=${cid}`),
  addZone: (d) => req('POST', '/api/zones', d),
  delZone: (id) => req('DELETE', `/api/zones/${id}`),
  copyConfig: (cid, payload) => req('POST', `/api/cameras/${cid}/copy-config`, payload),
  rules: (cid) => req('GET', `/api/rules?camera_id=${cid}`),
  addRule: (d) => req('POST', '/api/rules', d),
  delRule: (id) => req('DELETE', `/api/rules/${id}`),
  events: (params = {}) => req('GET', '/api/events?' + new URLSearchParams(params)),
  eventsCount: (params = {}) => req('GET', '/api/events/count?' + new URLSearchParams(params)),
  alertsStatus: () => req('GET', '/api/alerts/status'),
  alertTest: () => req('POST', '/api/alert-test'),
  alertDeliveries: (params = {}) => req('GET', '/api/alert-deliveries?' + new URLSearchParams(params)),
  alertDeliveryRetry: (id) => req('POST', `/api/alert-deliveries/${id}/retry`),
  alertSilences: (active = 1) => req('GET', '/api/alert-silences' + (active ? '?active=1' : '')),
  addAlertSilence: (d) => req('POST', '/api/alert-silences', d),
  deleteAlertSilence: (id) => req('DELETE', `/api/alert-silences/${id}`),
  alertTemplate: () => req('GET', '/api/alert-template'),
  audit: (params = {}) => req('GET', '/api/audit?' + new URLSearchParams(params)),
  alertTemplatePreview: (d) => req('POST', '/api/alert-template/preview', d),
  setEventStatus: (id, status) => req('POST', `/api/events/${id}/status`, { status }),
  bulkStatus: (payload) => req('POST', '/api/events/bulk-status', payload),
  cameraWebrtc: (id) => req('GET', `/api/cameras/${id}/webrtc`),
  maintenanceStatus: () => req('GET', '/api/maintenance/status'),
  maintenancePreview: () => req('GET', '/api/maintenance/preview'),
  // 任务79：配置导入导出（导出走 fetch+blob，令牌经请求头传递；<a> 裸链接带不了）
  async configExportDownload() {
    const headers = {}
    const token = adminToken.get()
    if (token) headers['X-Admin-Token'] = token
    const r = await fetch('/api/config/export', { headers })
    if (!r.ok) {
      let msg = `GET /api/config/export -> ${r.status}`
      try { msg = (await r.json()).detail || msg } catch {}
      if (r.status === 401) msg = '需要管理令牌：请在「摄像头」页顶部填写管理令牌后重试。' + msg
      throw new Error(msg)
    }
    const blob = await r.blob()
    const d = new Date()
    const name = `labsentinel-config-${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, '0')}${String(d.getDate()).padStart(2, '0')}.json`
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url; a.download = name; a.click()
    URL.revokeObjectURL(url)
    return JSON.parse(await blob.text())
  },
  configImport: (payload) => req('POST', '/api/config/import', payload),
  faceStatus: () => req('GET', '/api/face/status'),
  facePersons: () => req('GET', '/api/face/persons'),
  faceRegister: (formData) => req('POST', '/api/face/persons', formData),
  faceDeletePerson: (subject) => req('DELETE', `/api/face/persons/${encodeURIComponent(subject)}`),
  faceClearAll: () => req('DELETE', '/api/face/persons/all'),
  statsDaily: (params = {}) => req('GET', '/api/stats/daily?' + new URLSearchParams(params)),
  statsWeekly: (params = {}) => req('GET', '/api/stats/weekly?' + new URLSearchParams(params)),
  statsSummary: (params = {}) => req('GET', '/api/stats/summary?' + new URLSearchParams(params)),
  statsHourly: (params = {}) => req('GET', '/api/stats/hourly?' + new URLSearchParams(params)),
}

export const TYPE_TEXT = {
  enter_no_ppe: '进入安全区未穿实验服',
  enter: '进入安全区',
  exit: '离开安全区',
  occupancy_over: '在场人数超限',
  stranger: '陌生人进入', // 人脸模块（默认关闭）产生；与服务端 constants.py 同步
}

export const STATUS_TEXT = {
  new: '未处置',
  acked: '已确认',
  false_positive: '误报',
  ignored: '忽略',
}

export function connectEvents(onEvent, onState) {
  let ws, timer, closed = false
  function open() {
    if (closed) return
    ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws/events')
    ws.onopen = () => onState && onState('ok')
    ws.onmessage = (m) => { try { onEvent(JSON.parse(m.data)) } catch {} }
    ws.onclose = () => {
      // 2026-10-30 审阅修复：cleanup 里 ws.close() 会异步触发 onclose，原实现会重新武装
      // 3s 重连定时器 → 卸载后幽灵连接永久重连（每切一次页面泄漏一条）
      if (closed) return
      onState && onState('reconnecting')
      timer = setTimeout(open, 3000)  // 断线自动重连
    }
  }
  open()
  return () => { closed = true; clearTimeout(timer); ws && ws.close() }
}
