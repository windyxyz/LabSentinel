<script setup>
// RA5-UX（ADR-0011）：设置中心——从「摄像头」页拆出的管理面（分类侧栏 + 搜索）。
// 各区块整段搬移自 Cameras.vue（逻辑零改动）。
// UX1（2026-11-20）：9 分区扩容——「模型与检测」置顶（UX2 三模式）；设备管理/
// 人脸管理 从一级导航迁入；原「模型管理」页并入「模型与检测」的本地模型卡。
import { computed, onMounted, ref } from 'vue'
import { api, adminToken } from '../api'
import DetectPanel from './DetectPanel.vue'
import Devices from './Devices.vue'
import Face from './Face.vue'
import AnnotatorModal from '../components/AnnotatorModal.vue'

const search = ref('')
const active = ref('detect')
const sections = [
  { id: 'detect', label: '模型与检测', kw: '检测模式 本地 api 联网 协作 cascade 精判 省钱 预算 置信度 模型 vlm ppe 实验服', icon: '<rect x="2.5" y="3" width="11" height="10" rx="1.5"/><path d="M5 7h6M5 10h4"/>' },
  { id: 'token', label: '管理令牌', kw: '令牌 token 鉴权 认证', icon: '<circle cx="5" cy="8" r="3"/><path d="M7.6 8H14v2.2M12 8v2.2"/>' },
  { id: 'notify', label: '通知与告警', kw: '渠道 测试 静默 投递 失败 告警 企业微信 钉钉 邮件', icon: '<path d="M8 2.5a3.5 3.5 0 0 1 3.5 3.5v3l1.5 2.5H3l1.5-2.5v-3A3.5 3.5 0 0 1 8 2.5z"/><path d="M6.5 13a1.8 1.8 0 0 0 3 0"/>' },
  { id: 'template', label: '告警文案', kw: '模板 预览 占位符 文案 标题 正文', icon: '<path d="M3 4h10M3 8h10M3 12h6"/>' },
  { id: 'devices', label: '设备与配对', kw: '设备 配对 令牌 移动端 手机 撤销 三级', icon: '<rect x="4.5" y="2" width="7" height="12" rx="1.5"/><path d="M7 12.2h2"/>' },
  { id: 'face', label: '人脸识别', kw: '人脸 名册 注册 同意 合规 陌生人 删除', icon: '<circle cx="8" cy="5.5" r="2.8"/><path d="M2.5 13.8c.9-2.7 3-4 5.5-4s4.6 1.3 5.5 4"/>' },
  { id: 'data', label: '数据管理', kw: '导入 导出 备份 配置 json 迁移', icon: '<path d="M3 5.5C3 4.1 5.2 3 8 3s5 1.1 5 2.5S10.8 8 8 8 3 6.9 3 5.5z"/><path d="M3 5.5v5C3 11.9 5.2 13 8 13s5-1.1 5-2.5v-5"/>' },
  { id: 'plugins', label: '插件', kw: '插件 外挂 zip 上传 启用 日志 扩展', icon: '<path d="M6 2.5h4v2.2a1.6 1.6 0 1 0 2.2 2.2H14v4h-1.8a1.6 1.6 0 1 1 0 2.2H14v1.4H2v-1.4h1.8a1.6 1.6 0 1 0-2.2-2.2H2v-4h2.2A1.6 1.6 0 1 0 6 4.7z"/>' },
  { id: 'network', label: '网络与运维', kw: '局域网 端口 令牌 开机自启 日报 周报 定时 报告 重启', icon: '<circle cx="8" cy="8" r="5.5"/><path d="M2.5 8h11M8 2.5c2 1.8 2 9.2 0 11M8 2.5c-2 1.8-2 9.2 0 11"/>' },
  { id: 'service', label: '服务控制', kw: '停止 服务 重启 退出 关机 日志 尾部', icon: '<path d="M8 2.5v5.5"/><path d="M4.6 4.8a4.8 4.8 0 1 0 6.8 0"/>' },
]
const visibleSections = computed(() => {
  const q = search.value.trim().toLowerCase()
  if (!q) return sections
  return sections.filter(s => (s.label + s.kw).toLowerCase().includes(q))
})
function hit(s) {
  const q = search.value.trim().toLowerCase()
  return !!q && (s.label + s.kw).toLowerCase().includes(q)
}
// 有搜索词时直接展示命中分区（无需点击）；无搜索词按侧栏选中项展示
function show(id) {
  if (search.value.trim()) return hit(sections.find(s => s.id === id))
  return active.value === id
}
function go(id) { active.value = id; search.value = '' }

// ---- 以下为搬移的业务逻辑（零改动）----
const tokenInput = ref(adminToken.get())
const tokenMsg = ref('')

const cams = ref([])   // 静默窗口的摄像头下拉

function saveToken() {
  adminToken.set(tokenInput.value.trim())
  tokenMsg.value = adminToken.get() ? '令牌已保存（仅存本浏览器）' : '令牌已清除'
}


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
    const st = await api.alertsStatus()
    alertChannels.value = Array.isArray(st?.channels) ? st.channels : []
    const fd = await api.alertDeliveries({ status: 'failed', limit: 10 })
    failedDeliveries.value = Array.isArray(fd) ? fd : []
    const sl = await api.alertSilences()
    silences.value = Array.isArray(sl) ? sl : []
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
    const raw = await api.alertTest()
    // 后端形状兼容：数组（Python 契约）或 {results:[...]}（rust 现状）
    const results = Array.isArray(raw) ? raw : (raw.results || [])
    if (!results.length) {
      alertTestMsg.value = '尚未配置任何渠道——请先在上方「告警渠道状态」里配置至少一个渠道（企业微信/钉钉/邮件等），再发测试'
      loadAlerts()
      return
    }
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
    const warnings = Array.isArray(r?.warnings) ? r.warnings : []
    cfgMsg.value = `导入完成：新建 ${r?.created?.cameras ?? 0} 摄像头 / ${r?.created?.zones ?? 0} 区域 / ` +
      `${r?.created?.rules ?? 0} 规则；跳过 ${(r?.skipped?.cameras || []).length}/${(r?.skipped?.zones || []).length}/` +
      `${(r?.skipped?.rules || []).length}` + (warnings.length ? `\n${warnings.join('\n')}` : '')
    loadCams()
  } catch (e) { cfgMsg.value = String(e.message || e) }
  cfgBusy.value = false
  importFile.value = null
}


// 任务85/UX（2026-11-21）：告警文案模板——网页直接编辑保存（不再改配置文件）
const tpl = ref({ text: '', title: '', placeholders: {} })
const tplPreview = ref('')
const tplBusy = ref(false)
const tplEditTitle = ref('')
const tplEditText = ref('')
const tplMsg = ref('')
const tplSample = ref({ level: 3, type: 'enter_no_ppe', camera_id: 1, track_id: 7,
                        detail: '示例：有人未穿实验服进入试剂区' })

async function loadTemplate() {
  tplBusy.value = true
  try {
    tpl.value = await api.alertTemplate()
    tplEditTitle.value = tpl.value.title || ''
    tplEditText.value = tpl.value.text || ''
    tplPreview.value = ''
  } catch (e) { tplPreview.value = String(e.message || e) }
  tplBusy.value = false
}

async function saveTemplate() {
  tplBusy.value = true
  try {
    const r = await api.saveAlertTemplate({ title: tplEditTitle.value, text: tplEditText.value })
    tpl.value.text = r.effective || tplEditText.value
    tplPreview.value = ''
    tplMsg.value = '告警文案已保存并即时生效'
  } catch (e) { tplPreview.value = String(e.message || e) }
  tplBusy.value = false
}

async function resetTemplate() {
  tplBusy.value = true
  try {
    await api.saveAlertTemplate({ title: '', text: '' })
    await loadTemplate()
    tplMsg.value = '已恢复默认文案'
  } catch (e) { tplPreview.value = String(e.message || e) }
  tplBusy.value = false
}

async function previewTemplate() {
  tplBusy.value = true
  try {
    // 预览用「当前输入框内容」——后端以文本里传的 title/text 直接渲染
    const r = await api.alertTemplatePreview({
      ...tplSample.value,
      title: tplEditTitle.value,
      text: tplEditText.value,
    })
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

// UX3（2026-11-20）：无黑框启动后的日志可见出口——服务日志尾部（脱敏后回显）
const serverLog = ref(null)
const serverLogBusy = ref(false)
async function loadServerLog() {
  serverLogBusy.value = true
  try {
    const r = await api.logsTail(200)
    serverLog.value = {
      file: r.file || 'logs/server.log',
      lines: (r.lines || []).length ? (r.lines || []).join('\n') : '（暂无落盘日志）',
    }
  } catch (e) { serverLog.value = { file: '', lines: String(e.message || e) } }
  serverLogBusy.value = false
}

// LS-051：go2rtc 托管状态（运行中/未找到/错误）+ 一键开关
const go2rtc = ref(null)
const go2rtcBusy = ref(false)
async function loadGo2rtc() {
  try { go2rtc.value = await api.go2rtcStatus() } catch { /* 旧服务端无此端点：静默 */ }
}

// LS-053：网络与运维（局域网/自启/日报/周报）
const net = ref(null)
const autostart = ref(null)
const reports = ref(null)
const netBusy = ref(false)
const netMsg = ref('')
const newToken = ref('')
async function loadNetwork() {
  try { net.value = await api.networkConfig() } catch { /* 静默 */ }
  try { autostart.value = await api.autostartStatus() } catch { /* 静默 */ }
  try { reports.value = await api.reportSchedule() } catch { /* 静默 */ }
}
async function saveNetwork() {
  if (!net.value) return
  netBusy.value = true; netMsg.value = ''
  try {
    const r = await api.saveNetworkConfig({
      lan: net.value.lan,
      port: net.value.port,
      trusted_hosts: net.value.trusted_hosts,
      generate_token: net.value.generate_token === true,
    })
    if (r.token) newToken.value = r.token
    await loadNetwork()
    netMsg.value = '已写入 .env（重启服务后生效）'
  } catch (e) { netMsg.value = String(e.message || e) }
  netBusy.value = false
}
async function genToken() {
  if (!net.value) return
  netBusy.value = true; netMsg.value = ''
  try {
    const r = await api.saveNetworkConfig({ generate_token: true })
    newToken.value = r.token || ''
    await loadNetwork()
    netMsg.value = '已生成强随机管理令牌（请妥善保存）'
  } catch (e) { netMsg.value = String(e.message || e) }
  netBusy.value = false
}
async function saveReports() {
  if (!reports.value) return
  netBusy.value = true; netMsg.value = ''
  try {
    await api.saveReportSchedule({
      daily_enabled: reports.value.daily_enabled,
      daily_time: reports.value.daily_time,
      weekly_enabled: reports.value.weekly_enabled,
      weekly_day: Number(reports.value.weekly_day),
      weekly_time: reports.value.weekly_time,
    })
    netMsg.value = '日报/周报调度已保存'
  } catch (e) { netMsg.value = String(e.message || e) }
  netBusy.value = false
}
async function sendReportNow() {
  netBusy.value = true; netMsg.value = ''
  try {
    const r = await api.reportSendNow()
    netMsg.value = `已发送 ${r.sent}/${r.channels} 个渠道` + (r.errors?.length ? `；失败：${r.errors.join('；')}` : '')
  } catch (e) { netMsg.value = String(e.message || e) }
  netBusy.value = false
}
async function toggleAutostart() {
  if (!autostart.value?.supported) return
  netBusy.value = true
  try {
    await api.setAutostart(!autostart.value.enabled)
    await loadNetwork()
    netMsg.value = autostart.value.enabled ? '已开启开机自启' : '已关闭开机自启'
  } catch (e) { netMsg.value = String(e.message || e) }
  netBusy.value = false
}
// LS-054：样本标注池 + 只读分享链接
const pool = ref(null)
const poolStatus = ref('')
const shares = ref([])
const shareName = ref('值班分享')
const shareTtl = ref(24)
const shareToken = ref('')
const annotator = ref(null)
async function loadPool() {
  try { pool.value = await api.datasetSamples({ status: poolStatus.value }) } catch { /* 静默 */ }
}
async function exportDataset() {
  const a = document.createElement('a')
  a.href = api.datasetExportUrl()
  a.download = 'labsentinel-yolo-dataset.zip'
  a.click()
}
async function loadShares() {
  try { shares.value = (await api.shareLinks()).links || [] } catch { /* 静默 */ }
}
async function createShare() {
  netBusy.value = true; netMsg.value = ''
  try {
    const r = await api.createShareLink({ name: shareName.value, ttl_hours: Number(shareTtl.value) })
    shareToken.value = r.url || r.token
    await loadShares()
    netMsg.value = '已生成只读值守链接（只显示一次）'
  } catch (e) { netMsg.value = String(e.message || e) }
  netBusy.value = false
}
async function revokeShare(id) {
  if (!confirm('吊销该分享链接？已打开的页面将在下次请求时失效。')) return
  try { await api.deleteShareLink(id); await loadShares() } catch { /* 静默 */ }
}
function openAnnotator(s) {
  annotator.value = { sample: s, imageUrl: `/api/dataset/samples/${s.id}/image` }
}
async function restartServer() {
  if (!confirm('立即重启服务？当前分析与页面会短暂中断。')) return
  netBusy.value = true
  try { await api.restartServer(); netMsg.value = '正在重启，稍后刷新页面…' }
  catch (e) { netMsg.value = String(e.message || e) }
  netBusy.value = false
}
async function toggleGo2rtc() {
  if (!go2rtc.value) return
  go2rtcBusy.value = true
  try {
    await api.go2rtcConfig(!go2rtc.value.enabled)
    await loadGo2rtc()
  } catch (e) { serverLog.value = { file: '', lines: String(e.message || e) } }
  go2rtcBusy.value = false
}


// 任务131：插件（外挂）管理（默认禁用；启用=管理员显式授权）
const plugins = ref([])
const pluginsError = ref('')
const pluginsLoaded = ref(false)
const pluginBusy = ref('')
const pluginLog = ref(null)

async function loadPlugins() {
  try {
    plugins.value = (await api.pluginsList()).plugins
    pluginsError.value = ''
    pluginsLoaded.value = true
  } catch (e) {
    const msg = String(e.message || e)
    pluginsLoaded.value = true
    // 产品化：网络异常（Failed to fetch 等）不裸露浏览器原文
    pluginsError.value = msg.includes('401') || msg.includes('403')
      ? '需要管理令牌——请在页面顶部填写后重试'
      : '无法连接服务，请确认服务正在运行'
  }
}
function pluginStateLabel(p) {
  if (p.last_error && !p.running) return p.last_error
  if (p.running) return `运行中 (pid ${p.pid})`
  if (p.returncode !== null && p.returncode !== undefined)
    return `已退出 (rc=${p.returncode})`
  return p.enabled ? '已启用 · 未运行' : '未启用'
}
async function pluginAct(p, action) {
  if (pluginBusy.value) return
  const word = { enable: '启用', disable: '停用', start: '启动', stop: '停止' }[action] || action
  if (action === 'enable' && (p.permissions || []).includes('write')) {
    if (!confirm(`「${p.name}」含 write 权限：启用后将获得管理员令牌（等同完全控制本服务）。
确定继续？`)) return
  } else if (!confirm(`确定${word}插件「${p.name}」？`)) return
  pluginBusy.value = p.id
  try {
    await api.pluginAction(p.id, action)
    await loadPlugins()
  } catch (e) {
    const msg = String(e.message || e)
    alert(msg + (msg.includes('401') || msg.includes('403') ? '\n请先在页面顶部填写管理令牌' : ''))
  }
  pluginBusy.value = ''
}
async function showPluginLog(p) {
  if (pluginBusy.value) return
  pluginBusy.value = p.id
  try {
    const r = await api.pluginLogs(p.id)
    pluginLog.value = { id: p.id, text: r.logs || '（暂无日志输出）' }
  } catch (e) {
    pluginLog.value = { id: p.id, text: '日志读取失败: ' + (e.message || e) }
  }
  pluginBusy.value = ''
}


async function onUploadZip(e) {
  const f = e.target.files?.[0]
  if (!f) return
  pluginBusy.value = 'upload'
  try {
    const fd = new FormData()
    fd.append('file', f)
    await api.pluginsUpload(fd)
    await loadPlugins()
  } catch (err) {
    const m = String(err.message || err)
    pluginsError.value = m.includes('401') || m.includes('403')
      ? '需要管理令牌——请在页面顶部填写后重试'
      : '安装失败：' + m
  } finally {
    pluginBusy.value = ''
    e.target.value = ''
  }
}


async function loadCams() {
  try { cams.value = await api.cameras() } catch { /* 服务未就绪时静默 */ }
}
onMounted(() => {
  loadAlerts(); loadPlugins(); loadCams(); loadTemplate(); loadGo2rtc(); loadNetwork(); loadPool(); loadShares()
  // UX1：hash 深链分区（#/settings/face 等，App.vue 旧路由重定向后落到这里）
  const sub = window.location.hash.split('/')[2]
  if (sections.some((s) => s.id === sub)) active.value = sub
})
</script>

<template>
  <div class="settings-layout">
    <aside class="settings-nav">
      <input v-model="search" class="settings-search" placeholder="搜索设置"
             aria-label="搜索设置" type="search" />
      <div v-for="s in visibleSections" :key="s.id" class="settings-item"
           :class="{ active: active === s.id && !search.trim() }"
           :aria-current="active === s.id ? 'true' : undefined" @click="go(s.id)">
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3"
             stroke-linecap="round" stroke-linejoin="round" v-html="s.icon"></svg>
        <span>{{ s.label }}</span>
      </div>
      <p v-if="!visibleSections.length" class="muted settings-empty">未找到匹配的设置</p>
    </aside>
    <div class="settings-body">
      <section v-show="show('detect')" class="panel detect-panel-host">
        <DetectPanel />
      </section>
      <section v-show="show('token')" class="panel">
    <h3 style="margin-bottom:10px">管理令牌</h3>
    <div class="row">
      <input v-model="tokenInput" placeholder="服务端启用鉴权时填写（LS_ADMIN_TOKEN）" aria-label="管理令牌" style="flex:1" />
      <button @click="saveToken">保存令牌</button>
      <span v-if="tokenMsg" class="muted">{{ tokenMsg }}</span>
    </div>
    <p class="muted" style="margin-top:4px">
      令牌只保存在本浏览器（localStorage），不上传到任何地方。它是「管理员密码」性质：
      仅当服务端以局域网/外网模式启动且设置了 LS_ADMIN_TOKEN 时才需要填写——
      本机访问（127.0.0.1）默认无需令牌。</p>
      </section>
      <section v-show="show('notify')" class="panel">
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
    <p class="muted" style="margin-top:2px">各渠道的申请与配置方法见《部署指南》「告警渠道配置」章节。</p>
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
      <button class="ghost" @click="addSilence(silenceScope)">添加</button>
    </div>
    <table v-if="silences.length" style="margin-top:8px">
      <tr><th>ID</th><th>范围</th><th>目标</th><th>剩余</th><th>原因</th><th></th></tr>
      <tr v-for="s in silences" :key="s.id">
        <td>#{{ s.id }}</td><td>{{ s.scope }}</td><td>{{ s.target ?? '—' }}</td>
        <td class="muted">{{ Number.isFinite(Number(s.remaining_s)) ? Math.ceil(Number(s.remaining_s) / 60) + ' 分钟' : (s.until_ts || '—') }}</td>
        <td class="muted">{{ s.reason || '—' }}</td>
        <td><button class="ghost" @click="removeSilence(s.id)">解除</button></td>
      </tr>
    </table>
    <p v-else class="muted">无静默（事件照常推送）</p>
    <p v-if="silenceMsg" class="muted" style="margin-top:4px">{{ silenceMsg }}</p>

      </section>
      <section v-show="show('devices')" class="panel">
        <Devices />
      </section>
      <section v-show="show('face')" class="panel">
        <Face />
      </section>
      <section v-show="show('data')" class="panel">
    <h3 style="margin:14px 0 10px">配置导入导出</h3>
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
      导出的配置不含摄像头密码（已脱敏）；导入后不会自动启动摄像头。</p>
    <hr style="border:none;border-top:1px solid var(--divider);margin:12px 0" />
    <h3 style="margin:0 0 8px">样本标注池（数据飞轮）</h3>
    <div class="row" style="align-items:center;flex-wrap:wrap;gap:8px">
      <select v-model="poolStatus" aria-label="样本状态筛选" @change="loadPool">
        <option value="">全部状态</option>
        <option value="pending">待标注</option>
        <option value="verified">已核验</option>
        <option value="ambiguous">模糊弃用</option>
      </select>
      <button class="ghost" @click="loadPool">刷新</button>
      <button class="ghost" @click="exportDataset">导出 YOLO 训练集 ZIP（仅已核验）</button>
      <span class="muted" style="font-size:12px">共 {{ pool?.total ?? 0 }} 个样本</span>
    </div>
    <div v-if="pool?.class_counts?.length" class="muted" style="font-size:12px;margin-top:6px">
      类别分布：<span v-for="c in pool.class_counts" :key="c.class" style="margin-right:10px">{{ c.class }} {{ c.count }}</span>
    </div>
    <table v-if="pool?.samples?.length" style="margin-top:8px">
      <tr><th>ID</th><th>来源</th><th>状态</th><th>框数</th><th></th></tr>
      <tr v-for="s in pool.samples.slice(0, 50)" :key="s.id">
        <td>#{{ s.id }}</td>
        <td>{{ s.source_type }} {{ s.source_id }}</td>
        <td><span :class="s.status === 'verified' ? 'st-ok' : 'st-idle'">{{ s.status }}</span></td>
        <td>{{ Array.isArray(s.boxes) ? s.boxes.length : 0 }}</td>
        <td>
          <button class="ghost" @click="openAnnotator(s)">标注</button>
          <button class="ghost" @click="api.datasetDelete(s.id).then(loadPool)">删除</button>
        </td>
      </tr>
    </table>
    <p v-else class="muted" style="margin-top:8px">暂无样本——在事件详情或图片文件夹详情点「加入标注池」，或从实时画面抓拍。</p>
      </section>
      <section v-show="show('template')" class="panel">
    <h3 style="margin:14px 0 10px">告警文案模板 <span class="muted" style="font-size:12px">（网页编辑，保存即生效）</span></h3>
    <div class="row" style="align-items:center; margin-bottom:8px">
      <span class="muted" style="flex-shrink:0">标题：</span>
      <input v-model="tplEditTitle" aria-label="告警标题模板" style="flex:1" placeholder="LabSentinel 级别{level}告警" />
    </div>
    <textarea v-model="tplEditText" aria-label="告警正文模板"
              style="width:100%; height:96px; font-size:12.5px; line-height:1.5"
              placeholder="{icon} L{level} {type_text} · {ts}&#10;摄像头 #{camera_id}{track_part}&#10;{detail}"></textarea>
    <p class="muted" style="font-size:12px; margin:6px 0">
      可用占位符：{{ Object.keys(tpl.placeholders || {}).join(' / ') }}（正文需含 {detail} 或 {type_text}）</p>
    <div class="row" style="align-items:center">
      <button :disabled="tplBusy" @click="saveTemplate">{{ tplBusy ? '保存中…' : '保存并生效' }}</button>
      <button class="ghost" :disabled="tplBusy" @click="previewTemplate">用当前内容预览</button>
      <button class="ghost" :disabled="tplBusy" @click="resetTemplate">恢复默认</button>
      <button class="ghost" :disabled="tplBusy" @click="loadTemplate">放弃修改</button>
    </div>
    <p v-if="tplMsg" class="muted" style="margin-top:6px">{{ tplMsg }}</p>
    <div v-if="tplPreview" class="muted" style="white-space:pre-wrap; border:1px solid var(--divider); border-radius:6px; padding:8px; margin-top:6px">
      {{ tplPreview }}</div>
      </section>
      <section v-show="show('plugins')" class="panel">
    <h3 style="margin:14px 0 10px">插件（外挂）<span class="muted" style="font-size:12px">（默认禁用）</span></h3>
    <p class="muted" style="margin:0 0 6px">
      插件具备完整代码执行能力，默认禁用，需管理员在此显式启用后才可运行。</p>
    <div class="row" style="margin-bottom:6px">
      <button class="ghost" @click="loadPlugins">刷新插件</button>
      <label class="ghost" style="cursor:pointer">
        上传插件 zip<input type="file" accept=".zip" hidden @change="onUploadZip" />
      </label>
      <span v-if="pluginsError" class="muted">{{ pluginsError }}</span>
    </div>
    <table v-if="plugins.length">
      <tr><th>插件</th><th>版本</th><th>权限</th><th>状态</th><th></th></tr>
      <tr v-for="p in plugins" :key="p.id">
        <td>{{ p.name }}<div class="muted" style="font-size:11px">{{ p.id }}</div></td>
        <td class="muted">{{ p.version }}</td>
        <td>
          <span v-for="perm in (p.permissions || [])" :key="perm"
                :style="perm === 'write' ? 'color:#dc2626; font-weight:600' : ''">{{ perm }}</span>
          <span v-if="!(p.permissions || []).length" class="muted">无</span>
        </td>
        <td :class="p.running ? 'st-ok' : 'muted'">{{ pluginStateLabel(p) }}</td>
        <td style="white-space:nowrap">
          <button v-if="!p.enabled" class="ghost" :disabled="pluginBusy === p.id"
                  @click="pluginAct(p, 'enable')">启用</button>
          <button v-else class="ghost" :disabled="pluginBusy === p.id"
                  @click="pluginAct(p, 'disable')">停用</button>
          <button v-if="p.enabled && !p.running" :disabled="pluginBusy === p.id"
                  @click="pluginAct(p, 'start')">启动</button>
          <button v-if="p.running" class="ghost" :disabled="pluginBusy === p.id"
                  @click="pluginAct(p, 'stop')">停止</button>
          <button class="ghost" :disabled="pluginBusy === p.id"
                  @click="showPluginLog(p)">看日志</button>
        </td>
      </tr>
    </table>
    <p v-else-if="pluginsLoaded && !pluginsError" class="muted" style="margin:6px 0">
      暂无插件——把插件目录（含 plugin.json）放到数据目录 plugins/ 下，点「刷新插件」。</p>
    <p class="muted" style="font-size:12px; margin:6px 0 0">
      含 <b style="color:#dc2626">write</b> 权限的插件启用后将获得管理员令牌；未配置鉴权时插件以无令牌运行（内网使用）。</p>
      </section>
      <section v-show="show('network')" class="panel">
        <h3 style="margin:14px 0 10px">网络与运维</h3>
        <template v-if="net">
          <div class="row" style="align-items:center;flex-wrap:wrap;gap:8px">
            <label class="muted"><input type="checkbox" v-model="net.lan" /> 允许局域网访问</label>
            <label class="muted">端口 <input v-model.number="net.port" type="number" min="1" max="65535" style="width:90px" /></label>
            <label class="muted">信任主机 <input v-model="net.trusted_hosts" placeholder="可选，逗号分隔" style="width:200px" /></label>
            <span class="muted" style="font-size:12px">令牌：{{ net.admin_token_set ? '已设置' : '未设置' }}</span>
            <button class="ghost" :disabled="netBusy" @click="genToken">生成强随机令牌</button>
            <button :disabled="netBusy" @click="saveNetwork">保存网络配置</button>
            <button class="ghost" :disabled="netBusy" @click="restartServer">立即重启生效</button>
          </div>
          <p v-if="newToken" class="muted" style="font-size:12px; word-break:break-all">新令牌：<code>{{ newToken }}</code>（只显示一次，请妥善保存）</p>
          <p class="muted" style="font-size:12px">开启局域网访问前必须设置管理令牌；保存写入安装目录 .env，重启后生效。</p>
        </template>
        <hr style="border:none;border-top:1px solid var(--divider);margin:12px 0" />
        <template v-if="reports">
          <b>定时日报 / 周报</b>
          <div class="row" style="align-items:center;flex-wrap:wrap;gap:8px;margin-top:6px">
            <label class="muted"><input type="checkbox" v-model="reports.daily_enabled" /> 每日
              <input v-model="reports.daily_time" type="time" style="width:110px" /></label>
            <label class="muted"><input type="checkbox" v-model="reports.weekly_enabled" /> 每周
              <select v-model.number="reports.weekly_day" style="width:90px">
                <option :value="1">周一</option><option :value="2">周二</option><option :value="3">周三</option>
                <option :value="4">周四</option><option :value="5">周五</option><option :value="6">周六</option>
                <option :value="7">周日</option>
              </select>
              <input v-model="reports.weekly_time" type="time" style="width:110px" /></label>
            <button :disabled="netBusy" @click="saveReports">保存调度</button>
            <button class="ghost" :disabled="netBusy" @click="sendReportNow">立即发送测试</button>
          </div>
          <p class="muted" style="font-size:12px">复用已配置的告警渠道；同日/同周自动去重。上次日报：{{ reports.last_daily_date || '—' }}。</p>
        </template>
        <template v-if="autostart">
          <div class="row" style="align-items:center;gap:8px;margin-top:8px">
            <b>开机自启</b>
            <span v-if="!autostart.supported" class="muted">（当前平台不支持）</span>
            <template v-else>
              <span :class="autostart.enabled ? 'st-ok' : 'st-idle'">{{ autostart.enabled ? '已开启' : '已关闭' }}</span>
              <button class="ghost" :disabled="netBusy" @click="toggleAutostart">
                {{ autostart.enabled ? '关闭' : '开启' }}</button>
            </template>
          </div>
        </template>
        <hr style="border:none;border-top:1px solid var(--divider);margin:12px 0" />
        <b>只读值守分享链接</b>
        <div class="row" style="align-items:center;flex-wrap:wrap;gap:8px;margin-top:6px">
          <input v-model="shareName" placeholder="链接名称" style="width:160px" />
          <select v-model.number="shareTtl" aria-label="有效期">
            <option :value="24">24 小时</option>
            <option :value="168">7 天</option>
            <option :value="720">30 天</option>
          </select>
          <button :disabled="netBusy" @click="createShare">生成只读链接</button>
          <span v-if="shareToken" class="muted" style="font-size:12px;word-break:break-all">
            链接：<code>{{ shareToken }}</code>（只显示一次）</span>
        </div>
        <table v-if="shares.length" style="margin-top:8px">
          <tr><th>名称</th><th>有效期</th><th>到期</th><th>状态</th><th></th></tr>
          <tr v-for="l in shares" :key="l.id">
            <td>{{ l.name }}</td><td>{{ l.ttl_hours }}h</td><td>{{ l.expires_ts }}</td>
            <td><span :class="l.revoked || l.expired ? 'st-idle' : 'st-ok'">{{ l.revoked ? '已吊销' : (l.expired ? '已过期' : '有效') }}</span></td>
            <td><button v-if="!l.revoked" class="ghost" @click="revokeShare(l.id)">吊销</button></td>
          </tr>
        </table>
        <p v-if="netMsg" class="muted" style="margin-top:8px">{{ netMsg }}</p>
      </section>

      <section v-show="show('service')" class="panel">
    <h3 style="margin:14px 0 10px">服务控制</h3>
    <div class="row" style="align-items:center">
      <button class="ghost" @click="shutdownServer">停止服务…</button>
      <button class="ghost" :disabled="serverLogBusy" @click="loadServerLog">
        {{ serverLogBusy ? '读取中…' : '查看服务日志（尾部 200 行）' }}</button>
      <span class="muted">停止后摄像头分析与事件推送将暂停；重新启动程序后恢复。</span>
    </div>
    <div class="row" style="align-items:center;margin-top:8px">
      <a :href="api.diagnosticsBundleUrl()" download>
        <button class="ghost">导出诊断包（脱敏 ZIP）</button>
      </a>
      <span class="muted" style="font-size:12px">含版本/系统/GPU/工具探测/脱敏配置/近 2 天日志，不含任何密钥。</span>
    </div>
    <!-- LS-051：go2rtc 流媒体托管状态 -->
    <div v-if="go2rtc" class="row" style="align-items:center; margin-top:10px; flex-wrap:wrap">
      <b>实时画面服务（go2rtc）</b>
      <span :class="go2rtc.running ? 'st-ok' : 'st-idle'">
        {{ go2rtc.running ? `运行中（PID ${go2rtc.pid}）` : (go2rtc.enabled ? '未运行' : '已关闭') }}</span>
      <button class="ghost" :disabled="go2rtcBusy" @click="toggleGo2rtc">
        {{ go2rtc.enabled ? '关闭托管' : '开启托管' }}</button>
      <span v-if="go2rtc.last_error" class="muted" style="font-size:12px">{{ go2rtc.last_error }}</span>
    </div>
    <div v-if="serverLog" class="muted" style="white-space:pre-wrap; border:1px solid var(--divider); border-radius:6px; padding:8px; margin-top:8px; max-height:260px; overflow:auto; font-size:12px">{{ serverLog.file ? `[${serverLog.file}]\n` : '' }}{{ serverLog.lines }}</div>
      </section>
    </div>
  </div>
  <div v-if="pluginLog" style="position:fixed; inset:0; background:rgba(0,0,0,.45); display:flex; align-items:center; justify-content:center; z-index:60; -webkit-backdrop-filter:blur(6px); backdrop-filter:blur(6px)" @click.self="pluginLog = null">
    <div style="background:var(--dialog-bg); color:var(--text-primary); border:1px solid var(--stroke-default); box-shadow:var(--shadow-dialog); border-radius:var(--overlay-radius); padding:14px; width:min(760px, 92vw); max-height:80vh; overflow:auto">
      <div style="display:flex; align-items:center; justify-content:space-between">
        <b>插件日志 · {{ pluginLog.id }}</b>
        <button class="ghost" @click="pluginLog = null">关闭</button>
      </div>
      <pre style="white-space:pre-wrap; font-size:12px; margin-top:8px">{{ pluginLog.text }}</pre>
    </div>
  </div>
  <!-- LS-054：样本标注弹窗 -->
  <AnnotatorModal v-if="annotator" :sample="annotator.sample" :image-url="annotator.imageUrl"
                  @close="annotator = null" @saved="annotator = null; loadPool()" />
</template>

<style scoped>
/* 设置中心：左侧分类导航（WinUI NavigationView 风格）+ 右侧内容卡片 */
.settings-layout { display: flex; gap: 16px; align-items: flex-start; }
.settings-nav {
  width: 200px; flex-shrink: 0; position: sticky; top: 16px;
  background: var(--card-bg); border: 1px solid var(--stroke-default);
  border-radius: var(--overlay-radius); padding: 10px;
}
.settings-search { width: 100%; margin: 0 0 8px; }
.settings-item {
  display: flex; align-items: center; gap: 10px; min-height: 36px; padding: 0 10px;
  border-radius: var(--control-radius); color: var(--text-secondary);
  cursor: pointer; font-size: 13px; position: relative;
  transition: background-color var(--fast-duration) var(--fast-out-slow-in),
              color var(--fast-duration) var(--fast-out-slow-in);
}
.settings-item:hover { background: var(--subtle-hover); color: var(--text-primary); }
.settings-item.active { background: var(--ctrl-fill); color: var(--text-primary); font-weight: 500; }
.settings-item.active::before {
  content: ""; position: absolute; left: -10px; top: 50%;
  width: 3px; height: 16px; transform: translateY(-50%);
  border-radius: 2px; background: var(--accent);
}
.settings-item svg { width: 15px; height: 15px; flex-shrink: 0; opacity: 0.9; }
.settings-empty { padding: 6px 10px; font-size: 12px; }
.settings-body { flex: 1; min-width: 0; }
.settings-body .panel:last-child { margin-bottom: 0; }

@media (max-width: 760px) {
  .settings-layout { flex-direction: column; }
  .settings-nav { width: 100%; position: static; }
  .settings-item.active::before { display: none; }
}
</style>
