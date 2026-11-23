<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api, mediaUrl } from '../api'
import LiveCameraViewer from '../components/LiveCameraViewer.vue'
import AsyncState from '../components/AsyncState.vue'
import FsPicker from '../components/FsPicker.vue'

const cams = ref([])
const props = defineProps({ routeQuery: { type: String, default: '' } })
const cameraFilter = ref('')
const focusedCamera = ref(null)
watch(() => props.routeQuery, query => {
  const params = new URLSearchParams(query)
  cameraFilter.value = params.get('status') === 'offline' ? 'offline' : ''
  const id = Number(params.get('camera'))
  focusedCamera.value = Number.isInteger(id) && id > 0 ? id : null
}, { immediate: true })
const form = ref({ name: '', source: '', type: 'file', autostart: false, group_name: '' })
const dahua = ref({ host: '', port: 554, channel: 1, username: '', password: '', substream: true })
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
const addStatus = ref('')
const previewErrors = ref({})
const folderStats = ref({})
const pendingReplays = new Set()
let mounted = true
let replayTimer

function watchReplay(id) {
  pendingReplays.add(id)
  if (!replayTimer) replayTimer = setTimeout(checkReplays, 500)
}

async function checkReplays() {
  const ids = [...pendingReplays]
  await Promise.allSettled(ids.map(async id => {
    const result = await api.cameraStatus(id)
    if (!mounted || typeof result?.status !== 'string') return
    const camera = cams.value.find(camera => camera.id === id)
    if (camera) {
      camera.status = result.status
      camera.processed_frames = Number.isInteger(result.processed_frames) ? result.processed_frames : null
    }
    if (['completed', 'error', 'model_error', 'stopped'].includes(result.status)) pendingReplays.delete(id)
  }))
  replayTimer = mounted && pendingReplays.size ? setTimeout(checkReplays, 1000) : undefined
}

onUnmounted(() => {
  mounted = false
  clearTimeout(replayTimer)
})
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
const live = ref(null)
const liveErr = ref('')
const analysis = ref(null)
const analysisSaving = ref(false)
const analysisError = ref('')

function editAnalysis(camera) {
  analysisError.value = ''
  analysis.value = {
    id: camera.id, name: camera.name,
    mode: camera.analysis_mode || 'main', source: '',
    hasSubstream: camera.analysis_mode === 'substream',
  }
}

async function saveAnalysis() {
  const current = analysis.value
  if (!current || analysisSaving.value) return
  analysisSaving.value = true
  analysisError.value = ''
  try {
    await api.setAnalysisSource(current.id, { mode: current.mode, source: current.source.trim() })
    analysis.value = null
    await refresh()
  } catch (error) {
    analysisError.value = String(error.message || error)
  } finally {
    analysisSaving.value = false
  }
}
// RA5-UX：截图文件夹目录选择器（图片文件夹接入用）
const fsPick = ref(null)
const fsPickTitle = ref('选择文件夹')
function pickSource() {
  const isVideo = form.value.type === 'file'
  fsPickTitle.value = isVideo ? '选择视频文件' : '选择截图文件夹'
  let start = form.value.source
  if (isVideo && /\.(mp4|mkv|avi|mov|ts|flv)$/i.test(start)) {
    start = start.slice(0, Math.max(start.lastIndexOf('\\'), start.lastIndexOf('/')) + 1)
  }
  fsPick.value?.openAt(start)
}
function onPicked(p) {
  form.value.source = p
  if (form.value.type === 'file' && !form.value.name.trim()) {
    form.value.name = p.split(/[\\/]/).pop().replace(/\.[^.]+$/, '')
  }
}

async function refreshFolders() {
  if (!cams.value.some(c => c.type === 'folder')) return
  try {
    const summary = await api.folderImagesSummary({ days: 3650 })
    if (Array.isArray(summary?.items)) {
      folderStats.value = Object.fromEntries(summary.items.map(item => [item.camera_id, item]))
    }
  } catch { /* 保留最近一次读取到的汇总 */ }
}


// RA5/RA4.2 → UX2（2026-11-20）：分析器切换升级为四选——跟随全局（默认，全局
// 策略见设置→模型与检测）/ 仅本地 / 仅 API / 本地+API 协作；运行中由服务端软重启该路
const ANALYZER_LABELS = {
  default: '跟随全局（默认）', rust: '仅本地', vlm: '仅 API',
  cascade: '本地+API 协作',
}
function analyzerOf(c) {
  if (c.analyzer === 'python') return 'rust' // 历史值 python 一律视为仅本地
  return ANALYZER_LABELS[c.analyzer] ? c.analyzer : 'default' // 未知值回退跟随全局
}
async function setAnalyzer(c, ev) {
  const impl = ev.target.value
  const word = {
    default: '改为跟随全局策略', rust: '改为仅本地检测', vlm: '改为仅 API 精判',
    cascade: '改为本地+API 协作（本地先行，可疑帧交 API）',
  }[impl] || impl
  if (!confirm(`确定把「${c.name}」${word}？运行中将自动重启该路。`)) {
    ev.target.value = analyzerOf(c)
    return
  }
  try {
    await api.setCameraAnalyzer(c.id, impl)
    await refresh()
  } catch (e) {
    liveErr.value = `切换失败: ${e.message || e}`
    ev.target.value = analyzerOf(c)
  }
}

async function refresh() {
  try {
    const rows = await api.cameras()
    if (!Array.isArray(rows)) throw new Error('摄像头列表格式异常')
    cams.value = rows
    for (const camera of rows) {
      if (camera?.type === 'file' && ['starting', 'running'].includes(camera.status)) watchReplay(camera.id)
    }
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
  try {
    const result = await api.cameraGroups()
    if (Array.isArray(result?.groups)) groups.value = result.groups
  } catch { /* 保留已加载分组 */ }
  await refreshFolders()
}

// 任务111：分组筛选 + 批量启停
function isOffline(camera) {
  return ['rtsp', 'usb'].includes(camera.type) && camera.status !== 'stopped' &&
    (['error', 'model_error'].includes(camera.status) || Boolean(camera.offline_since))
}
const shownCams = computed(() => cams.value.filter(c =>
  (!groupFilter.value || c.group_name === groupFilter.value) &&
  (!cameraFilter.value || isOffline(viewFor(c))) &&
  (focusedCamera.value === null || c.id === focusedCamera.value)))
function groupLabel(c) { return c.group_name || '未分组' }
async function bulk(action) {
  const ids = [...checked.value]
  if (!ids.length || bulkBusy.value) return
  const word = action === 'start' ? '启动' : '停止'
  if (!confirm(`确定${word}选中的 ${ids.length} 路摄像头？`)) return
  bulkBusy.value = true
  try {
    const r = await api.bulkCameras(ids, action)
    if (Array.isArray(r?.missing) && r.missing.length) alert(`不存在的摄像头: ${r.missing.join(',')}`)
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
  const list = sys.value?.cameras
  const m = (Array.isArray(list) ? list.find(x => x.id === c.id) : null) || {}
  return { ...c, ...m, status: c.status }
}
onMounted(() => { refresh() })


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




async function add() {
  if (adding.value) return
  const isDahua = form.value.type === 'dahua'
  if (!form.value.name.trim() || (!isDahua && !form.value.source.trim())) {
    addStatus.value = '请填写名称和接入地址。'
    return
  }
  // USB 摄像头（2026-11-21）：枚举下拉直接给 `video=设备名`；手填场景（枚举
  // 不可用）自动补前缀——ffmpeg dshow 按设备名取流，裸数字索引 ffmpeg 不认
  if (form.value.type === 'usb' && !form.value.source.startsWith('video=')
      && !form.value.source.startsWith('/dev/')) {
    form.value.source = 'video=' + form.value.source.trim()
  }
  adding.value = true
  try {
    const { type, autostart } = form.value
    const payload = isDahua ? { ...form.value, type: 'rtsp', source: '', dahua: dahua.value } : form.value
    const result = await api.addCamera(payload)
    addStatus.value = type === 'file'
      ? (autostart ? '视频已添加，正在启动分析。' : '视频已添加，可先预览首帧，再点击启动分析。')
      : '监控源已添加。'
    if (type === 'file' && Number.isInteger(result?.id)) {
      showSnapshot(result.id)
      if (autostart) watchReplay(result.id)
    }
    form.value = { name: '', source: '', type: 'file', autostart: false }
    dahua.value.password = ''
    refresh()
  } catch (e) { addStatus.value = String(e.message || e) }
  adding.value = false
}

function showSnapshot(id) {
  previewErrors.value[id] = false
  snap.value[id] = Date.now()
}
async function start(id) {
  if (busyCam.value !== null) return
  busyCam.value = id
  try {
    const result = await api.startCamera(id)
    if (result?.started && cams.value.some(camera => camera.id === id && camera.type === 'file')) watchReplay(id)
    refresh()
  } catch (e) { alert(String(e.message || e)) }
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

function watchLive(cam) {
  liveErr.value = ''
  live.value = cam
}
</script>

<template>
  <div class="panel">
    <h3 style="margin:14px 0 10px">添加摄像头</h3>
    <div class="row">
      <input v-model="form.name" placeholder="名称" aria-label="摄像头名称" style="width:120px" />
      <template v-if="form.type === 'usb'">
        <select v-if="usbDevices.length" v-model="form.source" style="flex:1" aria-label="USB 设备选择">
          <option v-for="d in usbDevices" :key="d.index" :value="d.source">{{ d.name }}（索引 {{ d.index }}）</option>
        </select>
        <input v-else v-model="form.source" placeholder="USB 设备名（如 HD Webcam），或点「刷新设备」自动枚举" aria-label="USB 设备名" style="flex:1" />
        <button class="ghost" :disabled="usbLoading" @click="loadUsb(true)">{{ usbLoading ? '刷新中…' : '刷新设备' }}</button>
      </template>
      <input v-else-if="form.type !== 'dahua'" v-model="form.source"
             :placeholder="form.type === 'folder' ? '截图文件夹路径，如 D:\\screenshots（支持中文/网络盘）' : 'RTSP 地址 / 视频文件路径 / video=设备名 (USB)'"
             :aria-label="form.type === 'folder' ? '图片文件夹路径' : '摄像头地址'" style="flex:1" />
      <button v-if="form.type === 'file' || form.type === 'folder'" class="ghost" @click="pickSource">浏览…</button>
      <select v-model="form.type">
        <option value="file">视频文件</option>
        <option value="rtsp">RTSP</option>
        <option value="dahua">大华摄像头／录像机</option>
        <option value="usb">USB</option>
        <option value="folder">图片文件夹</option>
      </select>
      <select v-model="cameraFilter" aria-label="监控状态筛选">
        <option value="">全部状态</option>
        <option value="offline">离线摄像头</option>
      </select>
      <button v-if="focusedCamera !== null" type="button" class="ghost" @click="focusedCamera = null">显示全部摄像头</button>
      <a v-if="cameraFilter === 'offline'" href="#/settings/service">查看运行诊断</a>
      <label class="muted"><input type="checkbox" v-model="form.autostart" style="margin:0 4px 0 0" />立即启动</label>
      <input v-model="form.group_name" placeholder="分组（可选，≤32字）" aria-label="摄像头分组" style="width:140px" />
      <button :disabled="adding" @click="add">{{ adding ? '添加中…' : '添加' }}</button>
    </div>
    <div v-if="form.type === 'dahua'" class="row" style="margin-top:8px">
      <input v-model.trim="dahua.host" aria-label="大华设备地址" placeholder="设备局域网 IP 或主机名" />
      <label>RTSP 端口 <input v-model.number="dahua.port" type="number" min="1" max="65535" style="width:95px" /></label>
      <label>通道 <input v-model.number="dahua.channel" type="number" min="1" max="65535" style="width:75px" /></label>
      <input v-model="dahua.username" aria-label="大华设备用户名" autocomplete="off" placeholder="设备用户名" />
      <input v-model="dahua.password" aria-label="大华设备密码" type="password" autocomplete="new-password" placeholder="设备密码" />
      <label><input v-model="dahua.substream" type="checkbox" /> 用辅码流做检测</label>
    </div>
    <p v-if="form.type === 'dahua'" class="muted">预览使用主码流。填写大华云联或乐橙对应设备的局域网地址和设备账号。录像机按实际通道填写，单台摄像头通常为 1。</p>
    <p v-if="addStatus" role="status" class="muted">{{ addStatus }}</p>
    <p v-if="!shownCams.length && (cameraFilter || focusedCamera !== null)" class="muted" role="status">
      {{ cameraFilter === 'offline' ? '当前范围没有离线摄像头。' : '当前范围没有这路摄像头。' }}
    </p>
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
      <thead><tr><th>IP</th><th>名称</th><th>探测 :554</th><th></th></tr></thead>
      <tbody>
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
      </tbody>
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
      <thead><tr><th></th><th>ID</th><th>名称</th><th>类型</th><th>分组</th><th>状态</th><th>最新画面</th><th></th></tr></thead>
      <tbody>
      <tr v-for="c in shownCams" :key="c.id">
        <td><input type="checkbox" :value="c.id" v-model="checked" :aria-label="`选择摄像头 ${c.id}`" /></td>
        <td>{{ c.id }}</td>
        <td>{{ c.name }}</td>
        <td>{{ typeLabel(c.type) }}</td>
        <td>{{ groupLabel(c) }}</td>
        <td>
          <template v-for="v in [viewFor(c)]">
            <span :class="isOffline(v) ? 'st-err' : v.status === 'running' ? 'st-ok' : 'st-idle'">
              {{ v.status === 'model_error' ? '人员模型加载失败' : isOffline(v) ? (v.status === 'reconnecting' ? '离线 · 重连中' : '离线') : ({ running: '运行中', stopped: '已停止', completed: '检查完成', starting: '启动中', reconnecting: '重连中', error: '异常' }[v.status] || v.status) }}
            </span>
            <div v-if="v.status === 'model_error'"><a href="#/settings/detect">检查模型设置</a></div>
            <span v-if="v.fps" class="muted"> {{ v.fps }}fps</span>
            <div v-if="c.type === 'file' && v.status === 'running' && c.processed_frames != null" class="muted">
              已检查 {{ c.processed_frames }} 帧
            </div>
            <a v-if="c.type === 'file' && v.status === 'completed'"
               :href="`#/events?camera_id=${c.id}&category=check`">查看检查记录</a>
            <div v-if="v.last_error" class="st-error-line" :title="v.last_error">{{ v.last_error.slice(0, 60) }}</div>
            <div v-if="v.status === 'running' && v.frame_age_s != null && v.frame_age_s > 10" class="st-warn-line">画面停滞 {{ Math.round(v.frame_age_s) }} 秒</div>
            <div v-if="v.offline_since && v.status !== 'stopped'" class="muted">断流于 {{ v.offline_since }}</div>
            <div v-if="v.reconnect_count > 0" class="muted">重连 {{ v.reconnect_count }} 次</div>
            <!-- 任务114：守护状态 -->
            <div v-if="v.supervisor_state === 'backoff'" class="st-warn-line"
                 :title="`自动重启 ${v.restart_count ?? 0} 次 · 上次 ${v.last_restart_ts ?? '-'}`">
              重试中（第 {{ (v.restart_count ?? 0) + 1 }} 次）</div>
            <div v-if="v.supervisor_state === 'given_up'" class="st-err"
                 :title="`自动重启 ${v.restart_count ?? 0} 次均失败 · 上次 ${v.last_restart_ts ?? '-'}；手动点「启动」可重新纳入守护`">
              自动重启失败，请检查</div>
            <div v-if="c.type === 'folder' && folderStats[c.id]" class="muted">
              已处理 {{ folderStats[c.id].total }} 张 · 关键 {{ folderStats[c.id].key }} · 违规 {{ folderStats[c.id].violation }}
              <span v-if="folderStats[c.id].errors"> · 读取失败 {{ folderStats[c.id].errors }}</span>
            </div>
            <div v-if="c.type === 'folder' && v.current_file" class="muted" :title="v.current_file"
                 style="max-width:220px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap">
              最近：{{ v.current_file.split(/[\\/]/).pop() }}</div>
          </template>
        </td>
        <td>
          <span v-if="previewErrors[c.id]" class="muted">暂无预览</span>
          <img v-else-if="snap[c.id]" :src="mediaUrl(`/api/cameras/${c.id}/snapshot?t=${snap[c.id]}${c.type === 'file' ? '&preview=1' : ''}`)"
               alt="监控源预览" class="snap" @click="showSnapshot(c.id)" @error="previewErrors[c.id] = true" />
        </td>
        <td>
          <!-- UX2 检测模式：跟随全局 / 仅本地 / 仅 API / 本地+API 协作（全局策略在 设置→模型与检测） -->
          <select :value="analyzerOf(c)"
                  :aria-label="`分析方式 摄像头${c.id}`"
                  title="分析方式：跟随全局=设置中心的检测模式策略；仅本地=包内 ONNX 零成本；仅 API=全量交外部联网模型；协作=本地先扫可疑帧再精判（省费用）。运行中切换将自动重启该路"
                  style="margin-right:6px" @change="setAnalyzer(c, $event)">
            <option value="default">跟随全局（默认）</option>
            <option value="rust">仅本地</option>
            <option value="vlm">仅 API</option>
            <option value="cascade">本地+API 协作</option>
          </select>
            <button class="ghost" style="margin-right:6px" @click="showSnapshot(c.id)">{{ c.type === 'file' ? '预览画面' : '看快照' }}</button>
          <button v-if="c.type !== 'folder'" class="ghost" style="margin-right:6px" @click="watchLive(c)">实时画面</button>
          <button v-if="c.type === 'rtsp'" class="ghost" :disabled="analysisSaving" @click="editAnalysis(c)">分析码流</button>
          <button v-if="c.type === 'folder'" class="ghost" style="margin-right:6px" :disabled="busyCam !== null" @click="scanFolder(c.id)">
            {{ busyCam === c.id ? '扫描中…' : '立即扫描' }}</button>
          <button class="ghost" style="margin-right:6px" :disabled="busyCam !== null" @click="start(c.id)">
            {{ busyCam === c.id ? '启动中…' : viewFor(c).status === 'completed' ? '重新分析' : '启动' }}</button>
          <button class="ghost" :disabled="busyCam !== null" @click="del(c.id)">
            {{ busyCam === c.id ? '删除中…' : '删除' }}</button>
        </td>
      </tr>
      </tbody>
    </table>
    <form v-if="analysis" class="analysis-editor" @submit.prevent="saveAnalysis">
      <h3>{{ analysis.name }} · 分析码流</h3>
      <div class="row">
        <label>分析来源
          <select v-model="analysis.mode" :disabled="analysisSaving">
            <option value="main">直连主码流</option>
            <option value="relay">与预览共享本机中继</option>
            <option value="substream">单独使用子码流</option>
          </select>
        </label>
        <label v-if="analysis.mode === 'substream'">子码流地址
          <input v-model="analysis.source" type="password" autocomplete="off" :disabled="analysisSaving"
            :placeholder="analysis.hasSubstream ? '留空保留已保存的地址' : 'rtsp://摄像头地址/子码流'"
            :required="!analysis.hasSubstream" />
        </label>
      </div>
      <p v-if="analysis.mode === 'substream'" class="muted">请选择与主码流视角、宽高比一致的子码流。实时预览继续使用主码流。</p>
      <p v-if="analysis.mode !== 'main'" class="muted">分析源失联时回退到直连主码流。</p>
      <p v-if="analysisError" class="st-err" role="alert">{{ analysisError }}</p>
      <div class="row">
        <button type="submit" :disabled="analysisSaving">{{ analysisSaving ? '保存中…' : '保存' }}</button>
        <button type="button" class="ghost" :disabled="analysisSaving" @click="analysis = null">取消</button>
      </div>
    </form>
    <p v-if="liveErr" class="muted" style="margin-top:6px">{{ liveErr }}</p>
    </AsyncState>
  </div>

  <div v-if="live" style="position:fixed; inset:0; background:rgba(0,0,0,.85); display:flex; align-items:center; justify-content:center; cursor:zoom-out" @click="live = null">
    <div style="width:min(92vw, 960px); cursor:default" @click.stop>
      <LiveCameraViewer :camera="live" />
      <p class="muted" style="text-align:center; margin-top:8px">{{ live.name }} · 点击空白处关闭</p>
    </div>
  </div>
  <FsPicker ref="fsPick" :title="fsPickTitle" :video="form.type === 'file'" @pick="onPicked" />
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
.analysis-editor { display: grid; gap: 10px; padding: 16px; margin-top: 12px; border: 1px solid var(--stroke-default); border-radius: 10px; }
.analysis-editor .row { flex-wrap: wrap; }
.analysis-editor label { display: grid; gap: 4px; }
</style>
