<script setup>
// RA5-UX（ADR-0011）：设置中心——从「摄像头」页拆出的管理面（分类侧栏 + 搜索）。
// 各区块整段搬移自 Cameras.vue（逻辑零改动）。
import { computed, onMounted, ref } from 'vue'
import { api, adminToken } from '../api'

const search = ref('')
const active = ref('token')
const sections = [
  { id: 'token', label: '管理令牌', kw: '令牌 token 鉴权 认证', icon: '<circle cx="5" cy="8" r="3"/><path d="M7.6 8H14v2.2M12 8v2.2"/>' },
  { id: 'notify', label: '通知与告警', kw: '渠道 测试 静默 投递 失败 告警 企业微信 钉钉 邮件', icon: '<path d="M8 2.5a3.5 3.5 0 0 1 3.5 3.5v3l1.5 2.5H3l1.5-2.5v-3A3.5 3.5 0 0 1 8 2.5z"/><path d="M6.5 13a1.8 1.8 0 0 0 3 0"/>' },
  { id: 'data', label: '数据管理', kw: '导入 导出 备份 配置 json 迁移', icon: '<path d="M3 5.5C3 4.1 5.2 3 8 3s5 1.1 5 2.5S10.8 8 8 8 3 6.9 3 5.5z"/><path d="M3 5.5v5C3 11.9 5.2 13 8 13s5-1.1 5-2.5v-5"/>' },
  { id: 'template', label: '告警文案', kw: '模板 预览 占位符 文案 标题 正文', icon: '<path d="M3 4h10M3 8h10M3 12h6"/>' },
  { id: 'plugins', label: '插件', kw: '插件 外挂 zip 上传 启用 日志 扩展', icon: '<path d="M6 2.5h4v2.2a1.6 1.6 0 1 0 2.2 2.2H14v4h-1.8a1.6 1.6 0 1 1 0 2.2H14v1.4H2v-1.4h1.8a1.6 1.6 0 1 0-2.2-2.2H2v-4h2.2A1.6 1.6 0 1 0 6 4.7z"/>' },
  { id: 'service', label: '服务控制', kw: '停止 服务 重启 退出 关机', icon: '<path d="M8 2.5v5.5"/><path d="M4.6 4.8a4.8 4.8 0 1 0 6.8 0"/>' },
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
onMounted(() => { loadAlerts(); loadPlugins(); loadCams() })
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
      <section v-show="show('token')" class="panel">
    <h3 style="margin-bottom:10px">管理令牌</h3>
    <div class="row">
      <input v-model="tokenInput" placeholder="服务端启用鉴权时填写（LS_ADMIN_TOKEN）" aria-label="管理令牌" style="flex:1" />
      <button @click="saveToken">保存令牌</button>
      <span v-if="tokenMsg" class="muted">{{ tokenMsg }}</span>
    </div>
    <p class="muted" style="margin-top:4px">令牌仅保存在本浏览器；服务端未启用鉴权时无需填写。</p>
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
      </section>
      <section v-show="show('template')" class="panel">
    <h3 style="margin:14px 0 10px">告警文案模板</h3>
    <div class="row" style="align-items:center">
      <button class="ghost" :disabled="tplBusy" @click="loadTemplate">
        {{ tplBusy ? '加载中…' : '加载当前模板' }}</button>
      <button class="ghost" :disabled="tplBusy || !tpl.text" @click="previewTemplate">预览</button>
      <span class="muted">模板通过配置文件修改，重启后生效</span>
    </div>
    <textarea v-if="tpl.text" readonly class="muted"
              style="width:100%; height:80px; margin-top:6px; font-size:12px">{{ tpl.text }}</textarea>
    <p v-if="tpl.text" class="muted" style="font-size:12px">
      占位符：{{ Object.keys(tpl.placeholders || {}).join(' / ') }}</p>
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
      <section v-show="show('service')" class="panel">
    <h3 style="margin:14px 0 10px">服务控制</h3>
    <div class="row" style="align-items:center">
      <button class="ghost" @click="shutdownServer">停止服务…</button>
      <span class="muted">停止后摄像头分析与事件推送将暂停；重新启动程序后恢复。</span>
    </div>
      </section>
    </div>
  </div>
  <div v-if="pluginLog" style="position:fixed; inset:0; background:rgba(0,0,0,.7); display:flex; align-items:center; justify-content:center; z-index:60" @click.self="pluginLog = null">
    <div style="background:var(--panel, #16202e); color:inherit; border:1px solid var(--divider); border-radius:8px; padding:12px; width:min(760px, 92vw); max-height:80vh; overflow:auto">
      <div style="display:flex; align-items:center; justify-content:space-between">
        <b>插件日志 · {{ pluginLog.id }}</b>
        <button class="ghost" @click="pluginLog = null">关闭</button>
      </div>
      <pre style="white-space:pre-wrap; font-size:12px; margin-top:8px">{{ pluginLog.text }}</pre>
    </div>
  </div>
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
