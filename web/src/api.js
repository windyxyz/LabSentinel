const BASE = ''

// 管理令牌（任务16）：localStorage 持久化，启用鉴权的服务端要求写操作携带 X-Admin-Token
export const adminToken = {
  get: () => localStorage.getItem('admin_token') || '',
  set: (v) => v ? localStorage.setItem('admin_token', v) : localStorage.removeItem('admin_token'),
}

// T-108（S-07）：签名媒体 URL——令牌不再进 URL/日志。按路径缓存签名参数（页面内复用），
// 首次同步返回带旧 ?token= 的兼容 URL，签名到达后触发自定义事件让视图刷新（异步收尾）。
const _sigCache = new Map()
export function mediaUrl(path) {
  const t = adminToken.get()
  if (!t) return path
  const base = path.split('?')[0]
  const extra = path.includes('?') ? '&' + path.split('?').slice(1).join('&') : ''
  const hit = _sigCache.get(base)
  if (hit && Number(hit.exp) > Date.now() / 1000 + 5) {
    return base + (extra ? '?' + extra.slice(1) : '') + (extra ? '&' : '?') + 'exp=' + hit.exp + '&sig=' + hit.sig
  }
  // 异步批量签名（并发同路径只发一次）
  if (!_sigCache.has(base)) {
    _sigCache.set(base, { exp: 0, sig: '' })
    req('GET', '/api/media/sign?path=' + encodeURIComponent(base))
      .then((r) => {
        _sigCache.set(base, { exp: r.exp, sig: r.sig })
        window.dispatchEvent(new CustomEvent('ls-media-signed', { detail: { base } }))
      })
      .catch(() => _sigCache.delete(base))
  }
  // 签名未到达前回退旧 ?token=（LS_ALLOW_URL_TOKEN=0 时这类 URL 会 401，签名到达后自愈）
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
  cameraGroups: () => req('GET', '/api/camera-groups'),
  setCameraGroup: (id, name) => req('PUT', `/api/cameras/${id}/group`, { group_name: name }),
  bulkCameras: (ids, action) => req('POST', '/api/cameras/bulk', { ids, action }),
  discoverCameras: (params = {}) => req('GET', '/api/discovery/cameras?' + new URLSearchParams(params)),
  probeHost: (d) => req('POST', '/api/discovery/probe', d),
  updateCheck: (params = {}) => req('GET', '/api/update/check?' + new URLSearchParams(params)),
  pluginsList: () => req('GET', '/api/plugins'),
  pluginAction: (id, action) => req('POST', `/api/plugins/${id}/${action}`),
  pluginLogs: (id, tail = 200) => req('GET', `/api/plugins/${id}/logs?tail=${tail}`),
  eventsTimeline: (params = {}) => req('GET', '/api/events/timeline?' + new URLSearchParams(params)),
  startCamera: (id) => req('POST', `/api/cameras/${id}/start`),
  delCamera: (id) => req('DELETE', `/api/cameras/${id}`),
  // 任务121：图片文件夹自动识别
  folderImagesSummary: (params = {}) => req('GET', '/api/folder-images/summary?' + new URLSearchParams(params)),
  folderImages: (params = {}) => req('GET', '/api/folder-images?' + new URLSearchParams(params)),
  folderImage: (id) => req('GET', `/api/folder-images/${id}`),
  folderScan: (id) => req('POST', `/api/folders/${id}/scan`),
  zones: (cid) => req('GET', `/api/zones?camera_id=${cid}`),
  addZone: (d) => req('POST', '/api/zones', d),
  delZone: (id) => req('DELETE', `/api/zones/${id}`),
  copyConfig: (cid, payload) => req('POST', `/api/cameras/${cid}/copy-config`, payload),
  rules: (cid) => req('GET', `/api/rules?camera_id=${cid}`),
  addRule: (d) => req('POST', '/api/rules', d),
  delRule: (id) => req('DELETE', `/api/rules/${id}`),
  calendar: () => req('GET', '/api/calendar'),
  setCalendar: (d) => req('POST', '/api/calendar', d),
  delCalendar: (date) => req('DELETE', `/api/calendar/${date}`),
  ruleTemplates: () => req('GET', '/api/rule-templates'),
  createRuleTemplate: (d) => req('POST', '/api/rule-templates', d),
  applyRuleTemplate: (id, d) => req('POST', `/api/rule-templates/${id}/apply`, d),
  delRuleTemplate: (id) => req('DELETE', `/api/rule-templates/${id}`),
  events: (params = {}) => req('GET', '/api/events?' + new URLSearchParams(params)),
  eventsCount: (params = {}) => req('GET', '/api/events/count?' + new URLSearchParams(params)),
  alertsStatus: () => req('GET', '/api/alerts/status'),
  alertConfig: () => req('GET', '/api/alerts/config'),
  saveAlertConfig: (data) => req('PUT', '/api/alerts/config', data),
  resetAlertConfig: (channel) => req('POST', '/api/alerts/config/reset' + (channel ? `?channel=${encodeURIComponent(channel)}` : '')),
  alertTest: (channel) => req('POST', '/api/alert-test' + (channel ? `?channel=${encodeURIComponent(channel)}` : '')),
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
  shutdownServer: () => req('POST', '/api/system/shutdown'),  // 任务120：网页端停止服务
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
  statsQuality: (params = {}) => req('GET', '/api/stats/quality?' + new URLSearchParams(params)),
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
    // T-106/T-201：令牌走 Sec-WebSocket-Protocol 子协议（不进 URL/日志），
    // 服务端口径与 HTTP 读一致（auth_read_protected 开启才要求令牌）。
    const t = adminToken.get()
    const protos = t ? ['labsentinel', 'bearer.' + t] : undefined
    ws = new WebSocket((location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '/ws/events', protos)
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
