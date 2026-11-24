<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import { api } from '../api'

const props = defineProps({ routeQuery: { type: String, default: '' } })
const mode = ref('chat')
const cameras = ref([])
const sessions = ref([])
const reports = ref([])
const session = ref(null)
const report = ref(null)
const baseline = ref(null)
const history = ref([])
const scopeCamera = ref('')
const reportCamera = ref('')
const eventFromLink = ref(null)
const question = ref('')
const firstImages = ref(false)
const reportImages = ref(false)
const period = ref('week')
const startDate = ref('')
const endDate = ref('')
const archived = ref(false)
const nextSessions = ref(false)
const nextMessages = ref(false)
const nextReports = ref(false)
const busy = ref(false)
const error = ref('')
const notice = ref('')
const roleProfiles = ref([])
const roles = ref({})
const roleLabels = {
  verifier: '事件复核', shift_summary: '安全报告', chat: '安全问答', embeddings: '向量模型',
}
const periods = { day: '日报', week: '周报', month: '月报', custom: '自定义报告' }
const selectedCamera = computed(() => cameras.value.find(camera => Number(camera.id) === Number(session.value?.scope?.camera_id))?.name)
const canAsk = computed(() => !busy.value && Boolean(question.value.trim()))
let detailGeneration = 0
let reportGeneration = 0

function requestId() {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), byte => byte.toString(16).padStart(2, '0')).join('')
}
function array(value) { return Array.isArray(value) ? value : [] }
function validRow(value) { return value && Number.isSafeInteger(value.id) && value.id > 0 }
function cameraValue(value) { return Number.isSafeInteger(Number(value)) && Number(value) > 0 ? Number(value) : null }
function periodText(value) { return periods[value] || '安全报告' }
function markEvidence(message) {
  if (message?.evidence_mode === 'images') return '记录和原图'
  return message?.model ? '仅记录' : '本地记录摘要'
}
function count(source, field) {
  const value = source?.facts?.counts?.[field]
  return Number.isSafeInteger(value) ? value : '—'
}
function rate(source) {
  const value = source?.facts?.compliance?.rate
  return typeof value === 'number' && Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : '无有效抽样'
}
function compare(field) {
  const current = count(report.value, field)
  const previous = count(baseline.value, field)
  if (typeof current !== 'number' || typeof previous !== 'number') return '—'
  const change = current - previous
  return `${change > 0 ? '+' : ''}${change}`
}
function setupLink() {
  const q = new URLSearchParams(props.routeQuery)
  const id = cameraValue(q.get('event'))
  eventFromLink.value = id
  if (!id) return
  newChat()
  mode.value = 'chat'
  scopeCamera.value = cameraValue(q.get('camera')) || ''
  question.value = `请根据已有记录和可见证据复核事件 #${id}`
}
function newChat() {
  detailGeneration++
  session.value = null
  history.value = []
  question.value = ''
  scopeCamera.value = ''
  firstImages.value = false
  nextMessages.value = false
  error.value = ''
}
async function attempt(work) {
  if (busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try { await work() }
  catch (err) { error.value = err.message }
  finally { busy.value = false }
}
async function loadSessions(append = false) {
  const data = array(await api.aiSessions(append ? sessions.value.length : 0)).filter(validRow)
  nextSessions.value = data.length === 50
  sessions.value = append ? [...sessions.value, ...data] : data
}
async function loadReports(append = false) {
  const generation = ++reportGeneration
  const data = array(await api.aiReports(archived.value, append ? reports.value.length : 0)).filter(validRow)
  if (generation !== reportGeneration) return
  nextReports.value = data.length === 50
  reports.value = append ? [...reports.value, ...data] : data
}
async function showSession(id, append = false) {
  const generation = ++detailGeneration
  const data = await api.aiSession(id, append ? history.value.length : 0)
  if (generation !== detailGeneration) return
  if (!validRow(data?.session) || !Array.isArray(data?.messages)) throw new Error('会话返回格式无效')
  session.value = data.session
  scopeCamera.value = data.session.scope?.camera_id ?? ''
  history.value = append ? [...data.messages, ...history.value] : data.messages
  nextMessages.value = data.messages.length === 50
}
async function send() {
  const input = question.value.trim()
  if (!input) return
  await attempt(async () => {
    if (!session.value) {
      session.value = await api.aiOpenSession({ title: Array.from(input).slice(0, 38).join(''), scope: { camera_id: cameraValue(scopeCamera.value) } })
      if (!validRow(session.value) || !Number.isSafeInteger(session.value.revision)) throw new Error('无法建立会话')
    }
    const id = session.value.id
    const revision = session.value.revision
    const result = await api.aiAsk(id, { request_id: requestId(), revision, question: input, include_images: revision === 0 && firstImages.value })
    if (typeof result?.answer !== 'string' || !Number.isSafeInteger(result.revision)) throw new Error('会话返回格式无效')
    session.value = { ...session.value, revision: result.revision }
    history.value = [...history.value, result]
    question.value = ''
    await loadSessions()
  })
}
async function buildReport() {
  await attempt(async () => {
    const scope = { camera_id: cameraValue(reportCamera.value) }
    if (period.value === 'custom') Object.assign(scope, { start_date: startDate.value, end_date: endDate.value })
    const value = await api.aiGenerateReport({ request_id: requestId(), period: period.value, scope, include_images: reportImages.value })
    if (!validRow(value)) throw new Error('报告返回格式无效')
    if (archived.value) archived.value = false
    report.value = value
    await loadReports()
  })
}
async function toggleArchive() {
  if (!report.value) return
  await attempt(async () => {
    const isArchived = Boolean(report.value.archived_at)
    await api.aiArchiveReport(report.value.id, !isArchived)
    notice.value = isArchived ? '报告已移回当前列表' : '报告已归档'
    report.value = null
    await loadReports()
  })
}
function printReport() {
  if (!report.value) return
  const source = report.value
  const tab = window.open('', '_blank')
  if (!tab) { error.value = '浏览器拦截了打印窗口，请允许弹出窗口后重试'; return }
  tab.opener = null
  const doc = tab.document
  doc.title = `LabSentinel 安全报告 ${source.id}`
  const styles = doc.createElement('style')
  styles.textContent = 'body{font:15px/1.7 system-ui,sans-serif;color:#111;background:#fff;margin:2rem auto;max-width:46rem;padding:0 1rem} p{white-space:pre-wrap} h1,h2{line-height:1.2} @media print {body{margin:0 auto}}'
  doc.head.append(styles)
  const paragraphs = [
    ['h1', `${periodText(source.period)} #${source.id}`],
    ['p', `${source.scope?.start_date || ''} 至 ${source.scope?.end_date || ''}`],
    ['h2', '本地统计'],
    ['p', `事件 ${count(source, 'events')} 条 · 告警 ${count(source, 'alerts')} 条 · 检查 ${count(source, 'checks')} 条 · 未处置 ${count(source, 'unhandled_alerts')} 条\n穿戴抽样合规率 ${rate(source)}`],
    ['h2', source.model ? `AI 解读 · ${markEvidence(source)}` : '记录摘要'],
    ['p', String(source.answer || '')],
  ]
  for (const [tag, content] of paragraphs) {
    const node = doc.createElement(tag)
    node.textContent = content
    doc.body.append(node)
  }
  tab.focus()
  tab.print()
}
async function loadRoles() {
  const data = await api.aiRoles()
  roleProfiles.value = array(data?.profiles).filter(item => typeof item?.id === 'string')
  roles.value = Object.fromEntries(Object.keys(roleLabels).map(key => [key, typeof data?.roles?.[key] === 'string' ? data.roles[key] : '']))
}
async function selectMode(value) {
  mode.value = value
  if (value === 'reports') await attempt(loadReports)
  if (value === 'roles') await attempt(loadRoles)
}
async function saveRoles() {
  await attempt(async () => {
    await api.aiSaveRoles(Object.fromEntries(Object.entries(roles.value).filter(([, id]) => id)))
    notice.value = '模型分配已保存'
  })
}
async function testEmbeddings() {
  await attempt(async () => {
    const result = await api.aiTestEmbeddings()
    if (result?.ok !== true || !Number.isSafeInteger(result.dimensions)) throw new Error('向量连接测试未返回有效结果')
    notice.value = `${result.model} 已返回 ${result.dimensions} 维向量`
  })
}
watch(() => props.routeQuery, setupLink)
async function filterArchive() {
  report.value = null
  await attempt(loadReports)
}
onMounted(async () => {
  setupLink()
  await attempt(async () => {
    const outcomes = await Promise.allSettled([loadSessions(), api.cameras().then(value => { cameras.value = array(value).filter(validRow) })])
    const failed = outcomes.find(result => result.status === 'rejected')
    if (failed) throw failed.reason
  })
})
</script>

<template>
  <div class="assistant">
    <h2>AI 安全助手</h2>
    <div class="row mode" role="group" aria-label="安全助手功能">
      <button type="button" :disabled="busy" :class="{ ghost: mode !== 'chat' }" :aria-pressed="mode === 'chat'" @click="selectMode('chat')">安全问答</button>
      <button type="button" :disabled="busy" :class="{ ghost: mode !== 'reports' }" :aria-pressed="mode === 'reports'" @click="selectMode('reports')">安全报告</button>
      <button type="button" :disabled="busy" :class="{ ghost: mode !== 'roles' }" :aria-pressed="mode === 'roles'" @click="selectMode('roles')">模型分配</button>
    </div>
    <p v-if="error" role="alert" class="status-error">{{ error }}</p>
    <p v-if="notice" role="status">{{ notice }}</p>
    <section v-if="mode === 'chat'" class="workspace">
      <aside class="card history" aria-label="会话历史">
        <button type="button" class="ghost" :disabled="busy" @click="newChat">新建会话</button>
        <p v-if="!sessions.length" class="muted">暂无会话</p>
        <button v-for="item in sessions" :key="item.id" type="button" class="ghost history-item" :aria-current="session?.id === item.id" :disabled="busy" @click="attempt(() => showSession(item.id))">
          <span>{{ item.title }}</span><small class="muted">{{ item.updated_at?.replace('T', ' ') }}</small>
        </button>
        <button v-if="nextSessions" class="ghost" :disabled="busy" @click="attempt(() => loadSessions(true))">更多会话</button>
      </aside>
      <div class="card panel">
        <h3>{{ session?.title || '新会话' }}</h3>
        <label v-if="!session" class="row">机位
          <select v-model="scopeCamera" :disabled="busy"><option value="">全部机位</option><option v-for="camera in cameras" :key="camera.id" :value="camera.id">{{ camera.name }}</option></select>
        </label>
        <p v-else class="muted">{{ selectedCamera || (session?.scope?.camera_id ? `机位 #${session.scope.camera_id}` : '全部机位') }}</p>
        <button v-if="nextMessages" type="button" class="ghost" :disabled="busy" @click="attempt(() => showSession(session.id, true))">更早的对话</button>
        <div v-for="item in history" :key="item.request_id" class="exchange">
          <h4>{{ item.question }}</h4>
          <p class="muted">{{ markEvidence(item) }}<template v-if="item.model"> · {{ item.model }}</template></p>
          <p v-if="item.facts?.event?.id" class="muted"><a :href="`#/events?event=${item.facts.event.id}`">打开事件 #{{ item.facts.event.id }}</a></p>
          <p v-else class="muted">{{ item.facts?.scope?.start_date }} 至 {{ item.facts?.scope?.end_date }} · {{ count(item, 'alerts') }} 条告警</p>
          <p class="answer">{{ item.answer }}</p>
          <div v-if="array(item.facts?.image_event_ids).length" class="row evidence">
            <a v-for="id in item.facts.image_event_ids" :key="id" :href="`#/events?event=${id}`">证据 #{{ id }}</a>
          </div>
        </div>
        <form class="ask" @submit.prevent="send">
          <label for="safety-question">问题</label>
          <textarea id="safety-question" v-model="question" rows="3" maxlength="1000" :disabled="busy" placeholder="例如：这周哪天未戴口罩的告警最多？" />
          <label v-if="!session?.revision" class="checkbox"><input v-model="firstImages" type="checkbox" :disabled="busy">首轮附最多 4 张原图</label>
          <p v-else class="muted hint">后续追问使用事件记录和对话历史。</p>
          <p v-if="eventFromLink && !session" class="muted hint">正在追问事件 #{{ eventFromLink }}</p>
          <div class="row"><button type="submit" :disabled="!canAsk">{{ busy ? '处理中…' : '发送' }}</button></div>
        </form>
      </div>
    </section>
    <section v-else-if="mode === 'reports'" class="workspace">
      <aside class="card history" aria-label="报告历史">
        <label class="checkbox"><input v-model="archived" type="checkbox" :disabled="busy" @change="filterArchive">查看已归档</label>
        <p v-if="!reports.length" class="muted">暂无报告</p>
        <button v-for="item in reports" :key="item.id" type="button" class="ghost history-item" :aria-current="report?.id === item.id" @click="report = item">
          <span>{{ periodText(item.period) }} #{{ item.id }}</span><small class="muted">{{ item.scope?.start_date }} 至 {{ item.scope?.end_date }}</small>
        </button>
        <button v-if="nextReports" class="ghost" :disabled="busy" @click="attempt(() => loadReports(true))">更多报告</button>
      </aside>
      <div class="card panel">
        <form class="report-form" @submit.prevent="buildReport">
          <h3>生成报告</h3>
          <div class="row">
            <label>时段 <select v-model="period" :disabled="busy"><option v-for="(label, key) in periods" :key="key" :value="key">{{ label }}</option></select></label>
            <label>机位 <select v-model="reportCamera" :disabled="busy"><option value="">全部机位</option><option v-for="camera in cameras" :key="camera.id" :value="camera.id">{{ camera.name }}</option></select></label>
          </div>
          <div v-if="period === 'custom'" class="row">
            <label>开始日期 <input v-model="startDate" type="date" required :disabled="busy"></label>
            <label>结束日期 <input v-model="endDate" type="date" required :min="startDate || undefined" :disabled="busy"></label>
          </div>
          <label class="checkbox"><input v-model="reportImages" type="checkbox" :disabled="busy">附最多 4 张代表性原图</label>
          <div class="row"><button type="submit" :disabled="busy">{{ busy ? '处理中…' : '生成报告' }}</button></div>
        </form>
        <article v-if="report" class="report">
          <h3>{{ periodText(report.period) }} #{{ report.id }}</h3>
          <p class="muted">{{ report.scope?.start_date }} 至 {{ report.scope?.end_date }} · {{ markEvidence(report) }}<template v-if="report.model"> · {{ report.model }}</template></p>
          <dl class="facts">
            <div><dt>事件</dt><dd>{{ count(report, 'events') }}</dd></div>
            <div><dt>告警</dt><dd>{{ count(report, 'alerts') }}</dd></div>
            <div><dt>检查</dt><dd>{{ count(report, 'checks') }}</dd></div>
            <div><dt>未处置告警</dt><dd>{{ count(report, 'unhandled_alerts') }}</dd></div>
            <div><dt>穿戴抽样合规率</dt><dd>{{ rate(report) }}</dd></div>
          </dl>
          <p v-if="baseline && baseline.id !== report.id" class="muted">
            相比报告 #{{ baseline.id }}（{{ baseline.scope?.start_date }} 至 {{ baseline.scope?.end_date }}），事件 {{ compare('events') }} 条、告警 {{ compare('alerts') }} 条、待处置 {{ compare('unhandled_alerts') }} 条。
          </p>
          <h4>{{ report.model ? 'AI 解读' : '记录摘要' }}</h4>
          <p class="answer">{{ report.answer }}</p>
          <div v-if="array(report.facts?.image_event_ids).length" class="row evidence">
            <a v-for="id in report.facts.image_event_ids" :key="id" :href="`#/events?event=${id}`">证据 #{{ id }}</a>
          </div>
          <div class="row">
            <button class="ghost" :disabled="busy" @click="attempt(() => api.aiExportReport(report.id))">导出 Markdown</button>
            <button class="ghost" :disabled="busy" @click="printReport">打印 / 保存 PDF</button>
            <button class="ghost" :disabled="busy" @click="baseline = baseline?.id === report.id ? null : report">{{ baseline?.id === report.id ? '取消对比基准' : '用作对比基准' }}</button>
            <button class="ghost" :disabled="busy" @click="toggleArchive">{{ report.archived_at ? '取消归档' : '归档' }}</button>
          </div>
        </article>
      </div>
    </section>
    <section v-else class="card panel" aria-label="模型分配">
      <p v-if="!roleProfiles.length" class="muted">在「设置 → 检测模式」添加接入档案后，可在这里分配模型。</p>
      <div v-for="(label, key) in roleLabels" :key="key" class="role-row">
        <label :for="`safety-role-${key}`">{{ label }}</label>
        <select :id="`safety-role-${key}`" v-model="roles[key]" :disabled="busy">
          <option value="">跟随当前接入</option><option value="off">关闭</option>
          <option v-for="profile in roleProfiles" :key="profile.id" :value="profile.id">{{ profile.name }} · {{ profile.model }}</option>
        </select>
      </div>
      <div class="row">
        <button type="button" :disabled="busy" @click="saveRoles">保存模型分配</button>
        <button type="button" class="ghost" :disabled="busy || roles.embeddings === 'off'" title="使用已保存的接入档案，发送一条测试文本" @click="testEmbeddings">测试向量模型</button>
      </div>
    </section>
  </div>
</template>

<style scoped>
.assistant { display: grid; gap: 12px; align-content: start; }
.assistant :is(h2, h3, h4, p, dl) { margin: 0; }
.mode { padding-bottom: 4px; }
.workspace { display: grid; grid-template-columns: minmax(175px, 245px) minmax(0, 1fr); gap: 12px; align-items: start; }
.history, .panel { display: grid; align-content: start; gap: 12px; min-width: 0; }
.history-item { display: grid; gap: 2px; text-align: left; overflow-wrap: anywhere; }
.history-item[aria-current="true"] { background: var(--subtle-hover); border-color: var(--accent-strong); }
.exchange, .report { display: grid; gap: 10px; border-top: 1px solid var(--stroke-secondary); padding-top: 14px; }
.answer { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.6; }
.ask, .report-form { display: grid; gap: 12px; }
.ask textarea { width: 100%; resize: vertical; }
.checkbox { display: inline-flex; align-items: center; gap: 8px; }
.checkbox input { width: auto; margin: 0; }
.hint { font-size: .9em; }
.facts { display: grid; gap: 8px; grid-template-columns: repeat(auto-fit, minmax(110px, 1fr)); }
.facts div { background: var(--card-bg-secondary); padding: 9px; border-radius: 6px; border: 1px solid var(--stroke-default); }
.facts dd { margin: 2px 0 0; font-weight: 600; }
.role-row { display: grid; grid-template-columns: minmax(86px, 1fr) minmax(150px, 3fr); gap: 8px; align-items: center; max-width: 550px; }
.evidence a, .exchange a { color: var(--accent-strong); }
@media (max-width: 760px) { .workspace { grid-template-columns: 1fr; } .history { grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); } }
</style>
