<script setup>
// 统计报表：按天折线图 + 按小时柱状图 + 每日明细表。
// 图表用原生 SVG（体积敏感，不引 ECharts）。
import { computed, onMounted, ref, watch } from 'vue'
import { api, TYPE_TEXT, mediaUrl } from '../api'
import AsyncState from '../components/AsyncState.vue'

const days = ref(7)
const camId = ref('')
const cams = ref([])
const daily = ref([])
const hourly = ref([])
// 2026-10-30 审阅修复：toISOString() 是 UTC 日期，东八区早 8 点前"今日"实为昨天——改本地日期
function localToday() {
  const d = new Date()
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}
const hourDate = ref(localToday())
// 任务21：周视图 / 汇总卡片 / 自定义日期范围
const viewMode = ref('day')            // 'day' | 'week'
const weekCount = ref(4)               // 周视图周数（4/12）
const weekly = ref([])
const summary = ref(null)
const quality = ref(null)  // 任务102：处置质量统计
const rangeStart = ref('')
const rangeEnd = ref('')

// 任务66：三态
const loading = ref(true)
const stale = ref(false)  // 有旧图表时的刷新失败（只提示，不进错误分支）
const error = ref('')
let loadSeq = 0           // 2026-10-30 审阅：请求序号（watch 六个值连续触发时防旧响应覆盖）

async function load() {
  const seq = ++loadSeq
  const base = camId.value ? { camera_id: camId.value } : {}
  try {
    const s = await api.statsSummary({ days: 30, ...base })
    const h = (await api.statsHourly({ date: hourDate.value, ...base })).hours
    let d
    if (viewMode.value === 'week') {
      d = (await api.statsWeekly({ weeks: weekCount.value, ...base })).weeks
    } else if (rangeStart.value && rangeEnd.value) {
      d = (await api.statsDaily({
        start_ts: rangeStart.value, end_ts: rangeEnd.value, ...base })).days
    } else {
      d = (await api.statsDaily({ days: days.value, ...base })).days
    }
    if (seq !== loadSeq) return  // 已有更新请求在途，丢弃旧结果
    summary.value = s
    hourly.value = h
    daily.value = d
    // 任务102：处置质量（独立容错——旧服务端无此端点时只隐藏卡片，不影响图表）
    try {
      const q = await api.statsQuality({ days: days.value, ...base })
      if (seq === loadSeq) quality.value = q
    } catch { /* 端点缺失静默 */ }
    error.value = ''
    stale.value = false
  } catch (e) {
    if (seq !== loadSeq) return
    if (summary.value) stale.value = true  // 失败保留旧图表数据，只提示
    else error.value = String(e.message || e)
  } finally {
    if (seq === loadSeq) loading.value = false
  }
}
function setView(m) {
  viewMode.value = m
  days.value = m === 'week' ? 7 : days.value
  if (m === 'week') weekCount.value = 4
  load()
}
// 任务87：审计
const auditItems = ref([])
const auditTotal = ref(0)
const auditOffset = ref(0)
const auditActor = ref('')
const auditLoading = ref(false)
const auditError = ref('')

async function loadAudit() {
  auditLoading.value = true
  try {
    const params = { limit: 50, offset: auditOffset.value }
    if (auditActor.value) params.actor = auditActor.value
    const r = await api.audit(params)
    auditItems.value = r.items
    auditTotal.value = r.total
    auditError.value = ''
  } catch (e) { auditError.value = String(e.message || e) }
  auditLoading.value = false
}

onMounted(() => {
  // 2026-10-30 审阅修复：原 await api.cameras() 失败会跳过 load()，页面永久卡加载中
  load()
  api.cameras().then(list => { cams.value = list }).catch(() => { /* 摄像头筛选留空 */ })
})
watch([days, camId, hourDate, weekCount, rangeStart, rangeEnd], load)

// 表格里的行为类型列（只显示数据里出现过的类型）
const typeKeys = computed(() => {
  const s = new Set()
  daily.value.forEach(r => Object.keys(r).forEach(k => { if (TYPE_TEXT[k]) s.add(k) }))
  return [...s]
})

// ---- 折线图几何（viewBox 720x220；日/周视图共用）----
const W = 720, H = 220, PAD = 34
const trendRows = computed(() => viewMode.value === 'week' ? weekly.value : daily.value)
const trendKey = computed(() => viewMode.value === 'week' ? 'week_start' : 'date')
const maxTotal = computed(() => Math.max(1, ...trendRows.value.map(r => r.total)))
function linePoints(key) {
  const n = trendRows.value.length
  if (n < 2) return ''
  return trendRows.value.map((r, i) => {
    const x = PAD + i * (W - 2 * PAD) / (n - 1)
    const y = H - PAD - (r[key] || 0) / maxTotal.value * (H - 2 * PAD)
    return `${x.toFixed(1)},${y.toFixed(1)}`
  }).join(' ')
}
function xTick(i) { return PAD + i * (W - 2 * PAD) / Math.max(1, trendRows.value.length - 1) }

// 任务33：证据包 ZIP（复用当前筛选；无自定义范围时按 days 折算起始日）
const evidenceZipUrl = computed(() => {
  const p = new URLSearchParams()
  if (camId.value) p.set('camera_id', camId.value)
  if (rangeStart.value && rangeEnd.value) {
    p.set('start_ts', rangeStart.value)
    p.set('end_ts', rangeEnd.value)
  } else {
    const st = new Date()
    st.setDate(st.getDate() - (days.value - 1))
    p.set('start_ts', st.toISOString().slice(0, 10))
    p.set('end_ts', new Date().toISOString().slice(0, 10))
  }
  return mediaUrl('/api/export/evidence.zip?' + p)
})

// ---- 柱状图几何 ----
const maxHour = computed(() => Math.max(1, ...hourly.value.map(h => h.total)))
function barGeom(h) {
  const bw = (W - 2 * PAD) / 24
  const bh = h.total / maxHour.value * (H - 2 * PAD)
  return { x: PAD + h.hour * bw + bw * 0.15, y: H - PAD - bh, w: bw * 0.7, h: bh }
}
</script>

<template>
  <div class="cards" v-if="summary">
    <div class="card"><div class="muted">近{{ summary.days }}天事件</div><div class="num">{{ summary.total }}</div></div>
    <div class="card"><div class="muted">L1 提示</div><div class="num">{{ summary.by_level[1] }}</div></div>
    <div class="card"><div class="muted">L2 警告</div><div class="num">{{ summary.by_level[2] }}</div></div>
    <div class="card"><div class="muted">L3 严重</div><div class="num">{{ summary.by_level[3] }}</div></div>
    <div class="card"><div class="muted">最高事件日</div><div class="num" style="font-size:16px">{{ summary.top_days[0]?.date || '-' }}（{{ summary.top_days[0]?.total || 0 }}）</div></div>
  </div>

  <!-- 任务102：处置质量（误报率=已处置中误报占比，分母不含未处置） -->
  <div class="cards" v-if="quality && quality.total > 0">
    <div class="card"><div class="muted">未处置</div><div class="num">{{ quality.unhandled }}</div></div>
    <div class="card"><div class="muted">L3 严重（窗口内）</div><div class="num">{{ quality.l3_total }}</div></div>
    <div class="card"><div class="muted">误报率（已处置口径）</div>
      <div class="num">{{ (quality.false_positive_rate * 100).toFixed(1) }}%</div></div>
    <div class="card"><div class="muted">已处置 / 总数</div>
      <div class="num" style="font-size:16px">{{ quality.total - quality.unhandled }} / {{ quality.total }}</div></div>
  </div>
  <div class="panel" v-if="quality && quality.total > 0">
    <b>按摄像头处置质量（近 {{ quality.days }} 天，最多 10 路）</b>
    <table style="margin-top:8px">
      <tr><th>摄像头</th><th>事件</th><th>误报</th><th>误报率</th></tr>
      <tr v-for="c in quality.by_camera.slice(0, 10)" :key="c.camera_id">
        <td>{{ c.name || ('#' + c.camera_id) }}</td>
        <td>{{ c.total }}</td>
        <td>{{ c.false_positive }}</td>
        <td>{{ (c.rate * 100).toFixed(1) }}%</td>
      </tr>
    </table>
  </div>
  <div class="panel" v-else-if="quality && quality.total === 0">暂无数据（统计窗口内无事件）</div>

  <div class="panel">
    <div class="row">
      <b>统计报表</b>
      <button :class="viewMode === 'day' ? '' : 'ghost'" @click="setView('day')">日</button>
      <button :class="viewMode === 'week' ? '' : 'ghost'" @click="setView('week')">周</button>
      <select v-if="viewMode === 'day'" v-model.number="days">
        <option :value="7">最近 7 天</option>
        <option :value="30">最近 30 天</option>
      </select>
      <select v-else v-model.number="weekCount">
        <option :value="4">最近 4 周</option>
        <option :value="12">最近 12 周</option>
      </select>
      <input type="date" v-model="rangeStart" title="自定义开始日期（填起止生效）" />
      <input type="date" v-model="rangeEnd" title="自定义结束日期" />
      <select v-model="camId">
        <option value="">全部摄像头</option>
        <option v-for="c in cams" :key="c.id" :value="c.id">{{ c.name }}</option>
      </select>
      <input type="date" v-model="hourDate" />
      <button class="ghost" @click="load">刷新</button>
      <a :href="mediaUrl(`/api/export/events.csv?days=${days}${camId ? '&camera_id=' + camId : ''}`)" download>
        <button class="ghost">导出事件 CSV</button>
      </a>
      <a :href="mediaUrl(`/api/export/stats.csv?days=${days}`)" download>
        <button class="ghost">导出统计 CSV</button>
      </a>
      <a :href="mediaUrl(`/api/export/monthly.pdf?month=${hourDate.slice(0, 7)}`)" download>
        <button class="ghost">月报 PDF</button>
      </a>
      <a :href="evidenceZipUrl" download>
        <button class="ghost">导出证据包 ZIP</button>
      </a>
    </div>
  </div>

  <AsyncState :loading="loading" :error="error" :notice="stale ? '刷新失败，仍显示上一次数据' : ''"
              :empty="summary && summary.total === 0"
              empty-text="所选区间没有事件数据 —— 试试更换日期范围、选择全部摄像头，或先去摄像头页启动采集"
              @retry="load">
    <div class="panel">
    <b>{{ viewMode === 'week' ? '每周事件趋势' : '每日事件趋势' }}</b>
    <svg :viewBox="`0 0 ${W} ${H}`" style="width:100%; margin-top:8px">
      <!-- 网格与 Y 轴刻度 -->
      <line v-for="i in 5" :key="i" :x1="PAD" :x2="W - PAD"
            :y1="H - PAD - (i - 1) * (H - 2 * PAD) / 4" :y2="H - PAD - (i - 1) * (H - 2 * PAD) / 4"
            style="stroke:var(--divider)" stroke-width="1" />
      <text v-for="i in 5" :key="'t' + i" :x="4" :y="H - PAD - (i - 1) * (H - 2 * PAD) / 4 + 4"
            fill="#7a8899" font-size="10">{{ Math.round((i - 1) * maxTotal / 4) }}</text>
      <polyline :points="linePoints('total')" fill="none" stroke="var(--accent)" stroke-width="2" />
      <polyline :points="linePoints('l3')" fill="none" stroke="var(--l3)" stroke-width="1.5" />
      <polyline :points="linePoints('l2')" fill="none" stroke="var(--l2)" stroke-width="1.5" />
      <polyline :points="linePoints('l1')" fill="none" stroke="var(--l1)" stroke-width="1.5" />
      <!-- X 轴日期（最多标 8 个）-->
      <text v-for="(r, i) in trendRows" :key="'x' + r[trendKey]" v-show="i % Math.ceil(trendRows.length / 8) === 0"
            :x="xTick(i)" :y="H - PAD + 14" fill="#7a8899" font-size="10" text-anchor="middle">{{ r[trendKey].slice(5) }}</text>
    </svg>
    <div class="row muted" style="margin:0">
      <span>— 合计</span><span style="color:var(--l3)">— L3</span>
      <span style="color:var(--l2)">— L2</span><span style="color:var(--l1)">— L1</span>
    </div>
  </div>

  <div class="panel">
    <b>小时分布（{{ hourDate }}）</b>
    <svg :viewBox="`0 0 ${W} ${H}`" style="width:100%; margin-top:8px">
      <line :x1="PAD" :x2="W - PAD" :y1="H - PAD" :y2="H - PAD" style="stroke:var(--divider)" />
      <rect v-for="h in hourly" :key="h.hour" :x="barGeom(h).x" :y="barGeom(h).y"
            :width="barGeom(h).w" :height="barGeom(h).h" fill="var(--accent)" opacity="0.85" />
      <text v-for="h in hourly" v-show="h.hour % 3 === 0" :key="'hx' + h.hour"
            :x="PAD + (h.hour + 0.5) * (W - 2 * PAD) / 24" :y="H - PAD + 14"
            fill="#7a8899" font-size="10" text-anchor="middle">{{ h.hour }}</text>
    </svg>
  </div>

  <div class="panel">
    <b>每日明细</b>
    <table style="margin-top:8px">
      <tr>
        <th>日期</th><th>合计</th><th>L1</th><th>L2</th><th>L3</th>
        <th v-for="k in typeKeys" :key="k">{{ TYPE_TEXT[k] }}</th>
      </tr>
      <tr v-for="r in daily" :key="r.date">
        <td>{{ r.date }}</td><td>{{ r.total }}</td><td>{{ r.l1 }}</td><td>{{ r.l2 }}</td><td>{{ r.l3 }}</td>
        <td v-for="k in typeKeys" :key="k">{{ r[k] || 0 }}</td>
      </tr>
    </table>
  </div>
  </AsyncState>

  <details class="panel" style="margin-top:10px">
    <summary style="cursor:pointer"><b>操作审计（最近写操作，任务87）</b></summary>
    <div class="row" style="margin-top:8px">
      <select v-model="auditActor" aria-label="操作者筛选">
        <option value="">全部操作者</option>
        <option value="admin">admin</option>
        <option value="anonymous">anonymous</option>
      </select>
      <button class="ghost" @click="loadAudit">加载</button>
    </div>
    <AsyncState :loading="auditLoading" :error="auditError" :empty="!auditItems.length"
                empty-text="暂无写操作" @retry="loadAudit">
      <table style="margin-top:8px">
        <tr><th>时间</th><th>操作者</th><th>动作</th><th>目标</th><th>状态码</th><th>request_id</th></tr>
        <tr v-for="a in auditItems" :key="a.id">
          <td class="muted">{{ a.ts }}</td><td>{{ a.actor }}</td><td>{{ a.action }}</td>
          <td class="muted">{{ a.target || '—' }}</td>
          <td><span :class="a.status_code < 400 ? '' : 'st-err'">{{ a.status_code }}</span></td>
          <td class="muted" style="font-size:11px">{{ a.request_id?.slice(0, 8) }}…</td>
        </tr>
      </table>
      <div class="row pager">
        <button class="ghost" :disabled="auditOffset === 0" @click="auditOffset = Math.max(0, auditOffset - 50); loadAudit()">上一页</button>
        <span class="muted">共 {{ auditTotal }} 条</span>
        <button class="ghost" :disabled="auditOffset + 50 >= auditTotal" @click="auditOffset += 50; loadAudit()">下一页</button>
      </div>
    </AsyncState>
  </details>
</template>
