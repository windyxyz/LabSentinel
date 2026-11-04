<script setup>
import { computed, onMounted, onUnmounted, nextTick, ref } from 'vue'
import { api, TYPE_TEXT, STATUS_TEXT, mediaUrl } from '../api'
import AsyncState from '../components/AsyncState.vue'

const rows = ref([])
const camId = ref('')
const level = ref('')
const status = ref('')
const cams = ref([])
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
const drawerEl = ref(null)      // 抽屉 DOM（焦点管理）
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
const newRowsCount = 0  // 全选判定用（rows 中 new 事件数，computed 替代）

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
    alert(`批量处置完成：更新 ${r.updated}，跳过 ${r.skipped}` +
          (r.missing.length ? `，不存在 ${r.missing.length}` : '') +
          (r.failed.length ? `，失败 ${r.failed.length}` : ''))
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
  if (status.value) params.status = status.value
  if (hourFilter.value) {
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
    const newRows = await api.events(params)
    const newTotal = (await api.eventsCount(params)).count
    if (seq !== loadSeq) return  // 已有更新的请求在途，丢弃旧结果
    rows.value = newRows
    total.value = newTotal
    // 有旧数据时刷新失败只出提示（AsyncState 的 error 分支会隐藏内容）
    error.value = ''
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
  camId.value = ''; level.value = ''; status.value = ''
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
  if (status.value) p.set('status', status.value)
  if (hourFilter.value) {
    const hf = hourFilter.value
    p.set('start_ts', `${hf.date}T${pad2(hf.hour)}:00:00`)
    p.set('end_ts', `${hf.date}T${pad2(hf.hour)}:59:59`)
  } else {
    if (startDate.value) p.set('start_ts', startDate.value)
    if (endDate.value) p.set('end_ts', endDate.value)
  }
  if (q.value) p.set('q', q.value)
  return '/api/export/events.csv?' + p
})

let qTimer = null
function onQInput() { clearTimeout(qTimer); qTimer = setTimeout(resetAndLoad, 300) }
function applyPreset(v) {
  preset.value = v
  const now = new Date()
  const fmt = d => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
  if (!v) { startDate.value = ''; endDate.value = '' }
  else if (v === 'today') { startDate.value = fmt(now); endDate.value = fmt(now) }
  else if (v === 'yesterday') { const y = new Date(now - 864e5); startDate.value = endDate.value = fmt(y) }
  else { const end = fmt(now); const start = fmt(new Date(now - (v === '7d' ? 6 : 29) * 864e5)); startDate.value = start; endDate.value = end }
  resetAndLoad()
}
onMounted(() => {
  window.addEventListener('keydown', onKey)
  load(); loadTimeline()
  api.cameras().then(list => { cams.value = list }).catch(() => { /* 筛选下拉留空即可 */ })
})
onUnmounted(() => window.removeEventListener('keydown', onKey))

function onKey(e) {
  if (e.key === 'Escape') {
    if (zoom.value) { zoom.value = null }
    else if (videoZoom.value !== null) { videoZoom.value = null }
    else if (drawer.value) { closeDrawer() }
  }
}

// 任务135：View Transitions（尊重 prefers-reduced-motion；不支持/减少动效直接切换）
let vtRunning = false

function withViewTransition(apply) {
  const reduce = window.matchMedia
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches
  if (typeof document.startViewTransition !== 'function' || reduce || vtRunning) {
    apply()
    return
  }
  vtRunning = true
  document.startViewTransition(() => {
    apply()
  }).finished.finally(() => { vtRunning = false })
}

function openDetail(e, ev) {
  // 点击行打开抽屉；按钮/图片/视频区域点击不触发
  if (e.target.closest('button, img, video, a')) return
  lastFocus = document.activeElement
  withViewTransition(() => { drawer.value = ev })
  nextTick(() => drawerEl.value && drawerEl.value.focus())
}
function closeDrawer() {
  withViewTransition(() => { drawer.value = null })
  if (lastFocus) { lastFocus.focus(); lastFocus = null }
}

async function mark(e, status) {
  if (status === 'false_positive' &&
      !confirm(`确认把事件 #${e.id} 标记为误报？\n截图会复制到标注池（用于模型改进），事件保留但状态变更。`)) return
  if (busyEvent.value !== null) return
  busyEvent.value = e.id
  try {
    await api.setEventStatus(e.id, status)
    if (drawer.value && drawer.value.id === e.id) drawer.value = { ...drawer.value, status }
    load()
  } catch (err) { alert(String(err.message || err)) }
  busyEvent.value = null
}
</script>

<template>
  <div class="panel">
    <div class="row">
      <b>事件查询</b>
      <select v-model="camId" @change="resetAndLoad">
        <option value="">全部摄像头</option>
        <option v-for="c in cams" :key="c.id" :value="c.id">{{ c.name }}</option>
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
      <input type="date" v-model="startDate" @change="preset = ''; resetAndLoad" title="开始日期" aria-label="开始日期" />
      <input type="date" v-model="endDate" @change="preset = ''; resetAndLoad" title="结束日期" aria-label="结束日期" />
      <button class="ghost" @click="load">刷新</button>
      <span v-if="hourFilter" class="muted" style="font-size:12px">
        时间线 {{ hourFilter.date }} {{ pad2(hourFilter.hour) }}:00–59
        <button class="ghost" style="min-height:28px; padding:0 6px" @click="resetAndLoad()">清除</button>
      </span>
      <a :href="exportFilteredUrl" download>
        <button class="ghost">导出当前筛选</button>
      </a>
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
    <AsyncState :loading="loading" :error="error" :notice="stale ? '刷新失败，仍显示上一次数据' : ''"
                :empty="loaded && !rows.length"
                :empty-text="camId || level || status || startDate || endDate || q || hourFilter ? '当前筛选条件下没有事件' + (q ? `（关键词：${q}）` : '') : '暂无事件 —— 启动摄像头并配置规则后，事件会出现在这里'"
                @retry="load">
      <table class="events-table">
      <tr><th class="col-time"><input type="checkbox" aria-label="全选当前页"
             :checked="selected.length && selected.length === newRowsCount"
             @change="toggleAll($event.target.checked)" /></th>
          <th>摄像头</th><th>类型</th><th>级别</th><th>详情</th><th>状态</th><th>截图</th><th>视频</th><th>处置</th></tr>
      <tr v-for="e in rows" :key="e.id"
          :style="{ cursor: 'pointer',
                    viewTransitionName: (drawer && drawer.id === e.id) ? 'none' : `evi-${e.id}` }"
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
        <td><span :class="'lv' + e.level">L{{ e.level }}</span></td>
        <td class="muted">{{ e.detail }}</td>
        <td>
          <span :class="'lv' + (e.status === 'new' ? 3 : e.status === 'acked' ? 1 : 2)">
            {{ STATUS_TEXT[e.status] || e.status }}
          </span>
        </td>
        <td>
          <img v-if="e.has_snapshot" :src="mediaUrl(`/api/events/${e.id}/snapshot`)" alt="事件截图缩略图" class="snap" @click="zoom = mediaUrl(`/api/events/${e.id}/snapshot`)" />
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
    <div v-if="drawer" class="drawer-mask" @click.self="closeDrawer">
      <aside class="drawer" ref="drawerEl" tabindex="-1" role="dialog" aria-modal="true"
             :style="{ viewTransitionName: drawer ? `evi-${drawer.id}` : '' }">
        <header class="drawer-head">
          <b>#{{ drawer.id }} {{ TYPE_TEXT[drawer.type] || drawer.type }}</b>
          <span v-if="aiBadge(drawer)" :class="aiBadge(drawer).cls" :title="aiTitle(drawer)"
                style="font-size:12px; margin-left:6px">{{ aiBadge(drawer).text }}（仅供参考）</span>
          <button class="ghost" @click="closeDrawer">关闭</button>
        </header>
        <div class="drawer-body">
          <p class="muted">{{ drawer.ts }} · 摄像头 #{{ drawer.camera_id }} · track {{ drawer.track_id }}
            · <span :class="'lv' + drawer.level">L{{ drawer.level }}</span>
            · {{ STATUS_TEXT[drawer.status] || drawer.status }}</p>
          <p>{{ drawer.detail }}</p>
          <img v-if="drawer.has_snapshot" :src="mediaUrl(`/api/events/${drawer.id}/snapshot`)" alt="事件截图大图，点击放大"
               class="drawer-snap" @click="zoom = mediaUrl(`/api/events/${drawer.id}/snapshot`)" />
          <video v-if="drawer.has_video" :src="mediaUrl(`/api/events/${drawer.id}/video`)" controls
                 style="width:100%; border-radius:var(--overlay-radius); background:#000" />
          <div v-if="drawer.status === 'new'" class="row">
            <button :disabled="busyEvent !== null" @click="mark(drawer, 'acked')">确认</button>
            <button class="ghost" :disabled="busyEvent !== null" @click="mark(drawer, 'false_positive')">误报</button>
            <button class="ghost" :disabled="busyEvent !== null" @click="mark(drawer, 'ignored')">忽略</button>
          </div>
          <p v-else class="muted">处理人 {{ drawer.handled_by || '-' }} · {{ drawer.handled_ts || '-' }}</p>
          <!-- 任务103：单事件证据包（无媒体时仅含 event.json+README，不置灰） -->
          <a class="ghost" :href="mediaUrl(`/api/events/${drawer.id}/evidence.zip`)"
             :download="`event-${drawer.id}-evidence.zip`"
             style="display:inline-flex; align-items:center; text-decoration:none">
            <button class="ghost">下载证据包</button></a>
          <p class="muted" style="font-size:12px; margin:4px 0 0">
            含事件信息+截图+视频；无媒体时仅含事件信息与说明 README。</p>
        </div>
      </aside>
    </div>
  </Teleport>

  <div v-if="zoom" style="position:fixed; inset:0; background:rgba(0,0,0,.8); display:flex; align-items:center; justify-content:center; cursor:zoom-out" @click="zoom = null">
    <img :src="zoom" alt="事件截图放大视图" style="max-width:92vw; max-height:92vh" />
  </div>

  <div v-if="videoZoom !== null" style="position:fixed; inset:0; background:rgba(0,0,0,.8); display:flex; align-items:center; justify-content:center; cursor:zoom-out" @click="videoZoom = null">
    <video :src="mediaUrl(`/api/events/${videoZoom}/video`)" controls autoplay aria-label="事件视频回放" style="max-width:92vw; max-height:92vh" @click.stop />
  </div>
</template>

<style>
/* 任务135：View Transitions（仅 Events 页；不支持的浏览器自动忽略） */
::view-transition-group(*) { animation-duration: 220ms; animation-timing-function: cubic-bezier(0.2, 0, 0, 1); }
::view-transition-old(root), ::view-transition-new(root) { animation: none; }
@media (prefers-reduced-motion: reduce) {
  ::view-transition-group(*), ::view-transition-old(*), ::view-transition-new(*) { animation: none !important; }
}
</style>

<style scoped>
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
  background: rgba(0, 0, 0, 0.4);
  display: flex; justify-content: flex-end;
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
.drawer-snap { width: 100%; border-radius: var(--overlay-radius); cursor: zoom-in; margin: 8px 0; }
@media (max-width: 480px) {
  .drawer { width: 100vw; border-radius: 0; }
}
/* 任务132：AI 复核徽标（仅供参考） */
.ai-warn { color: #b45309; font-weight: 600; }
.ai-ok { color: #16a34a; }
.ai-mute { color: #6b7280; }
</style>
