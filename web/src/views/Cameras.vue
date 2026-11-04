<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import { api, adminToken, mediaUrl } from '../api'
import WebrtcPlayer from '../components/WebrtcPlayer.vue'
import AsyncState from '../components/AsyncState.vue'

const cams = ref([])
const form = ref({ name: '', source: '', type: 'file', autostart: false, group_name: '' })
// 任务111：分组与批量启停
const groupFilter = ref('')
const checked = ref([])
const bulkBusy = ref(false)
const groups = ref([])
const TYPE_LABEL = { rtsp: 'RTSP', file: '视频文件', usb: 'USB', folder: '图片文件夹' }
function typeLabel(t) { return TYPE_LABEL[t] || t }
// 任务66：三态 + 按钮防重复
const loading = ref(true)
const loaded = ref(false)
const stale = ref(false)      // 2026-10-30 审阅：有旧列表时的刷新失败
const camsError = ref('')
const busyCam = ref(null)      // start/del 进行中的摄像头 id
const adding = ref(false)
// 任务60：USB 设备枚举（索引=DirectShow 顺序仅本机参考，设备名更稳定）
const usbDevices = ref([])
const usbError = ref('')
const usbLoading = ref(false)

async function loadUsb(refresh = false) {
  usbLoading.value = true
  try {
    const r = await api.usbDevices(refresh)
    usbDevices.value = r.devices || []
    usbError.value = r.error || ''
  } catch {
    usbDevices.value = []
    usbError.value = '设备枚举接口不可用，可手动输入索引'
  }
  usbLoading.value = false
}
watch(() => form.value.type, t => { if (t === 'usb') loadUsb() })
const snap = ref({})  // id -> timestamp (用于强制刷新 img)
const live = ref(null)   // { id, name, wsUrl } 正在观看实时画面的摄像头
const liveErr = ref('')
const tokenInput = ref(adminToken.get())
const tokenMsg = ref('')
// 任务23：告警渠道状态与自测
const CHANNEL_NAMES = {
  wecom: '企业微信', dingtalk: '钉钉', feishu: '飞书', serverchan: 'Server酱·微信',
  pushplus: 'PushPlus·微信', qmsg: 'Qmsg·QQ', email: '邮件',
}
const alertChannels = ref([])
const alertTesting = ref(false)
const alertTestMsg = ref('')
const failedDeliveries = ref([])

async function loadAlerts() {
  try {
    alertChannels.value = (await api.alertsStatus()).channels
    failedDeliveries.value = await api.alertDeliveries({ status: 'failed', limit: 10 })
    silences.value = await api.alertSilences()  // 任务81
  } catch { /* 服务未就绪时静默 */ }
}

// 任务81：告警静默
const silences = ref([])
const silenceMsg = ref('')
const silenceScope = ref('global')
const silenceTarget = ref('')
const silenceDuration = ref(3600)
const silenceReason = ref('')

async function addSilence(scope, target = silenceTarget.value, duration_s = silenceDuration.value) {
  silenceMsg.value = ''
  try {
    await api.addAlertSilence({ scope, target: target || null, duration_s,
                                reason: silenceReason.value || null })
    silenceReason.value = ''
    loadAlerts()
  } catch (e) { silenceMsg.value = String(e.message || e) }
}

async function removeSilence(id) {
  try { await api.deleteAlertSilence(id); loadAlerts() }
  catch (e) { silenceMsg.value = String(e.message || e) }
}

async function runAlertTest() {
  alertTesting.value = true
  alertTestMsg.value = ''
  try {
    const results = await api.alertTest()
    const okN = results.filter(r => r.ok).length
    alertTestMsg.value = `测试完成：${okN}/${results.length} 渠道成功` +
      results.filter(r => !r.ok).map(r => `（${CHANNEL_NAMES[r.channel] || r.channel}：${r.error || '失败'}）`).join('')
    loadAlerts()
  } catch (e) { alertTestMsg.value = String(e.message || e) }
  alertTesting.value = false
}

async function retryDelivery(d) {
  try { await api.alertDeliveryRetry(d.id); loadAlerts() }
  catch (e) { alert(String(e.message || e)) }
}

function saveToken() {
  adminToken.set(tokenInput.value.trim())
  tokenMsg.value = adminToken.get() ? '令牌已保存（仅存本浏览器）' : '令牌已清除'
  refresh()
}

async function refresh() {
  try {
    cams.value = await api.cameras()
    camsError.value = ''
    stale.value = false
    loaded.value = true
  } catch (e) {
    // 2026-10-30 审阅：有旧列表时只出提示（AsyncState error 分支会隐藏内容，提示会变假话）
    if (loaded.value) stale.value = true
    else camsError.value = String(e.message || e)
  } finally {
    loading.value = false
  }
  try {
    sys.value = await api.system()
  } catch { /* 就绪前静默 */ }
  try { groups.value = (await api.cameraGroups()).groups } catch { /* 旧服务端无此端点 */ }
}

// 任务111：分组筛选 + 批量启停
const shownCams = computed(() =>
  groupFilter.value ? cams.value.filter(c => (c.group_name || '') === groupFilter.value) : cams.value)
function groupLabel(c) { return c.group_name || '未分组' }
async function bulk(action) {
  const ids = [...checked.value]
  if (!ids.length || bulkBusy.value) return
  const word = action === 'start' ? '启动' : '停止'
  if (!confirm(`确定${word}选中的 ${ids.length} 路摄像头？`)) return
  bulkBusy.value = true
  try {
    const r = await api.bulkCameras(ids, action)
    if (r.missing.length) alert(`不存在的摄像头: ${r.missing.join(',')}`)
    if (r.failed.length) alert(`失败: ${r.failed.map(f => `#${f.id} ${f.error}`).join('；')}`)
    checked.value = []
    await refresh()
  } catch (e) { alert(String(e.message || e)) }
  bulkBusy.value = false
}
async function groupAction(name, action) {
  const ids = cams.value.filter(c => (c.group_name || '') === name).map(c => c.id)
  if (!ids.length || bulkBusy.value) return
  const word = action === 'start' ? '启动' : '停止'
  if (!confirm(`确定${word}「${name || '未分组'}」的 ${ids.length} 路？`)) return
  bulkBusy.value = true
  try { await api.bulkCameras(ids, action); await refresh() }
  catch (e) { alert(String(e.message || e)) }
  bulkBusy.value = false
}

// 任务65：DB 行 + manager 运行指标合并（按 id）
const sys = ref({ cameras: [] })
function viewFor(c) {
  const m = sys.value.cameras.find(x => x.id === c.id) || {}
  return { ...c, ...m }
}
onMounted(() => { refresh(); loadAlerts() })

// 任务113：局域网发现助手（只读探测；发现结果仅作填表参考，不自动添加）
const discover = ref({ busy: false, devices: [], error: '', lan: [], shown: false })
const BRANDS = [
  { key: 'hik', label: '海康威视', tpl: 'rtsp://用户名:密码@{ip}:554/Streaming/Channels/102' },
  { key: 'dahua', label: '大华', tpl: 'rtsp://用户名:密码@{ip}:554/cam/realmonitor?channel=1&subtype=1' },
  { key: 'uniview', label: '宇视', tpl: 'rtsp://用户名:密码@{ip}:554/media/video1' },
  { key: 'generic', label: '通用 ONVIF', tpl: 'rtsp://用户名:密码@{ip}:554/' },
]
const brandKey = ref('hik')
const probeRow = ref(null)   // { ip, result }
const probeBusyIp = ref('')

async function discoverCams(refresh) {
  discover.value.busy = true
  discover.value.shown = true
  try {
    const r = await api.discoverCameras(refresh ? { refresh: 1 } : {})
    discover.value.devices = r.devices
    discover.value.error = r.error
    discover.value.lan = r.lan || []
  } catch (e) { discover.value.error = String(e.message || e) }
  discover.value.busy = false
}
function fillFromDevice(d) {
  const tpl = (BRANDS.find(b => b.key === brandKey.value) || BRANDS[0]).tpl
  form.value.source = tpl.replace('{ip}', d.ip)
  if (!form.value.name) form.value.name = d.name || `摄像头-${d.ip}`
}
async function probeIp(ip) {
  if (probeBusyIp.value) return
  probeBusyIp.value = ip
  try {
    probeRow.value = { ip, result: await api.probeHost({ host: ip, port: 554 }) }
  } catch (e) { probeRow.value = { ip, result: { error: String(e.message || e) } } }
  probeBusyIp.value = ''
}

// 任务79：配置导入导出
const cfgBusy = ref(false)
const cfgMsg = ref('')
const importMode = ref('merge')
const confirmReplace = ref(false)
const importFile = ref(null)

async function exportConfig() {
  cfgBusy.value = true
  cfgMsg.value = ''
  try {
    const data = await api.configExportDownload()
    cfgMsg.value = `已导出 ${data.cameras.length} 摄像头 / ${data.zones.length} 区域 / ` +
      `${data.rules.length} 规则（已下载到浏览器下载目录）`
  } catch (e) { cfgMsg.value = String(e.message || e) }
  cfgBusy.value = false
}

function onImportFile(ev) {
  importFile.value = ev.target.files?.[0] || null
  cfgMsg.value = ''
}

async function importConfig() {
  if (!importFile.value || cfgBusy.value) return
  if (importMode.value === 'replace' && !confirmReplace.value) {
    cfgMsg.value = 'replace 模式需先勾选确认（将清空现有摄像头/区域/规则）'
    return
  }
  cfgBusy.value = true
  cfgMsg.value = '解析中…'
  try {
    const text = await importFile.value.text()
    let data
    try { data = JSON.parse(text) } catch { throw new Error('不是合法 JSON 文件') }
    cfgMsg.value = '导入中…'
    const r = await api.configImport({ mode: importMode.value,
                                       confirm_replace: confirmReplace.value, data })
    cfgMsg.value = `导入完成：新建 ${r.created.cameras} 摄像头 / ${r.created.zones} 区域 / ` +
      `${r.created.rules} 规则；跳过 ${r.skipped.cameras.length}/${r.skipped.zones.length}/` +
      `${r.skipped.rules.length}` + (r.warnings.length ? `\n${r.warnings.join('\n')}` : '')
    refresh()
  } catch (e) { cfgMsg.value = String(e.message || e) }
  cfgBusy.value = false
  importFile.value = null
}

// 任务85：告警文案模板（只读展示 + 预览）
const tpl = ref({ text: '', title: '', placeholders: {} })
const tplPreview = ref('')
const tplBusy = ref(false)
const tplSample = ref({ level: 3, type: 'enter_no_ppe', camera_id: 1, track_id: 7,
                        detail: '示例：有人未穿实验服进入试剂区' })

async function loadTemplate() {
  tplBusy.value = true
  try {
    tpl.value = await api.alertTemplate()
    tplPreview.value = ''
  } catch (e) { tplPreview.value = String(e.message || e) }
  tplBusy.value = false
}

async function previewTemplate() {
  tplBusy.value = true
  try {
    const r = await api.alertTemplatePreview(tplSample.value)
    tplPreview.value = `标题：${r.title}\n正文：${r.text}`
  } catch (e) { tplPreview.value = String(e.message || e) }
  tplBusy.value = false
}

// 任务120：网页端停止服务（响应返回后服务进程 ~1s 内退出）
async function shutdownServer() {
  if (!confirm('确定停止 LabSentinel 服务？\n停止后面板将断开，重新启动程序后才能再次访问。')) return
  try {
    await api.shutdownServer()
  } catch { /* 停止瞬间连接中断属预期，忽略 */ }
  alert('停止指令已发出。服务退出后本页面将无法刷新；可关闭浏览器窗口。')
}

async function add() {
  if (!form.value.name || !form.value.source || adding.value) return
  adding.value = true
  try {
    await api.addCamera(form.value)
    form.value = { name: '', source: '', type: 'file', autostart: false }
    refresh()
  } catch (e) { alert(String(e.message || e)) }
  adding.value = false
}
async function start(id) {
  if (busyCam.value !== null) return
  busyCam.value = id
  try { await api.startCamera(id); refresh() } catch (e) { alert(String(e.message || e)) }
  busyCam.value = null
}
async function scanFolder(id) {
  if (busyCam.value !== null) return
  busyCam.value = id
  try { await api.folderScan(id); refresh() } catch (e) { alert(String(e.message || e)) }
  busyCam.value = null
}
async function del(id) {
  if (busyCam.value !== null) return
  if (!confirm(`确认删除摄像头 #${id}？（其区域与规则会一并删除）`)) return
  busyCam.value = id
  try { await api.delCamera(id); refresh() } catch (e) { alert(String(e.message || e)) }
  busyCam.value = null
}

async function watchLive(cam) {
  liveErr.value = ''
  try {
    const r = await api.cameraWebrtc(cam.id)
    live.value = { id: cam.id, name: cam.name, wsUrl: r.ws_url }
  } catch {
    liveErr.value = '获取实时画面地址失败'
  }
}
</script>

<template>
  <div class="panel">
    <h3 style="margin-bottom:10px">管理令牌</h3>
    <div class="row">
      <input v-model="tokenInput" placeholder="服务端启用鉴权时填写（LS_ADMIN_TOKEN）" aria-label="管理令牌" style="flex:1" />
      <button @click="saveToken">保存令牌</button>
      <span v-if="tokenMsg" class="muted">{{ tokenMsg }}</span>
    </div>
    <p class="muted" style="margin-top:4px">令牌仅保存在本浏览器 localStorage；服务端未启用鉴权（LS_ADMIN_TOKEN 为空）时无需填写。</p>

    <h3 style="margin:14px 0 10px">告警渠道状态</h3>
    <table>
      <tr><th>渠道</th><th>配置状态</th><th>投递 ok</th><th>failed</th><th>pending</th></tr>
      <tr v-for="ch in alertChannels" :key="ch.name">
        <td>{{ CHANNEL_NAMES[ch.name] || ch.name }}</td>
        <td>
          <span :class="'lv' + (ch.configured ? 1 : 2)">{{ ch.configured ? '已配置 ✓' : '未配置' }}</span>
        </td>
        <td>{{ ch.deliveries.ok }}</td>
        <td :style="ch.deliveries.failed ? 'color:var(--l3); font-weight:600' : ''">{{ ch.deliveries.failed }}</td>
        <td>{{ ch.deliveries.pending }}</td>
      </tr>
    </table>
    <div class="row" style="margin-top:8px">
      <button :disabled="alertTesting" @click="runAlertTest">{{ alertTesting ? '测试中…' : '发送测试告警' }}</button>
      <span v-if="alertTestMsg" class="muted">{{ alertTestMsg }}</span>
    </div>
    <p class="muted" style="margin-top:2px">渠道申请与配置方法见仓库 docs/部署指南.md「4. 告警渠道配置」。</p>

    <h3 style="margin:14px 0 10px">失败投递（最近 10 条）</h3>
    <table v-if="failedDeliveries.length">
      <tr><th>ID</th><th>事件</th><th>渠道</th><th>尝试次数</th><th>最近错误</th><th></th></tr>
      <tr v-for="d in failedDeliveries" :key="d.id">
        <td>{{ d.id }}</td>
        <td>#{{ d.event_id }}</td>
        <td>{{ CHANNEL_NAMES[d.channel] || d.channel }}</td>
        <td>{{ d.attempts }}</td>
        <td class="muted" style="max-width:280px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap">{{ d.last_error || '-' }}</td>
        <td><button class="ghost" @click="retryDelivery(d)">重发</button></td>
      </tr>
    </table>
    <p v-else class="muted">无失败投递 ✓</p>

    <h3 style="margin:14px 0 10px">告警静默（维护窗口）</h3>
    <div class="row" style="align-items:center">
      <button class="ghost" :disabled="cfgBusy" @click="addSilence('global', null, 3600)">
        全局静默 1h</button>
      <select v-model="silenceTarget" aria-label="摄像头" style="max-width:160px">
        <option value="">选摄像头…</option>
        <option v-for="c in cams" :key="c.id" :value="String(c.id)">{{ c.name }}</option>
      </select>
      <button class="ghost" :disabled="cfgBusy || !silenceTarget"
              @click="addSilence('camera', silenceTarget, 3600)">本摄像头静默 1h</button>
      <button class="ghost" :disabled="cfgBusy || !silenceTarget"
              @click="addSilence('camera', silenceTarget, 14400)">4h</button>
      <button class="ghost" :disabled="cfgBusy || !silenceTarget"
              @click="addSilence('camera', silenceTarget, 86400)">24h</button>
    </div>
    <div class="row" style="margin-top:6px; align-items:center">
      <select v-model="silenceScope" aria-label="静默范围">
        <option value="global">全局</option>
        <option value="camera">摄像头（填 ID）</option>
        <option value="level">级别（1/2/3）</option>
        <option value="type">类型</option>
      </select>
      <input v-model="silenceTarget" placeholder="target（ID/级别/类型）" style="width:140px" />
      <select v-model.number="silenceDuration" aria-label="时长">
        <option :value="3600">1 小时</option><option :value="14400">4 小时</option>
        <option :value="86400">24 小时</option><option :value="604800">7 天</option>
      </select>
      <input v-model="silenceReason" placeholder="原因（可选）" style="width:160px" />
      <button class="ghost" @click="addSilence()">添加</button>
    </div>
    <table v-if="silences.length" style="margin-top:8px">
      <tr><th>ID</th><th>范围</th><th>目标</th><th>剩余</th><th>原因</th><th></th></tr>
      <tr v-for="s in silences" :key="s.id">
        <td>#{{ s.id }}</td><td>{{ s.scope }}</td><td>{{ s.target ?? '—' }}</td>
        <td class="muted">{{ Math.ceil(s.remaining_s / 60) }} 分钟</td>
        <td class="muted">{{ s.reason || '—' }}</td>
        <td><button class="ghost" @click="removeSilence(s.id)">解除</button></td>
      </tr>
    </table>
    <p v-else class="muted">无静默（事件照常推送）</p>
    <p v-if="silenceMsg" class="muted" style="margin-top:4px">{{ silenceMsg }}</p>

    <h3 style="margin:14px 0 10px">配置导入导出（任务79）</h3>
    <div class="row" style="align-items:center">
      <button class="ghost" :disabled="cfgBusy" @click="exportConfig">
        {{ cfgBusy ? '导出中…' : '导出配置 JSON' }}</button>
      <input type="file" accept=".json,application/json" style="max-width:220px"
             aria-label="选择配置 JSON 文件" @change="onImportFile" />
      <select v-model="importMode" aria-label="导入模式">
        <option value="merge">合并（跳过同名）</option>
        <option value="replace">替换（清空现有配置）</option>
      </select>
      <label v-if="importMode === 'replace'" class="muted">
        <input type="checkbox" v-model="confirmReplace" style="margin:0 4px 0 0" />
        确认清空现有摄像头/区域/规则（事件保留但失去关联）
      </label>
      <button :disabled="cfgBusy || !importFile" @click="importConfig">
        {{ cfgBusy ? '导入中…' : '导入' }}</button>
    </div>
    <p v-if="cfgMsg" class="muted" style="margin-top:6px; white-space:pre-wrap">{{ cfgMsg }}</p>
    <p class="muted" style="margin-top:4px">
      导出默认脱敏 RTSP 密码（***:***@）；含密码版本需另行处理。导入不自动启动摄像头。</p>

    <h3 style="margin:14px 0 10px">告警文案模板（任务85）</h3>
    <div class="row" style="align-items:center">
      <button class="ghost" :disabled="tplBusy" @click="loadTemplate">
        {{ tplBusy ? '加载中…' : '加载当前模板' }}</button>
      <button class="ghost" :disabled="tplBusy || !tpl.text" @click="previewTemplate">预览</button>
      <span class="muted">修改模板请改 .env/config.env 的 LS_ALERT_TEMPLATE_*，重启生效</span>
    </div>
    <textarea v-if="tpl.text" readonly class="muted"
              style="width:100%; height:80px; margin-top:6px; font-size:12px">{{ tpl.text }}</textarea>
    <p v-if="tpl.text" class="muted" style="font-size:12px">
      占位符：{{ Object.keys(tpl.placeholders || {}).join(' / ') }}</p>
    <div v-if="tplPreview" class="muted" style="white-space:pre-wrap; border:1px solid var(--divider); border-radius:6px; padding:8px; margin-top:6px">
      {{ tplPreview }}</div>

    <h3 style="margin:14px 0 10px">服务控制（任务120）</h3>
    <div class="row" style="align-items:center">
      <button class="ghost" @click="shutdownServer">停止服务…</button>
      <span class="muted">停止后摄像头分析、事件推送全部暂停；重新启动程序（快捷方式/启动.bat）恢复。</span>
    </div>

    <h3 style="margin:14px 0 10px">添加摄像头</h3>
    <div class="row">
      <input v-model="form.name" placeholder="名称" aria-label="摄像头名称" style="width:120px" />
      <template v-if="form.type === 'usb'">
        <select v-if="usbDevices.length" v-model="form.source" style="flex:1" aria-label="USB 设备选择">
          <option v-for="d in usbDevices" :key="d.index" :value="d.source">{{ d.name }}（索引 {{ d.index }}）</option>
        </select>
        <input v-else v-model="form.source" placeholder="USB 设备索引，如 0 / 1" aria-label="USB 设备索引" style="flex:1" />
        <button class="ghost" :disabled="usbLoading" @click="loadUsb(true)">{{ usbLoading ? '刷新中…' : '刷新设备' }}</button>
      </template>
      <input v-else v-model="form.source"
             :placeholder="form.type === 'folder' ? '截图文件夹路径，如 D:\\screenshots（支持中文/网络盘）' : 'RTSP 地址 / 视频文件路径 / 0 (USB)'"
             :aria-label="form.type === 'folder' ? '图片文件夹路径' : '摄像头地址'" style="flex:1" />
      <select v-model="form.type">
        <option value="file">视频文件</option>
        <option value="rtsp">RTSP</option>
        <option value="usb">USB</option>
        <option value="folder">图片文件夹</option>
      </select>
      <label class="muted"><input type="checkbox" v-model="form.autostart" style="margin:0 4px 0 0" />立即启动</label>
      <input v-model="form.group_name" placeholder="分组（可选，≤32字）" aria-label="摄像头分组" style="width:140px" />
      <button :disabled="adding" @click="add">{{ adding ? '添加中…' : '添加' }}</button>
    </div>
    <p v-if="form.type === 'usb' && usbError" class="muted" style="margin-top:6px">{{ usbError }}</p>
    <p v-if="form.type === 'folder'" class="muted" style="margin-top:6px">
      图片文件夹接入：自动轮询该目录（含子目录）的 jpg/png/bmp/webp/tif，识别人员与实验服；
      “有人/违规”的图片会进入「图片识别」页汇报。原始截图只读，不移动、不删除。
      建议先到「安全区配置」画好区域与规则再启动。</p>
    <p class="muted" style="margin-top:6px">提示：不勾"立即启动"可先到"安全区配置"画好区域、加好规则，再回来启动。</p>

    <!-- 任务113：局域网发现助手（只读，不自动添加/启动） -->
    <h3 style="margin:14px 0 10px">发现摄像头（局域网）</h3>
    <div class="row">
      <select v-model="brandKey" aria-label="品牌模板">
        <option v-for="b in BRANDS" :key="b.key" :value="b.key">{{ b.label }}</option>
      </select>
      <button class="ghost" :disabled="discover.busy" @click="discoverCams(false)">
        {{ discover.busy ? '搜索中…' : '搜索局域网摄像头' }}</button>
      <button class="ghost" :disabled="discover.busy" @click="discoverCams(true)">强制刷新</button>
    </div>
    <p v-if="discover.shown && discover.lan.length" class="muted" style="margin:6px 0 0">
      本机局域网地址：{{ discover.lan.join('、') }}（摄像头需与之同网段）</p>
    <p v-if="discover.shown && discover.error" class="muted" style="margin:6px 0 0">{{ discover.error }}</p>
    <table v-if="discover.devices.length" style="margin-top:8px">
      <tr><th>IP</th><th>名称</th><th>探测 :554</th><th></th></tr>
      <tr v-for="d in discover.devices" :key="d.ip">
        <td>{{ d.ip }}</td>
        <td>{{ d.name || '-' }}</td>
        <td>
          <button class="ghost" :disabled="probeBusyIp === d.ip" @click="probeIp(d.ip)">
            {{ probeBusyIp === d.ip ? '探测中…' : '探测' }}</button>
          <span v-if="probeRow && probeRow.ip === d.ip" class="muted">
            {{ probeRow.result.port_open ? 'RTSP 口开放' : probeRow.result.error }}
            <template v-if="probeRow.result.ffprobe">
              （{{ probeRow.result.ffprobe.codec }}
              {{ probeRow.result.ffprobe.width }}×{{ probeRow.result.ffprobe.height }}
              @{{ probeRow.result.ffprobe.fps || '?' }}fps）</template>
          </span>
        </td>
        <td><button @click="fillFromDevice(d)">填入表单</button></td>
      </tr>
    </table>
    <p class="muted" style="margin-top:6px">
      需要 ONVIF/WS-Discovery（UDP 3702 组播）；搜不到就用品牌模板手输。填入后把"用户名:密码"
      改成真实账号（建议子码流、只读账号），再点添加。不会自动添加或启动任何摄像头。</p>
  </div>

  <div class="panel">
    <AsyncState :loading="loading" :error="camsError" :notice="stale ? '刷新失败，仍显示上一次列表' : ''"
                :empty="loaded && !cams.length"
                empty-text="暂无摄像头 —— 用上方表单添加：视频文件/RTSP/USB/图片文件夹均可，添加后回到这里启动"
                @retry="refresh">
    <!-- 任务111：分组筛选与批量启停 -->
    <div class="row" style="margin-bottom:8px">
      <b>分组</b>
      <select v-model="groupFilter" aria-label="分组筛选">
        <option value="">全部（{{ cams.length }} 路）</option>
        <option v-for="g in groups" :key="g.name" :value="g.name">
          {{ g.name || '未分组' }}（{{ g.count }} 路 / 运行 {{ g.running }}）</option>
      </select>
      <button class="ghost" :disabled="bulkBusy || !groupFilter" @click="groupAction(groupFilter, 'start')">启动本组</button>
      <button class="ghost" :disabled="bulkBusy || !groupFilter" @click="groupAction(groupFilter, 'stop')">停止本组</button>
      <button :disabled="bulkBusy || !checked.length" @click="bulk('start')">
        批量启动（{{ checked.length }}）</button>
      <button :disabled="bulkBusy || !checked.length" @click="bulk('stop')">
        批量停止（{{ checked.length }}）</button>
    </div>
    <table>
      <tr><th></th><th>ID</th><th>名称</th><th>类型</th><th>分组</th><th>状态</th><th>最新画面</th><th></th></tr>
      <tr v-for="c in shownCams" :key="c.id">
        <td><input type="checkbox" :value="c.id" v-model="checked" :aria-label="`选择摄像头 ${c.id}`" /></td>
        <td>{{ c.id }}</td>
        <td>{{ c.name }}</td>
        <td>{{ typeLabel(c.type) }}</td>
        <td>{{ groupLabel(c) }}</td>
        <td>
          <template v-for="v in [viewFor(c)]">
            <span :class="v.status === 'running' ? 'st-ok' : v.status === 'error' ? 'st-err' : 'st-idle'">{{ v.status }}</span>
            <span v-if="v.fps" class="muted"> {{ v.fps }}fps</span>
            <div v-if="v.last_error" class="st-error-line" :title="v.last_error">{{ v.last_error.slice(0, 60) }}</div>
            <div v-if="v.status === 'running' && v.frame_age_s != null && v.frame_age_s > 10" class="st-warn-line">画面停滞 {{ v.frame_age_s }}s</div>
            <div v-if="v.reconnect_count > 0" class="muted">重连 {{ v.reconnect_count }} 次</div>
            <!-- 任务114：守护状态 -->
            <div v-if="v.supervisor_state === 'backoff'" class="st-warn-line"
                 :title="`自动重启 ${v.restart_count ?? 0} 次 · 上次 ${v.last_restart_ts ?? '-'}`">
              重试中（第 {{ (v.restart_count ?? 0) + 1 }} 次）</div>
            <div v-if="v.supervisor_state === 'given_up'" class="st-err"
                 :title="`自动重启 ${v.restart_count ?? 0} 次均失败 · 上次 ${v.last_restart_ts ?? '-'}；手动点「启动」可重新纳入守护`">
              自动重启失败，请检查</div>
            <div v-if="c.type === 'folder'" class="muted">
              已识别 {{ v.processed_total ?? 0 }} 张 · 关键 {{ v.key_total ?? 0 }} · 违规 {{ v.violation_total ?? 0 }}
              <span v-if="v.backlog"> · 待处理 {{ v.backlog }}</span>
              <span v-if="v.error_total"> · 读取失败 {{ v.error_total }}</span>
            </div>
            <div v-if="c.type === 'folder' && v.current_file" class="muted" :title="v.current_file"
                 style="max-width:220px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap">
              最近：{{ v.current_file.split(/[\\/]/).pop() }}</div>
          </template>
        </td>
        <td><img v-if="snap[c.id]" :src="mediaUrl(`/api/cameras/${c.id}/snapshot?t=${snap[c.id]}`)" alt="摄像头最新画面缩略图" class="snap" @click="snap[c.id] = Date.now()" /></td>
        <td>
          <button class="ghost" style="margin-right:6px" @click="snap[c.id] = Date.now()">看快照</button>
          <button v-if="c.type !== 'folder'" class="ghost" style="margin-right:6px" @click="watchLive(c)">实时画面</button>
          <button v-if="c.type === 'folder'" class="ghost" style="margin-right:6px" :disabled="busyCam !== null" @click="scanFolder(c.id)">
            {{ busyCam === c.id ? '扫描中…' : '立即扫描' }}</button>
          <button class="ghost" style="margin-right:6px" :disabled="busyCam !== null" @click="start(c.id)">
            {{ busyCam === c.id ? '启动中…' : '启动' }}</button>
          <button class="ghost" :disabled="busyCam !== null" @click="del(c.id)">
            {{ busyCam === c.id ? '删除中…' : '删除' }}</button>
        </td>
      </tr>
    </table>
    <p v-if="liveErr" class="muted" style="margin-top:6px">{{ liveErr }}（需先启动 tools/go2rtc.exe，见 scripts/start-all.bat）</p>
    </AsyncState>
  </div>

  <div v-if="live" style="position:fixed; inset:0; background:rgba(0,0,0,.85); display:flex; align-items:center; justify-content:center; cursor:zoom-out" @click="live = null">
    <div style="width:min(92vw, 960px); cursor:default" @click.stop>
      <WebrtcPlayer :ws-url="live.wsUrl" :label="live.name" />
      <p class="muted" style="text-align:center; margin-top:8px">实时画面（WebRTC，延迟约 1s）· 点击空白处关闭</p>
    </div>
  </div>
</template>

<style scoped>
/* 任务17B：375px 小屏适配（scoped） */
.panel { overflow-x: auto; }
button { min-height: 40px; }
/* 任务65：摄像头状态三态着色与可观测信息 */
.st-ok { color: #16a34a; font-weight: 600; }
.st-err { color: #dc2626; font-weight: 600; }
.st-idle { color: #6b7280; }
.st-error-line { color: #dc2626; font-size: 12px; max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.st-warn-line { color: #b45309; font-size: 12px; }
</style>
