<script setup>
import { computed, onMounted, onUnmounted, nextTick, ref, watch } from 'vue'
import { api, CHANNEL_NAMES, TYPE_TEXT, STATUS_TEXT, mediaUrl } from '../api'
import AnnotatorModal from '../components/AnnotatorModal.vue'
import AsyncState from '../components/AsyncState.vue'
import SnapshotOverlay from '../components/SnapshotOverlay.vue'
import EventExplanation from '../components/EventExplanation.vue'
import SuppressionReview from '../components/SuppressionReview.vue'

const rows = ref([])
const camId = ref('')
const level = ref('')
const category = ref('')
const status = ref('')
const cams = ref([])
const props = defineProps({ routeQuery: { type: String, default: '' } })
const zoom = ref(null)
const videoZoom = ref(null)
// 任务19：分页 + 时间范围
const PAGE_SIZE = 50
const offset = ref(0)
const total = ref(0)
const startDate = ref('')
const endDate = ref('')
const hourFilter = ref(null)  // 任务136：时间线点击的小时（{date:'YYYY-MM-DD',hour:N}）
const pad2 = n => String(n).padStart(2, '0')
const q = ref('')            // 任务84 关键词
const aiVerdict = ref('')    // 任务132：AI 复核筛选（likely_true|likely_false|uncertain|none）
// 任务136：事件时间线（div grid 热力条，无图表库）
const tl = ref(null)
const tlOpen = ref(false)
const tlLoading = ref(false)

function tlColor(h) {
  if (!h.total) return 'var(--divider, #2a3648)'
  if (h.total >= 10) return '#dc2626'
  if (h.total >= 3) return '#b45309'
  return '#2563eb'
}
async function loadTimeline() {
  tlLoading.value = true
  try {
    tl.value = await api.eventsTimeline({ days: 7 })
  } catch { /* 静默：时间线是增强，不影响表格 */ }
  tlLoading.value = false
}
function onToggleTimeline(e) {
  tlOpen.value = e.target.open
  if (e.target.open && !tl.value?.days_list) loadTimeline()
}
function jumpHour(d, h) {
  if (!h.total) return
  const base = d.date
  startDate.value = base
  endDate.value = base
  hourFilter.value = { date: base, hour: h.hour }
  preset.value = ''
  resetAndLoad({ keepHour: true })
}
const preset = ref('')        // 日期预设
// 任务132：AI 复核徽标（仅供参考，不改处置语义）
const AI_BADGE = { likely_false: { text: 'AI: 疑似误报', cls: 'ai-warn' },
                   likely_true: { text: 'AI: 疑似真实', cls: 'ai-ok' },
                   uncertain: { text: 'AI: 不确定', cls: 'ai-mute' } }
function aiBadge(ev) {
  const v = ev.ai_verdict
  return v && AI_BADGE[v] ? AI_BADGE[v] : null
}
function aiTitle(ev) {
  const r = ev.ai_review
  if (!r) return 'VLM 复核结果（仅供参考，不改变处置状态）'
  return `模型 ${r.reason ? '· ' + r.reason : ''} 置信 ${r.confidence ?? '-'}`
    .replace('模型 ', '模型') + (ev.ai_review_model ? ` · ${ev.ai_review_model}` : '')
}
const drawer = ref(null)        // 详情抽屉：当前事件对象
const previewUnavailable = ref(false)
const canPreview = event => event.has_snapshot || event.type === 'photo_check'
watch(() => drawer.value?.id, () => {
  previewUnavailable.value = false
  poolMsg.value = ''
})
const drawerEl = ref(null)      // 抽屉 DOM（焦点管理）
const faceMatch = ref(null)
const notes = ref([])
const notesLoading = ref(false)
const notesError = ref('')
const note = ref('')
const noteSaving = ref(false)
const deliveriesOpen = ref(false)
const deliveries = ref([])
const deliveryError = ref('')
const deliveryLoading = ref(false)
const deliveryReload = ref(0)
const retrying = ref(null)
const actionNotice = ref('')
const DELIVERY_LABELS = { ok: '已送达', pending: '等待发送', retry: '等待重试', failed: '失败', dead_letter: '重试结束' }
const actorLabel = actor => actor === 'admin' ? '管理员' : actor === 'anonymous(auth-off)' ? '本机操作' : (actor || '—').replace(/^device:/, '')

watch(() => drawer.value?.id, async (id, _, onCleanup) => {
  notes.value = []
  notesError.value = ''
  note.value = ''
  deliveriesOpen.value = false
  if (!Number.isInteger(id)) return
  const request = new AbortController()
  onCleanup(() => request.abort())
  notesLoading.value = true
  try {
    const data = await api.eventNotes(id, request.signal)
    if (!request.signal.aborted) notes.value = Array.isArray(data) ? data : []
  } catch (err) {
    if (!request.signal.aborted) notesError.value = err.message
  } finally {
    if (!request.signal.aborted) notesLoading.value = false
  }
})

watch([() => drawer.value?.id, deliveriesOpen, deliveryReload], async ([id, open], _, onCleanup) => {
  deliveries.value = []
  deliveryError.value = ''
  if (!open || !Number.isInteger(id)) return
  const request = new AbortController()
  onCleanup(() => request.abort())
  deliveryLoading.value = true
  try {
    const data = await api.alertDeliveries({ event_id: id }, request.signal)
    if (!request.signal.aborted) deliveries.value = Array.isArray(data) ? data : []
  } catch (err) {
    if (!request.signal.aborted) deliveryError.value = err.message
  } finally {
    if (!request.signal.aborted) deliveryLoading.value = false
  }
})

async function saveNote() {
  const id = drawer.value?.id
  const text = note.value.trim()
  if (!Number.isInteger(id) || !text || notesLoading.value || noteSaving.value) return
  noteSaving.value = true
  notesError.value = ''
  try {
    const added = await api.addEventNote(id, text)
    if (drawer.value?.id === id && Number.isInteger(added?.id)) {
      notes.value = [added, ...notes.value].slice(0, 100)
      note.value = ''
    }
  } catch (err) {
    if (drawer.value?.id === id) notesError.value = err.message
  } finally { noteSaving.value = false }
}

async function retryDelivery(id) {
  const event = drawer.value?.id
  if (retrying.value !== null) return
  retrying.value = id
  deliveryError.value = ''
  try {
    await api.alertDeliveryRetry(id)
    if (drawer.value?.id === event) deliveryReload.value++
  } catch (err) {
    if (drawer.value?.id === event) deliveryError.value = err.message
  } finally { retrying.value = null }
}
watch(() => drawer.value?.id, async (id, _, onCleanup) => {
  faceMatch.value = null
  if (!Number.isInteger(id)) return
  const request = new AbortController()
  onCleanup(() => request.abort())
  try {
    const data = await api.faceEventMatch(id, request.signal)
    if (!request.signal.aborted && typeof data?.matched?.subject === 'string') {
      faceMatch.value = data.matched
    }
  } catch { /* 无身份记录或当前会话无管理权限 */ }
})
let lastFocus = null
// 任务66：三态 + 按钮防重复
const loading = ref(true)
const loaded = ref(false)
const stale = ref(false)      // 有旧数据时的刷新失败（只提示，不进错误分支）
const error = ref('')
const busyEvent = ref(null)   // 正在处置的事件 id
let loadSeq = 0               // 2026-10-30 审阅：请求序号，防旧响应覆盖新结果
// 任务83：批量处置
const selected = ref([])
const bulkBusy = ref(false)
const newRowsCount = computed(() => rows.value.filter(e => e.status === 'new').length)

function toggleSel(id, on) {
  if (on && !selected.value.includes(id)) selected.value = [...selected.value, id]
  if (!on) selected.value = selected.value.filter(x => x !== id)
}
function toggleAll(on) {
  const ids = rows.value.filter(e => e.status === 'new').map(e => e.id)
  selected.value = on ? ids : selected.value.filter(x => !ids.includes(x))
}
async function bulk(status) {
  if (!selected.value.length || bulkBusy.value) return
  if (status === 'false_positive' &&
      !confirm(`把 ${selected.value.length} 条标记为误报？\n截图会复制到标注池（用于模型改进），事件保留。`)) return
  bulkBusy.value = true
  try {
    const r = await api.bulkStatus({ ids: selected.value, status })
    const missing = Array.isArray(r?.missing) ? r.missing : []
    const failed = Array.isArray(r?.failed) ? r.failed : []
    alert(`批量处置完成：更新 ${r?.updated ?? 0}，跳过 ${r?.skipped ?? 0}` +
          (missing.length ? `，不存在 ${missing.length}` : '') +
          (failed.length ? `，失败 ${failed.length}` : '') +
          (status === 'false_positive' ? `。标注池已有 ${r?.samples_saved ?? 0} 张，无法加入 ${r?.samples_unavailable ?? 0} 张` : ''))
    selected.value = []
    load()
  } catch (e) { alert(String(e.message || e)) }
  bulkBusy.value = false
}

async function load() {
  const seq = ++loadSeq  // 2026-10-30 审阅：快速切换筛选时旧响应可能后到覆盖新结果
  const params = { limit: PAGE_SIZE, offset: offset.value }
  if (camId.value) params.camera_id = camId.value
  if (level.value) params.level = level.value
  if (category.value) params.category = category.value
  if (status.value) params.status = status.value
  if (preset.value === 'today') params.today = 1
  else if (hourFilter.value) {
    const hf = hourFilter.value
    params.start_ts = `${hf.date}T${pad2(hf.hour)}:00:00`
    params.end_ts = `${hf.date}T${pad2(hf.hour)}:59:59`
  } else {
    if (startDate.value) params.start_ts = startDate.value
    if (endDate.value) params.end_ts = endDate.value
  }
  if (q.value) params.q = q.value
  if (aiVerdict.value) params.ai_verdict = aiVerdict.value
  try {
    const [newRows, count] = await Promise.all([api.events(params), api.eventsCount(params)])
    const newTotal = count?.count
    if (seq !== loadSeq) return  // 已有更新的请求在途，丢弃旧结果
    rows.value = Array.isArray(newRows) ? newRows : []
    total.value = Number(newTotal) || 0
    // 有旧数据时刷新失败只出提示（AsyncState 的 error 分支会隐藏内容）
    error.value = ''
    stale.value = false
    loaded.value = true
  } catch (e) {
    if (seq !== loadSeq) return
    if (loaded.value) error.value = ''
    stale.value = true
    if (!loaded.value) error.value = String(e.message || e)
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}

function clearFilters() {
  camId.value = ''; level.value = ''; status.value = ''; category.value = ''
  startDate.value = ''; endDate.value = ''
  q.value = ''; preset.value = ''; hourFilter.value = null; aiVerdict.value = ''
  resetAndLoad()
}
function resetAndLoad(opts = {}) {   // 筛选变化重置页码；时间线跳小时时保留 hourFilter
  if (!opts.keepHour) hourFilter.value = null
  offset.value = 0
  load()
}
function nextPage() { if (offset.value + PAGE_SIZE < total.value) { offset.value += PAGE_SIZE; load() } }
function prevPage() { offset.value = Math.max(0, offset.value - PAGE_SIZE); load() }
// 2026-10-30 审阅修复：原 onMounted 里 `await api.cameras()` 失败会跳过 load()/事件监听注册，
// 页面永久卡"加载中…"。改为同步注册监听 + load 先行 + 摄像头列表失败单独降级
const exportFilteredUrl = computed(() => {
  const p = new URLSearchParams()
  if (camId.value) p.set('camera_id', camId.value)
  if (level.value) p.set('level', level.value)
  if (category.value) p.set('category', category.value)
  if (status.value) p.set('status', status.value)
  if (preset.value === 'today') p.set('today', '1')
  else if (hourFilter.value) {
    const hf = hourFilter.value
    p.set('start_ts', `${hf.date}T${pad2(hf.hour)}:00:00`)
    p.set('end_ts', `${hf.date}T${pad2(hf.hour)}:59:59`)
  } else {
    if (startDate.value) p.set('start_ts', startDate.value)
    if (endDate.value) p.set('end_ts', endDate.value)
  }
  if (q.value) p.set('q', q.value)
  if (aiVerdict.value) p.set('ai_verdict', aiVerdict.value)
  return '/api/export/events.csv?' + p
})

let qTimer = null
function onQInput() { clearTimeout(qTimer); qTimer = setTimeout(resetAndLoad, 300) }
function applyPreset(v, reload = true) {
  preset.value = v
  const now = new Date()
  const fmt = d => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
  if (!v) { startDate.value = ''; endDate.value = '' }
  else if (v === 'today') { startDate.value = fmt(now); endDate.value = fmt(now) }
  else if (v === 'yesterday') { const y = new Date(now - 864e5); startDate.value = endDate.value = fmt(y) }
  else { const end = fmt(now); const start = fmt(new Date(now - (v === '7d' ? 6 : 29) * 864e5)); startDate.value = start; endDate.value = end }
  if (reload) resetAndLoad()
}
function applyRoute() {
  const query = new URLSearchParams(props.routeQuery)
  const nextCategory = query.get('category')
  category.value = ['check', 'alert', 'recovery'].includes(nextCategory) ? nextCategory : ''
  const nextStatus = query.get('status')
  status.value = Object.hasOwn(STATUS_TEXT, nextStatus) ? nextStatus : ''
  applyPreset(query.get('today') === '1' ? 'today' : '', false)
}
async function openLinkedEvent() {
  const id = Number(new URLSearchParams(props.routeQuery).get('event'))
  if (!Number.isSafeInteger(id) || id <= 0) return
  const query = props.routeQuery
  try {
    const data = await api.eventExplanation(id)
    if (query === props.routeQuery && data?.event?.id === id) {
      drawer.value = data.event
      nextTick(() => drawerEl.value?.focus({ preventScroll: true }))
    }
  } catch (error) {
    if (query === props.routeQuery) actionNotice.value = error.message
  }
}
watch(() => props.routeQuery, () => { applyRoute(); resetAndLoad(); openLinkedEvent() })
onMounted(() => {
  applyRoute()
  window.addEventListener('keydown', onKey)
  load(); loadTimeline()
  openLinkedEvent()
  api.cameras().then(list => { cams.value = Array.isArray(list) ? list : [] }).catch(() => { /* 筛选下拉留空即可 */ })
})
onUnmounted(() => window.removeEventListener('keydown', onKey))

// LS-052：键盘快审——J/↓ 下一条、K/↑ 上一条、A 确认、F 误报、I 忽略、Space 播放/暂停、Esc 关闭。
// 焦点在 input/textarea/select 时不触发（避免误伤搜索框/下拉）。
function onKey(e) {
  if (annotation.value) return
  if (e.ctrlKey || e.metaKey || e.altKey) return
  if (e.key === 'Escape') {
    if (zoom.value) { zoom.value = null }
    else if (videoZoom.value !== null) { videoZoom.value = null }
    else if (drawer.value) { closeDrawer() }
    return
  }
  const tag = (e.target?.tagName || '').toLowerCase()
  const typing = tag === 'input' || tag === 'textarea' || tag === 'select'
  if (typing) return
  const k = e.key.length === 1 ? e.key.toLowerCase() : e.key
  const list = rows.value || []
  if (!list.length) return
  const curIdx = drawer.value ? list.findIndex((r) => r.id === drawer.value.id) : -1
  const openAt = (idx) => {
    if (idx < 0 || idx >= list.length) return
    drawer.value = list[idx]
  }
  if (k === 'j' || k === 'ArrowDown') {
    e.preventDefault()
    openAt(curIdx < 0 ? 0 : Math.min(curIdx + 1, list.length - 1))
  } else if (k === 'k' || k === 'ArrowUp') {
    e.preventDefault()
    openAt(curIdx < 0 ? 0 : Math.max(curIdx - 1, 0))
  } else if (k === 'a' && drawer.value) {
    e.preventDefault(); quickMark('acked')
  } else if (k === 'f' && drawer.value) {
    e.preventDefault(); quickMark('false_positive')
  } else if (k === 'i' && drawer.value) {
    e.preventDefault(); quickMark('ignored')
  } else if (k === ' ' && drawer.value) {
    e.preventDefault()
    const v = document.querySelector('.drawer video')
    if (v) { v.paused ? v.play().catch(() => {}) : v.pause() }
  }
}

// LS-054：事件截图一键加入标注池（AI 预标注）
const poolBusy = ref(false)
const poolMsg = ref('')
const annotation = ref(null)
async function addToPool(e) {
  if (!e?.id) return
  poolBusy.value = true
  poolMsg.value = ''
  try {
    const r = await api.datasetFromEvent(e.id)
    if (r?.sample?.id) annotation.value = r.sample
    else poolMsg.value = `已加入标注池（样本 #${r.id}）`
  } catch (err) { poolMsg.value = String(err.message || err) }
  poolBusy.value = false
}

// 快审标记后自动跳到下一条
async function quickMark(status) {
  const cur = drawer.value
  if (!cur) return
  const idx = rows.value.findIndex((r) => r.id === cur.id)
  await mark(cur, status)
  const next = rows.value[idx + 1] || rows.value[idx] || null
  if (next) drawer.value = next
  else closeDrawer()
}

function openDetail(e, ev) {
  // 点击行打开抽屉；按钮/图片/视频区域点击不触发
  if (e.target.closest('button, img, video, a')) return
  lastFocus = document.activeElement
  drawer.value = ev
  nextTick(() => drawerEl.value?.focus({ preventScroll: true }))
}
function closeDrawer() {
  drawer.value = null
  if (lastFocus) { lastFocus.focus({ preventScroll: true }); lastFocus = null }
}

async function mark(e, status) {
  if (status === 'false_positive' &&
      !confirm(`确认把事件 #${e.id} 标记为误报？\n截图会复制到标注池（用于模型改进），事件保留但状态变更。`)) return
  if (busyEvent.value !== null) return
  busyEvent.value = e.id
  actionNotice.value = ''
  try {
    const result = await api.setEventStatus(e.id, status)
    if (drawer.value?.id === e.id) drawer.value = { ...drawer.value, status, handled_by: result?.handled_by, handled_ts: result?.handled_ts }
    if (status === 'false_positive') {
      actionNotice.value = result?.sample_id ? '已标记为误报，截图可在标注池中核验。'
        : `已标记为误报。${result?.sample_error || '截图暂未加入标注池。'}`
    }
    load()
  } catch (err) { alert(String(err.message || err)) }
  busyEvent.value = null
}

async function rememberSpot(event) {
  if (busyEvent.value !== null) return
  busyEvent.value = event.id
  actionNotice.value = ''
  try {
    await api.rememberEventSpot(event.id)
    actionNotice.value = '已记住误报点。可到该监控源的区域设置中移除。'
  } catch (error) {
    actionNotice.value = error.message
  } finally {
    busyEvent.value = null
  }
}
</script>

<template>
  <div class="panel">
    <p v-if="actionNotice" class="muted" role="status">{{ actionNotice }}</p>
    <div class="row">
      <b>事件查询</b>
      <select v-model="camId" @change="resetAndLoad">
        <option value="">全部摄像头</option>
        <option v-for="c in cams" :key="c.id" :value="c.id">{{ c.name }}</option>
      </select>
      <select v-model="category" aria-label="事件分类" @change="resetAndLoad">
        <option value="">全部分类</option>
        <option value="check">检查记录</option>
        <option value="alert">告警事件</option>
        <option value="recovery">恢复记录</option>
      </select>
      <select v-model="level" @change="resetAndLoad">
        <option value="">全部级别</option>
        <option value="1">L1 提示</option>
        <option value="2">L2 警告</option>
        <option value="3">L3 严重</option>
      </select>
      <select v-model="status" @change="resetAndLoad">
        <option value="">全部状态</option>
        <option v-for="(t, k) in STATUS_TEXT" :key="k" :value="k">{{ t }}</option>
      </select>
      <input v-model="q" @input="onQInput" placeholder="搜索详情/类型，如 实验服、reagent"
             aria-label="关键词搜索" style="width:200px" />
      <select v-model="aiVerdict" @change="resetAndLoad" aria-label="AI 复核筛选">
        <option value="">AI 复核：全部</option>
        <option value="likely_false">AI: 疑似误报</option>
        <option value="likely_true">AI: 疑似真实</option>
        <option value="uncertain">AI: 不确定</option>
        <option value="none">未复核</option>
      </select>
      <select v-model="preset" @change="applyPreset(preset)" aria-label="日期预设">
        <option value="">自定义</option>
        <option value="today">今天</option>
        <option value="yesterday">昨天</option>
        <option value="7d">近 7 天</option>
        <option value="30d">近 30 天</option>
      </select>
      <input type="date" v-model="startDate" @change="preset = ''; resetAndLoad()" title="开始日期" aria-label="开始日期" />
      <input type="date" v-model="endDate" @change="preset = ''; resetAndLoad()" title="结束日期" aria-label="结束日期" />
      <button class="ghost" @click="load">刷新</button>
      <span v-if="hourFilter" class="muted" style="font-size:12px">
        时间线 {{ hourFilter.date }} {{ pad2(hourFilter.hour) }}:00–59
        <button class="ghost" style="min-height:28px; padding:0 6px" @click="resetAndLoad()">清除</button>
      </span>
      <a :href="mediaUrl(exportFilteredUrl)" download>
        <button class="ghost">导出当前筛选</button>
      </a>
    </div>
    <!-- LS-052：键盘快审提示条（浅/深主题均用设计令牌） -->
    <div class="kbd-hints muted">
      <span><kbd>J</kbd>/<kbd>↓</kbd> 下一条</span>
      <span><kbd>K</kbd>/<kbd>↑</kbd> 上一条</span>
      <span><kbd>A</kbd> 已确认</span>
      <span><kbd>F</kbd> 误报</span>
      <span><kbd>I</kbd> 忽略</span>
      <span><kbd>Space</kbd> 播放/暂停</span>
      <span><kbd>Esc</kbd> 关闭</span>
    </div>

    <!-- 任务136：事件时间线（7 天×24 小时热力条，点击跳该小时） -->
    <details class="tl-box" @toggle="onToggleTimeline">
      <summary style="cursor:pointer">事件时间线（近 7 天，点击小时格筛选）</summary>
      <p v-if="tlLoading" class="muted" style="margin:6px 0">加载中…</p>
      <div v-else-if="tl" style="overflow-x:auto">
        <div style="min-width:520px">
          <div v-for="d in tl.days_list" :key="d.date" style="display:flex; align-items:center; gap:4px; margin-top:3px">
            <span class="muted" style="width:76px; flex:none; font-size:12px">{{ d.date.slice(5) }}</span>
            <div style="display:grid; grid-template-columns:repeat(24, 1fr); gap:2px; flex:1">
              <div v-for="h in d.hours" :key="h.hour"
                   :title="`${d.date} ${String(h.hour).padStart(2, '0')}:00 — ${h.total} 条事件${h.l3 ? `（L3 ${h.l3}）` : ''}`"
                   :style="{ background: tlColor(h), height: '16px', borderRadius: '3px',
                             cursor: h.total ? 'pointer' : 'default', position: 'relative' }"
                   @click="jumpHour(d, h)">
                <span v-if="h.l3" style="position:absolute; top:-5px; right:-2px; font-size:9px;
                      background:#dc2626; color:#fff; border-radius:8px; padding:0 4px; line-height:1.4">{{ h.l3 }}</span>
              </div>
            </div>
            <span class="muted" style="width:40px; flex:none; text-align:right; font-size:12px">{{ d.total }}</span>
          </div>
          <p class="muted" style="font-size:11px; margin:6px 0 0">
            图例：<span style="color:#2563eb">1-2</span> /
            <span style="color:#b45309">3-9</span> /
            <span style="color:#dc2626">≥10</span>（灰=无事件；红角标=L3 数）</p>
        </div>
      </div>
    </details>
    <SuppressionReview :camera-id="camId" />
    <AsyncState :loading="loading" :error="error" :notice="stale ? '刷新失败，仍显示上一次数据' : ''"
                :empty="loaded && !rows.length"
                :empty-text="camId || category || level || status || startDate || endDate || q || hourFilter ? '当前筛选条件下没有事件' + (q ? `（关键词：${q}）` : '') : '暂无事件。图片和视频完成检查后会留下记录。规则命中时会另生成告警。'"
                @retry="load">
      <table class="events-table">
      <thead><tr><th class="col-time"><input type="checkbox" aria-label="全选当前页"
             :checked="newRowsCount > 0 && selected.length === newRowsCount"
             @change="toggleAll($event.target.checked)" /></th>
          <th>时间</th><th>摄像头</th><th>类型</th><th>级别</th><th>详情</th><th>状态</th><th>截图</th><th>视频</th><th>处置</th></tr></thead>
      <tbody>
      <tr v-for="e in rows" :key="e.id"
          style="cursor:pointer"
          @click="openDetail($event, e)">
        <td><input v-if="e.status === 'new'" type="checkbox" aria-label="选择此事件"
               :checked="selected.includes(e.id)"
               @click.stop @change="toggleSel(e.id, $event.target.checked)" /></td>
        <td>{{ e.ts }}</td>
        <td>#{{ e.camera_id }}</td>
        <td>{{ TYPE_TEXT[e.type] || e.type }}
          <span v-if="aiBadge(e)" :class="aiBadge(e).cls" :title="aiTitle(e)"
                style="font-size:11px; margin-left:4px; white-space:nowrap">{{ aiBadge(e).text }}</span>
        </td>
        <td>
          <span v-if="e.category === 'check'" class="muted">检查记录</span>
          <span v-else-if="e.category === 'recovery' || e.resolved_ts" class="muted">已恢复</span>
          <span v-else :class="'lv' + e.level">{{ e.alert_tier === 'warning' ? '宽限提示' : `L${e.level}` }}</span>
        </td>
        <td class="muted">{{ e.detail }}</td>
        <td>
          <span :class="'lv' + (e.status === 'new' ? 3 : e.status === 'acked' ? 1 : 2)">
            {{ STATUS_TEXT[e.status] || e.status }}
          </span>
        </td>
        <td>
          <img v-if="e.has_snapshot" :src="mediaUrl(`/api/events/${e.id}/thumb`)" alt="事件截图缩略图"
               class="snap" loading="lazy" decoding="async" width="120" height="68"
               @click="zoom = mediaUrl(`/api/events/${e.id}/snapshot`)" />
        </td>
        <td>
          <button v-if="e.has_video" class="ghost" aria-label="播放事件视频" @click="videoZoom = e.id">▶</button>
        </td>
        <td style="white-space:nowrap">
          <template v-if="e.status === 'new'">
            <span v-if="e.escalated_ts" class="muted" :title="`未处置提醒已发送：${e.escalated_ts}`"
                  style="margin-right:4px">⏰</span>
            <button class="ghost" style="margin-right:4px" :disabled="busyEvent !== null"
                    @click="mark(e, 'acked')">确认</button>
            <button class="ghost" style="margin-right:4px" :disabled="busyEvent !== null"
                    @click="mark(e, 'false_positive')">误报</button>
            <button class="ghost" :disabled="busyEvent !== null" @click="mark(e, 'ignored')">忽略</button>
          </template>
          <span v-else class="muted" style="font-size:12px">{{ e.handled_ts?.slice(5, 16) }}</span>
        </td>
      </tr>
      </tbody>
    </table>
    <div v-if="selected.length" class="row pager" style="align-items:center">
      <b>已选 {{ selected.length }} 条</b>
      <button :disabled="bulkBusy" @click="bulk('acked')">批量确认</button>
      <button class="ghost" :disabled="bulkBusy"
              @click="bulk('false_positive')">批量误报（截图入标注池）</button>
      <button class="ghost" :disabled="bulkBusy" @click="bulk('ignored')">批量忽略</button>
      <button class="ghost" @click="selected = []">清除选择</button>
    </div>
    <div class="row pager">
      <button class="ghost" :disabled="offset === 0" @click="prevPage">上一页</button>
      <span class="muted">第 {{ offset + 1 }}-{{ Math.min(offset + PAGE_SIZE, total) }} 条 / 共 {{ total }} 条</span>
      <button class="ghost" :disabled="offset + PAGE_SIZE >= total" @click="nextPage">下一页</button>
    </div>
    </AsyncState>
  </div>

  <!-- 事件详情抽屉（任务17A：WinUI ContentDialog 风格） -->
  <Teleport to="body">
    <Transition name="drawer">
    <div v-if="drawer" class="drawer-mask" @click.self="closeDrawer">
      <aside class="drawer" ref="drawerEl" tabindex="-1" role="dialog" aria-modal="true">
        <header class="drawer-head">
          <b>#{{ drawer.id }} {{ TYPE_TEXT[drawer.type] || drawer.type }}</b>
          <span v-if="aiBadge(drawer)" :class="aiBadge(drawer).cls" :title="aiTitle(drawer)"
                style="font-size:12px; margin-left:6px">{{ aiBadge(drawer).text }}（仅供参考）</span>
          <button class="ghost" @click="closeDrawer">关闭</button>
        </header>
        <div class="drawer-body">
          <p class="muted">{{ drawer.ts }} · 摄像头 #{{ drawer.camera_id }}
            <template v-if="drawer.category === 'alert'"> · 轨迹 {{ drawer.track_id }} · <span :class="'lv' + drawer.level">L{{ drawer.level }}</span></template>
            · {{ STATUS_TEXT[drawer.status] || drawer.status }}</p>
          <p>{{ drawer.detail }}</p>
          <EventExplanation :key="drawer.id" :event-id="drawer.id" />
          <a :href="`#/assistant?event=${drawer.id}&camera=${drawer.camera_id}`">向 AI 助手追问</a>
          <p v-if="drawer.episode_id" class="muted">
            事件段 #{{ drawer.episode_id }} · {{ Number(drawer.duration_s || 0).toFixed(1) }} 秒 · {{ drawer.hit_count }} 次检出
            <template v-if="drawer.resolved_ts"> · 恢复于 {{ drawer.resolved_ts }}</template>
          </p>
          <p v-if="faceMatch" class="muted">人脸辅助识别 · {{ faceMatch.subject }} · 相似度 {{ Number(faceMatch.similarity).toFixed(2) }}</p>
          <SnapshotOverlay v-if="canPreview(drawer) && !previewUnavailable" :key="drawer.id"
               :eid="drawer.id" :src="mediaUrl(`/api/events/${drawer.id}/snapshot`)" :available="drawer.has_overlay === true"
               @error="previewUnavailable = true" @zoom="url => zoom = url" />
          <p v-else-if="previewUnavailable" class="muted">事件图片已不可用。</p>
          <video v-if="drawer.has_video" :src="mediaUrl(`/api/events/${drawer.id}/video`)" controls
                 style="width:100%; border-radius:var(--overlay-radius); background:#000" />
          <div v-if="drawer.status === 'new'" class="row">
            <button :disabled="busyEvent !== null" @click="mark(drawer, 'acked')">确认</button>
            <button class="ghost" :disabled="busyEvent !== null" @click="mark(drawer, 'false_positive')">误报</button>
            <button class="ghost" :disabled="busyEvent !== null" @click="mark(drawer, 'ignored')">忽略</button>
          </div>
          <p v-else-if="drawer.category !== 'check'" class="muted">处理人 {{ actorLabel(drawer.handled_by) }} · {{ drawer.handled_ts || '—' }}</p>
          <button v-if="drawer.status === 'false_positive' && ['enter', 'enter_no_ppe', 'no_mask'].includes(drawer.type)"
                  class="ghost" :disabled="busyEvent !== null" @click="rememberSpot(drawer)">记住这个误报点</button>
          <p v-if="actionNotice" class="muted" role="status">{{ actionNotice }}</p>
          <!-- 任务103：单事件证据包（无媒体时仅含 event.json+README，不置灰） -->
          <a class="ghost" :href="mediaUrl(`/api/events/${drawer.id}/evidence.zip`)"
             :download="`event-${drawer.id}-evidence.zip`"
             style="display:inline-flex; align-items:center; text-decoration:none">
            <button class="ghost">下载证据包</button></a>
          <button v-if="canPreview(drawer) && !previewUnavailable" class="ghost" :disabled="poolBusy" @click="addToPool(drawer)">
            {{ poolBusy ? '加入中…' : '快速标注（加入标注池）' }}</button>
          <span v-if="poolMsg" class="muted" style="font-size:12px">{{ poolMsg }}</span>
          <p class="muted" style="font-size:12px; margin:4px 0 0">
            含事件信息+截图+视频；无媒体时仅含事件信息与说明 README。</p>
          <section class="event-followup" aria-label="值班交接">
            <h3>值班交接</h3>
            <p v-if="notesLoading" class="muted" role="status">正在读取交接备注…</p>
            <p v-else-if="!notes.length && !notesError" class="muted">暂无交接备注。</p>
            <ol v-if="notes.length">
              <li v-for="item in notes" :key="item.id">
                <small class="muted">{{ actorLabel(item.actor) }} · {{ item.ts }}</small>
                <div class="note-text">{{ item.text }}</div>
              </li>
            </ol>
            <form class="note-form" @submit.prevent="saveNote">
              <textarea v-model="note" rows="2" maxlength="500" aria-label="交接备注" placeholder="填写交接备注" />
              <button type="submit" :disabled="!note.trim() || notesLoading || noteSaving">{{ noteSaving ? '保存中…' : '添加备注' }}</button>
            </form>
            <p v-if="notesError" class="st-err" role="alert">{{ notesError }}</p>
          </section>
          <details class="event-followup" :key="drawer.id" @toggle="deliveriesOpen = $event.target.open">
            <summary>告警投递记录</summary>
            <p v-if="deliveryLoading" class="muted" role="status">正在读取投递记录…</p>
            <p v-if="deliveryError" class="st-err" role="alert">{{ deliveryError }}</p>
            <p v-if="!deliveryLoading && !deliveryError && !deliveries.length" class="muted">该事件暂无外部渠道投递记录。</p>
            <ul v-if="deliveries.length">
              <li v-for="delivery in deliveries" :key="delivery.id">
                <div class="row delivery-row">
                  <b>{{ CHANNEL_NAMES[delivery.channel] || delivery.channel }} · {{ DELIVERY_LABELS[delivery.status] || delivery.status }}</b>
                  <button v-if="['retry', 'failed', 'dead_letter'].includes(delivery.status)" type="button" class="ghost"
                    :disabled="retrying !== null" @click="retryDelivery(delivery.id)">{{ retrying === delivery.id ? '重试中…' : '立即重试' }}</button>
                </div>
                <p v-if="delivery.last_error" class="muted">{{ delivery.last_error }}</p>
                <ol v-if="Array.isArray(delivery.history) && delivery.history.length" class="muted">
                  <li v-for="(attempt, index) in delivery.history" :key="index">{{ attempt.ts }} · {{ DELIVERY_LABELS[attempt.status] || attempt.status }}<span v-if="attempt.error"> · {{ attempt.error }}</span></li>
                </ol>
              </li>
            </ul>
          </details>
        </div>
      </aside>
    </div>
    </Transition>
  </Teleport>

  <div v-if="zoom" style="position:fixed; inset:0; background:rgba(0,0,0,.8); display:flex; align-items:center; justify-content:center; cursor:zoom-out" @click="zoom = null">
    <img :src="zoom" alt="事件截图放大视图" style="max-width:92vw; max-height:92vh" />
  </div>

  <div v-if="videoZoom !== null" style="position:fixed; inset:0; background:rgba(0,0,0,.8); display:flex; align-items:center; justify-content:center; cursor:zoom-out" @click="videoZoom = null">
    <video :src="mediaUrl(`/api/events/${videoZoom}/video`)" controls autoplay aria-label="事件视频回放" style="max-width:92vw; max-height:92vh" @click.stop />
  </div>
  <AnnotatorModal v-if="annotation" :sample="annotation" :image-url="`/api/dataset/samples/${annotation.id}/image`"
    @close="annotation = null" @saved="annotation = null" />
</template>

<style scoped>
/* LS-052：键盘快审提示条 */
.kbd-hints { display: flex; gap: 12px; flex-wrap: wrap; font-size: 12px; margin: 4px 2px 8px; }
.kbd-hints kbd { font-family: ui-monospace, monospace; font-size: 11px; padding: 1px 5px;
  border: 1px solid var(--stroke-default); border-radius: 4px; background: var(--card-bg-secondary);
  color: var(--text-primary); }
/* 任务19：分页条 */
.pager { justify-content: flex-start; }
/* 表格小屏横向滚动（任务17B） */
.panel { overflow-x: auto; }
/* 触控尺寸 ≥40px */
button { min-height: 40px; }
/* ---- 任务37：375px 下表格 9 列曾被压成一字一行；给最小宽度，让滚动发生在容器内 ---- */
.events-table { min-width: 760px; }
.events-table th, .events-table td { white-space: nowrap; }
/* 首列时间 sticky：横向滚动时保持可见（背景用卡片色防透字） */
.events-table th.col-time,
.events-table td:first-child {
  position: sticky; left: 0;
  background: var(--card-bg, #fff);
  z-index: 1;
}
/* ---- 事件详情抽屉（17A）：WinUI ContentDialog 风格，全部用主线令牌 ---- */
.drawer-mask {
  position: fixed; inset: 0; z-index: 50;
  background: transparent;
  display: flex; justify-content: flex-end;
}
/* 2026-11-21：模态过渡改纯 CSS（遮罩淡入 + 抽屉右滑入），替代 View
   Transitions（其旧页面快照在动画禁用配置下残留，用户实测「过渡有问题」） */
.drawer-mask:not(.drawer-enter-from):not(.drawer-leave-to) { background-color: rgba(0, 0, 0, 0.4); }
.drawer-enter-active, .drawer-leave-active { transition: background-color 0.22s ease; }
.drawer-enter-active .drawer, .drawer-leave-active .drawer {
  transition: transform 0.22s cubic-bezier(0.2, 0, 0, 1);
}
.drawer-enter-from .drawer, .drawer-leave-to .drawer { transform: translateX(100%); }
@media (prefers-reduced-motion: reduce) {
  .drawer-enter-active, .drawer-leave-active,
  .drawer-enter-active .drawer, .drawer-leave-active .drawer { transition: none; }
}
.drawer {
  width: min(440px, 100vw);
  height: 100%;
  background: var(--dialog-bg);
  border-radius: var(--overlay-radius) 0 0 var(--overlay-radius);
  box-shadow: var(--shadow-dialog);
  padding: 16px;
  overflow-y: auto;
  outline: none;
  color: var(--text-primary, inherit);
}
.drawer-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
.drawer-body p { margin: 8px 0; word-break: break-word; }
.event-followup { border-top: 1px solid var(--stroke-default); margin-top: 16px; padding-top: 12px; overflow-wrap: anywhere; }
.event-followup summary { cursor: pointer; }
.event-followup ol, .event-followup ul { margin: 8px 0; padding-left: 22px; }
.note-text { white-space: pre-wrap; }
.note-form { display: grid; gap: 8px; margin-top: 8px; }
.note-form button { justify-self: end; }
.delivery-row { justify-content: space-between; flex-wrap: wrap; }
@media (max-width: 480px) {
  .drawer { width: 100vw; border-radius: 0; }
}
/* 任务132：AI 复核徽标（仅供参考） */
.ai-warn { color: #b45309; font-weight: 600; }
.ai-ok { color: #16a34a; }
.ai-mute { color: #6b7280; }
</style>
