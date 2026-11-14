<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import { api, mediaUrl } from '../api'
import WebrtcPlayer from '../components/WebrtcPlayer.vue'
import AsyncState from '../components/AsyncState.vue'
import FsPicker from '../components/FsPicker.vue'

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
// RA5-UX：截图文件夹目录选择器（图片文件夹接入用）
const fsPick = ref(null)
const fsPickTitle = ref('选择文件夹')
function pickSource() {
  fsPickTitle.value = '选择截图文件夹'
  fsPick.value?.openAt(form.value.source || '')
}
function onPicked(p) {
  form.value.source = p
}


// T-336：一键切换/回退分析器（python↔rust）；运行中由服务端自动重启该路
async function setAnalyzer(c, ev) {
  const impl = ev.target.value
  const word = impl === 'rust' ? '切换到 Rust 分析器（灰度）' : '回退到 Python 分析器'
  if (!confirm(`确定把「${c.name}」${word}？运行中将自动重启该路。`)) {
    ev.target.value = c.analyzer || 'python'
    return
  }
  try {
    await api.setCameraAnalyzer(c.id, impl)
    await refresh()
  } catch (e) {
    liveErr.value = `切换失败: ${e.message || e}`
    ev.target.value = c.analyzer || 'python'
  }
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
      <button v-if="form.type === 'folder'" class="ghost" @click="pickSource">浏览…</button>
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
          <!-- T-336：迁移期一键切换/回退分析器（folder 源 rust 数据面不支持，不展示） -->
          <select v-if="c.type !== 'folder'" :value="c.analyzer || 'python'"
                  :aria-label="`分析器实现 摄像头${c.id}`"
                  title="分析器实现：python=默认 / rust=灰度；运行中切换将自动重启该路"
                  style="margin-right:6px" @change="setAnalyzer(c, $event)">
            <option value="python">py</option>
            <option value="rust">rust</option>
          </select>
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
  <FsPicker ref="fsPick" :title="fsPickTitle" @pick="onPicked" />
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
