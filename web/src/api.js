import { ref } from 'vue'

const BASE = ''

// 管理令牌（任务16）：localStorage 持久化，启用鉴权的服务端要求写操作携带 X-Admin-Token
export const adminToken = {
  get: () => localStorage.getItem('admin_token') || '',
  set: (v) => v ? localStorage.setItem('admin_token', v) : localStorage.removeItem('admin_token'),
}

let browserSessionMode = false
export const setBrowserSessionMode = enabled => { browserSessionMode = enabled }
export const requestToken = () => sessionStorage.getItem('ls_readonly_token') || (browserSessionMode ? '' : adminToken.get())
export const accessChanged = () => window.dispatchEvent(new Event('ls-access-changed'))

export async function fetchMedia(path, signal, etag) {
  const token = requestToken()
  const headers = token ? { 'X-Admin-Token': token } : {}
  if (etag) headers['If-None-Match'] = etag
  const response = await fetch(path, { headers, cache: 'no-store', signal })
  if (!response.ok && !(etag && response.status === 304)) {
    if (response.status === 401 && browserSessionMode) accessChanged()
    let message = `下载失败（HTTP ${response.status}）`
    if (response.headers.get('content-type')?.includes('application/json')) {
      const body = await response.json().catch(() => null)
      if (typeof body?.detail === 'string') message = body.detail
    }
    throw Object.assign(new Error(message), { status: response.status })
  }
  return response
}

export async function fetchBinary(path) {
  return (await fetchMedia(path)).blob()
}

export async function downloadBinary(path, filename) {
  const url = URL.createObjectURL(await fetchBinary(path))
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

// 按身份与媒体路径缓存短期签名，签名就绪后更新引用它的视图。
const _sigCache = new Map()
const signatureVersion = ref(0)

// RA5-UX：Welcome 引导向导完成标记（localStorage；跳过/完成均写入不再弹出）
export const setupDone = {
  isDone: () => localStorage.getItem('ls-welcome-done') === '1',
  mark: () => localStorage.setItem('ls-welcome-done', '1'),
}
export function mediaUrl(path) {
  if (!path || typeof path !== 'string') return ''
  const t = requestToken()
  if (!t) return path
  void signatureVersion.value
  const base = path.split('?')[0]
  const key = `${t}\n${base}`
  const extra = path.includes('?') ? '&' + path.split('?').slice(1).join('&') : ''
  const hit = _sigCache.get(key)
  if (hit && Number(hit.exp) > Date.now() / 1000 + 5) {
    return base + (extra ? '?' + extra.slice(1) : '') + (extra ? '&' : '?') + 'exp=' + hit.exp + '&sig=' + hit.sig
  }
  // 异步批量签名（并发同路径只发一次）
  if (!hit || hit.exp) {
    _sigCache.set(key, { exp: 0, sig: '' })
    req('GET', '/api/media/sign?path=' + encodeURIComponent(base))
      .then((r) => {
        if (!Number.isFinite(Number(r?.exp)) || typeof r?.sig !== 'string') throw new Error('无效的媒体签名')
        _sigCache.set(key, { exp: r.exp, sig: r.sig })
        signatureVersion.value++
      })
      .catch(() => _sigCache.delete(key))
  }
  return ''
}

async function req(method, path, body, signal) {
  const isForm = body instanceof FormData
  const headers = body && !isForm ? { 'Content-Type': 'application/json' } : {}
  const token = requestToken()
  if (token) headers['X-Admin-Token'] = token
  const r = await fetch(BASE + path, {
    method,
    headers,
    body: body ? (isForm ? body : JSON.stringify(body)) : undefined,
    signal,
  })
  if (!r.ok) {
    let msg = `${method} ${path} -> ${r.status}`
    let code
    try {
      const error = await r.json()
      if (typeof error?.detail === 'string') msg = error.detail
      code = error?.code
    } catch {}
    if (code === 'password_required' || code === 'device_expired') accessChanged()
    else if (r.status === 401 && !path.startsWith('/api/access/')) {
      msg = '需要管理令牌。请在「设置 → 访问控制」填写 API 凭据后重试。'
    }
    throw Object.assign(new Error(msg), { status: r.status, code })
  }
  return r.status === 204 ? null : r.json()
}

export const api = {
  accessStatus: () => req('GET', '/api/access/status'),
  accessSettings: () => req('GET', '/api/access/settings'),
  saveAccess: (data) => req('PUT', '/api/access/settings', data),
  accessLogin: (password) => req('POST', '/api/access/login', { password }),
  accessLogout: () => req('POST', '/api/access/logout', {}),
  revokeOtherSessions: () => req('POST', '/api/access/revoke-others', {}),
  recoverAccess: (data) => req('POST', '/api/access/recover', data),
  mcpConfig: () => req('GET', '/api/mcp/config'),
  saveMcpConfig: (data) => req('PUT', '/api/mcp/config', data),
  mcpTokens: () => req('GET', '/api/mcp/tokens'),
  createMcpToken: (data) => req('POST', '/api/mcp/tokens', data),
  revokeMcpToken: (id) => req('DELETE', `/api/mcp/tokens/${encodeURIComponent(id)}`),
  system: () => req('GET', '/api/system'),
  cameras: () => req('GET', '/api/cameras'),
  cameraStatus: (id) => req('GET', `/api/cameras/${id}/status`),
  usbDevices: (refresh = false) => req('GET', '/api/cameras/usb-devices' + (refresh ? '?refresh=1' : '')),
  addCamera: (d) => req('POST', '/api/cameras', d),
  cameraGroups: () => req('GET', '/api/camera-groups'),
  setCameraGroup: (id, name) => req('PUT', `/api/cameras/${id}/group`, { group_name: name }),
  setAnalysisSource: (id, body) => req('PUT', `/api/cameras/${id}/analysis-source`, body),
  bulkCameras: (ids, action) => req('POST', '/api/cameras/bulk', { ids, action }),
  discoverCameras: (params = {}) => req('GET', '/api/discovery/cameras?' + new URLSearchParams(params)),
  probeHost: (d) => req('POST', '/api/discovery/probe', d),
  pluginsList: () => req('GET', '/api/plugins'),
  pluginAction: (id, action) => req('POST', `/api/plugins/${id}/${action}`),
  pluginLogs: (id, tail = 200) => req('GET', `/api/plugins/${id}/logs?tail=${tail}`),
  // RA5-UX：插件 zip 上传（multipart）+ 目录浏览 + Welcome 向导状态 + 模型切换
  pluginsUpload: (file) => req('POST', '/api/plugins/upload', file),
  fsList: (path) => req('GET', '/api/fs/list?' + new URLSearchParams({ path })),
  setupStatus: () => req('GET', '/api/setup/status'),
  setModelActive: (path) => req('PUT', '/api/models/active', { path }),
  modelsLocal: () => req('GET', '/api/models/local'),
  // UX2（2026-11-20）：检测模式全局策略（local/api/cascade）
  detectConfig: () => req('GET', '/api/detect/config'),
  localVlm: (probe = false) => req('GET', '/api/detect/local-vlm' + (probe ? '?probe=true' : '')),
  startLocalVlm: (payload) => req('POST', '/api/detect/local-vlm/start', payload),
  stopLocalVlm: () => req('POST', '/api/detect/local-vlm/stop', {}),
  saveDetectConfig: (d) => req('PUT', '/api/detect/config', d),
  detectTest: () => req('POST', '/api/detect/test', {}),
  detectUsage: () => req('GET', '/api/detect/usage'),
  logsTail: (limit = 200) => req('GET', '/api/logs?limit=' + limit),
  // LS-051：go2rtc 托管状态 / 开关
  go2rtcStatus: () => req('GET', '/api/go2rtc/status'),
  go2rtcConfig: (enabled) => req('PUT', '/api/go2rtc/config', { enabled }),
  // LS-053：日报/周报、局域网配置、开机自启、重启
  reportSchedule: () => req('GET', '/api/reports/schedule'),
  saveReportSchedule: (d) => req('PUT', '/api/reports/schedule', d),
  reportSendNow: () => req('POST', '/api/reports/send-now', {}),
  aiSessions: (offset = 0) => req('GET', '/api/assistant/sessions?' + new URLSearchParams({ offset })),
  aiOpenSession: (body) => req('POST', '/api/assistant/sessions', body),
  aiSession: (id, offset = 0) => req('GET', `/api/assistant/sessions/${id}?` + new URLSearchParams({ offset })),
  aiAsk: (id, body) => req('POST', `/api/assistant/sessions/${id}/messages`, body),
  aiReports: (archived = false, offset = 0) => req('GET', '/api/assistant/reports?' + new URLSearchParams({ archived, offset })),
  aiGenerateReport: (body) => req('POST', '/api/assistant/reports', body),
  aiArchiveReport: (id, archived) => req('PATCH', `/api/assistant/reports/${id}`, { archived }),
  aiExportReport: (id) => downloadBinary(`/api/assistant/reports/${id}/export.md`, `safety-report-${id}.md`),
  aiRoles: () => req('GET', '/api/assistant/roles'),
  aiSaveRoles: (body) => req('PUT', '/api/assistant/roles', body),
  aiTestEmbeddings: () => req('POST', '/api/assistant/roles/embeddings/test', {}),
  networkConfig: () => req('GET', '/api/system/network'),
  saveNetworkConfig: (d) => req('PUT', '/api/system/network', d),
  autostartStatus: () => req('GET', '/api/system/autostart'),
  setAutostart: (enabled) => req('PUT', '/api/system/autostart', { enabled }),
  restartServer: () => req('POST', '/api/system/restart', {}),
  // LS-054：数据飞轮样本池 / 诊断包 / 只读分享链接
  datasetSamples: (params = {}) => req('GET', '/api/dataset/samples?' + new URLSearchParams(params)),
  datasetFromEvent: (eid) => req('POST', `/api/dataset/samples/from-event/${eid}`, {}),
  datasetFromFolder: (iid) => req('POST', `/api/dataset/samples/from-folder/${iid}`, {}),
  datasetCapture: (cid) => req('POST', `/api/dataset/samples/capture/${cid}`, {}),
  datasetUpdate: (id, d) => req('PUT', `/api/dataset/samples/${id}`, d),
  datasetDelete: (id) => req('DELETE', `/api/dataset/samples/${id}`),
  datasetExportUrl: (params = {}) => {
    const query = new URLSearchParams(params).toString()
    return query ? '/api/dataset/export.zip?' + query : '/api/dataset/export.zip'
  },
  diagnosticsBundleUrl: () => '/api/diagnostics/bundle.zip',
  shareLinks: () => req('GET', '/api/share-links'),
  createShareLink: (d) => req('POST', '/api/share-links', d),
  deleteShareLink: (id) => req('DELETE', `/api/share-links/${id}`),
  verifyShareLink: (token) => req('GET', '/api/share-links/verify?token=' + encodeURIComponent(token)),
  eventsTimeline: (params = {}) => req('GET', '/api/events/timeline?' + new URLSearchParams(params)),
  setCameraAnalyzer: (id, analyzer) => req('PUT', `/api/cameras/${id}/analyzer`, { analyzer }),
  startCamera: (id) => req('POST', `/api/cameras/${id}/start`),
  delCamera: (id) => req('DELETE', `/api/cameras/${id}`),
  // 任务121：图片文件夹自动识别
  folderImagesSummary: (params = {}) => req('GET', '/api/folder-images/summary?' + new URLSearchParams(params)),
  folderImages: (params = {}) => req('GET', '/api/folder-images?' + new URLSearchParams(params)),
  folderImage: (id) => req('GET', `/api/folder-images/${id}`),
  folderScan: (id) => req('POST', `/api/folders/${id}/scan`),
  zones: (cid) => req('GET', `/api/zones?camera_id=${cid}`),
  addZone: (d) => req('POST', '/api/zones', d),
  updateZone: (id, polygon) => req('PUT', `/api/zones/${id}`, { polygon }),
  updateZoneSpatial: (id, settings) => req('PUT', `/api/zones/${id}`, settings),
  spatialSettings: (cid) => req('GET', `/api/cameras/${cid}/spatial`),
  cameraLabRules: (cid) => req('GET', `/api/cameras/${cid}/lab-rules`),
  scenePacks: () => req('GET', '/api/scene-packs'),
  previewScenePack: (choice) => req('POST', '/api/scene-packs/preview', choice),
  applyScenePack: (choice) => req('POST', '/api/scene-packs/apply', choice),
  saveCameraLabRules: (cid, data) => req('PUT', `/api/cameras/${cid}/lab-rules`, data),
  addCameraMask: (cid, data) => req('POST', `/api/cameras/${cid}/masks`, data),
  deleteCameraMask: (cid, id) => req('DELETE', `/api/cameras/${cid}/masks/${id}`),
  rememberEventSpot: (id) => req('POST', `/api/events/${id}/remember-spot`),
  eventOverlay: (id) => req('GET', `/api/events/${id}/overlay`),
  eventExplanation: (id) => req('GET', `/api/events/${id}/explain`),
  suppressions: (filters) => req('GET', '/api/review/suppressions?' + new URLSearchParams(filters)),
  saveSuppressionSample: (id) => req('POST', `/api/review/suppressions/${id}/sample`),
  suppressionSnapshot: async (id) => {
    const token = requestToken()
    const response = await fetch(`/api/review/suppressions/${id}/snapshot`, {
      credentials: 'same-origin',
      headers: token ? { 'X-Admin-Token': token } : {},
    })
    if (!response.ok) throw new Error('截图暂时不可用')
    return response.blob()
  },
  deleteFpSpot: (cid, id) => req('DELETE', `/api/cameras/${cid}/fp-spots/${id}`),
  delZone: (id) => req('DELETE', `/api/zones/${id}`),
  copyConfig: (cid, payload) => req('POST', `/api/cameras/${cid}/copy-config`, payload),
  rules: (cid) => req('GET', `/api/rules?camera_id=${cid}`),
  addRule: (d) => req('POST', '/api/rules', d),
  updateRuleGrace: (id, data) => req('PUT', `/api/rules/${id}`, data),
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
  eventNotes: (id, signal) => req('GET', `/api/events/${id}/notes`, undefined, signal),
  addEventNote: (id, text) => req('POST', `/api/events/${id}/notes`, { text }),
  alertsStatus: () => req('GET', '/api/alerts/status'),
  alertConfig: () => req('GET', '/api/alerts/config'),
  alertActuator: () => req('GET', '/api/alerts/actuator'),
  saveAlertActuator: (data) => req('PUT', '/api/alerts/actuator', data),
  saveAlertConfig: (data) => req('PUT', '/api/alerts/config', data),
  resetAlertConfig: (channel) => req('POST', '/api/alerts/config/reset' + (channel ? `?channel=${encodeURIComponent(channel)}` : '')),
  pushPolicy: () => req('GET', '/api/alerts/push-policy'),
  savePushPolicy: (d) => req('PUT', '/api/alerts/push-policy', d),
  alertTest: (channel) => req('POST', '/api/alert-test' + (channel ? `?channel=${encodeURIComponent(channel)}` : '')),
  alertDeliveries: (params = {}, signal) => req('GET', '/api/alert-deliveries?' + new URLSearchParams(params), undefined, signal),
  alertDeliveryRetry: (id) => req('POST', `/api/alert-deliveries/${id}/retry`),
  alertSilences: (active = 1) => req('GET', '/api/alert-silences' + (active ? '?active=1' : '')),
  addAlertSilence: (d) => req('POST', '/api/alert-silences', d),
  deleteAlertSilence: (id) => req('DELETE', `/api/alert-silences/${id}`),
  alertTemplate: () => req('GET', '/api/alert-template'),
  saveAlertTemplate: (d) => req('PUT', '/api/alert-template', d),
  audit: (params = {}) => req('GET', '/api/audit?' + new URLSearchParams(params)),
  alertTemplatePreview: (d) => req('POST', '/api/alert-template/preview', d),
  // Q-08 / T-406：实验服模型按需下载（许可确认 + SHA256 校验）；0.3.1 起随包
  ppeStatus: () => req('GET', '/api/models/ppe/status'),
  ppeDownload: (confirm_license) => req('POST', '/api/models/ppe/download', { confirm_license }),
  // 0.4.0：实验服模型启停/替换
  // 0.5.0 LS-050：接受 {enabled?, violation_conf?, wearing_conf?, clahe_enabled?}
  ppeConfig: (patch) => req('PUT', '/api/models/ppe/config', patch),
  ppeUpload: (file) => {
    const fd = new FormData()
    fd.append('file', file)
    return req('POST', '/api/models/ppe/upload', fd)
  },
  ppeReset: () => req('POST', '/api/models/ppe/reset', {}),
  // LS-055：模型 A/B 对比评测
  ppeBenchmark: (candidate_path, activate = false, candidate_format = 'auto') =>
    req('POST', '/api/models/ppe/benchmark', { candidate_path, activate, candidate_format }),
  // 0.4.0：API 多接入档案（保存多个上游，随时切换/测试/自动拉模型）
  detectProfiles: () => req('GET', '/api/detect/profiles'),
  saveDetectProfile: (p) => req('POST', '/api/detect/profiles', p),
  activateDetectProfile: (id) => req('POST', `/api/detect/profiles/${id}/activate`, {}),
  deleteDetectProfile: (id) => req('DELETE', `/api/detect/profiles/${id}`),
  detectModels: (p) => req('POST', '/api/detect/models', p),
  // R2-203：设备配对与三级令牌
  createPairing: (data) => req('POST', '/api/pairing', data),
  claimPairing: (data) => req('POST', '/api/pairing/claim', data),
  cancelPairing: (code) => req('POST', '/api/pairing/revoke', { code }),
  deviceLogout: () => req('POST', '/api/pairing/logout', {}),
  listDevices: () => req('GET', '/api/devices'),
  editDevice: (id, data) => req('PUT', `/api/devices/${id}`, data),
  revokeDevice: (id) => req('DELETE', `/api/devices/${id}`),
  setEventStatus: (id, status) => req('POST', `/api/events/${id}/status`, { status }),
  bulkStatus: (payload) => req('POST', '/api/events/bulk-status', payload),
  cameraWebrtc: (id, signal) => req('GET', `/api/cameras/${id}/webrtc`, undefined, signal),
  maintenanceStatus: () => req('GET', '/api/maintenance/status'),
  semanticStatus: () => req('GET', '/api/events/explore/settings'),
  semanticConfigure: enabled => req('PUT', '/api/events/explore/settings', { enabled }),
  semanticSearch: params => req('GET', '/api/events/explore?' + new URLSearchParams(params)),
  storageUsage: () => req('GET', '/api/storage'),
  storagePreview: selection => req('POST', '/api/storage/preview', selection),
  storageCleanup: request => req('POST', '/api/storage/cleanup', request),
  storageCompact: () => req('POST', '/api/storage/compact', {}),
  storagePolicy: rules => req('PUT', '/api/storage/policy', rules),
  maintenancePreview: () => req('GET', '/api/maintenance/preview'),
  shutdownServer: () => req('POST', '/api/system/shutdown'),  // 任务120：网页端停止服务
  // 任务79：配置导入导出（导出走 fetch+blob，令牌经请求头传递；<a> 裸链接带不了）
  async configExportDownload() {
    const headers = {}
    const token = requestToken()
    if (token) headers['X-Admin-Token'] = token
    const r = await fetch('/api/config/export', { headers })
    if (!r.ok) {
      let msg = `GET /api/config/export -> ${r.status}`
      try { msg = (await r.json()).detail || msg } catch {}
      if (r.status === 401) msg = '需要管理令牌：请在「设置 → 管理令牌」填写管理令牌后重试。' + msg
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
  faceConfig: () => req('GET', '/api/face/config'),
  saveFaceConfig: (patch) => req('PUT', '/api/face/config', patch),
  installFaceLocal: () => req('POST', '/api/face/models/local/install', {}),
  testFaceApi: (id) => req('POST', `/api/face/models/${id}/test`, {}),
  setFaceCamera: (id, permission, alert_on_match) => req('PUT', `/api/face/cameras/${id}`, { permission, alert_on_match }),
  recognizeFace: (form) => req('POST', '/api/face/recognize', form),
  faceLibrary: () => req('GET', '/api/face/library'),
  removeFacePerson: (id) => req('DELETE', `/api/face/library/${id}`),
  faceEventMatch: (id, signal) => req('GET', `/api/face/events/${id}`, undefined, signal),
  facePersons: () => req('GET', '/api/face/persons'),
  faceRegister: (formData) => req('POST', '/api/face/persons', formData),
  faceDeletePerson: (subject) => req('DELETE', `/api/face/persons/${encodeURIComponent(subject)}`),
  faceClearAll: () => req('DELETE', '/api/face/persons/all'),
  statsDaily: (params = {}) => req('GET', '/api/stats/daily?' + new URLSearchParams(params)),
  statsCompliance: (params = {}) => req('GET', '/api/stats/compliance?' + new URLSearchParams(params)),
  statsWeekly: (params = {}) => req('GET', '/api/stats/weekly?' + new URLSearchParams(params)),
  statsSummary: (params = {}) => req('GET', '/api/stats/summary?' + new URLSearchParams(params)),
  statsQuality: (params = {}) => req('GET', '/api/stats/quality?' + new URLSearchParams(params)),
  statsHourly: (params = {}) => req('GET', '/api/stats/hourly?' + new URLSearchParams(params)),
  statsHotspots: (params = {}) => req('GET', '/api/stats/hotspots?' + new URLSearchParams(params)),
}

export const TYPE_TEXT = {
  motion_recording: '运动触发录像',
  alert_resolved: '违规已恢复',
  ppe_policy: '区域穿戴不合规',
  hood_lean: '通风橱探身',
  sash_limit: '通风橱拉门过高',
  person_fall: '疑似跌倒',
  thermal_overheat: '温度传感器超限',
  incubator_door_open: '培养箱门持续开启',
  spill_or_leak: '泼洒或泄漏',
  glove_contamination: '手套疑似交叉接触',
  photo_check: '图片检查',
  video_check: '视频检查',
  face_match: '识别到已录入人员',
  enter_no_ppe: '进入安全区未穿实验服',
  enter: '进入安全区',
  exit: '离开安全区',
  occupancy_over: '在场人数超限',
  stranger: '陌生人进入', // 人脸模块（默认关闭）产生；与服务端 constants.py 同步
  solo_after_hours: '单人留守超时', // LS-050
  no_gloves: '未戴手套',
  no_goggles: '未戴护目镜',
  no_mask: '未戴口罩',
  camera_offline: '摄像头断流',
  camera_recovered: '摄像头恢复',
  camera_tamper: '摄像头画面异常',
}

export const CHANNEL_NAMES = {
  wecom: '企业微信', dingtalk: '钉钉', feishu: '飞书', serverchan: 'Server酱·微信',
  pushplus: 'PushPlus·微信', qmsg: 'Qmsg·QQ', email: '邮件', webhook: 'Webhook',
}

export const STATUS_TEXT = {
  recorded: '已记录',
  new: '未处置',
  acked: '已确认',
  false_positive: '误报',
  ignored: '忽略',
}

export function connectEvents(onEvent, onState, onUpdate, { since = null, onHistoryReset } = {}) {
  let ws, timer, closed = false
  let cursor = Number.isSafeInteger(since) && since >= 0 ? since : null
  function open() {
    if (closed) return
    // T-106/T-201：令牌走 Sec-WebSocket-Protocol 子协议（不进 URL/日志），
    // 服务端口径与 HTTP 读一致（auth_read_protected 开启才要求令牌）。
    const t = requestToken()
    const protos = t ? ['labsentinel', 'bearer.' + t] : undefined
    const url = new URL('/ws/events', location.href)
    url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
    url.searchParams.set('since', cursor ?? 'latest')
    ws = new WebSocket(url.toString(), protos)
    ws.onopen = () => onState && onState('ok')
    ws.onmessage = (m) => {
      if (closed) return
      try {
        const event = JSON.parse(m.data)
        if (Number.isSafeInteger(event?.stream_cursor)) {
          cursor = event.stream_cursor
          if (event.stream_history_reset === true) onHistoryReset?.()
        } else if ((event?.op === 'episode_update' || event?.op === 'recording_update') && Number.isSafeInteger(event.id)) {
          onUpdate?.(event)
        } else if (Number.isSafeInteger(event?.id) && (cursor === null || event.id > cursor)) {
          cursor = event.id
          onEvent(event)
        }
      } catch { /* Malformed event payload. */ }
    }
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
