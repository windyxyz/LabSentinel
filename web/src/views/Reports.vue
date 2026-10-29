<script setup>
// 统计报表：按天折线图 + 按小时柱状图 + 每日明细表。
// 图表用原生 SVG（体积敏感，不引 ECharts）。
import { computed, onMounted, ref, watch } from 'vue'
import { api, TYPE_TEXT } from '../api'
import AsyncState from '../components/AsyncState.vue'

const days = ref(7)
const camId = ref('')
const cams = ref([])
const daily = ref([])
const hourly = ref([])
const hourDate = ref(new Date().toISOString().slice(0, 10))
// 任务21：周视图 / 汇总卡片 / 自定义日期范围
const viewMode = ref('day')            // 'day' | 'week'
const weekCount = ref(4)               // 周视图周数（4/12）
const weekly = ref([])
const summary = ref(null)
const rangeStart = ref('')
const rangeEnd = ref('')

// 任务66：三态
const loading = ref(true)
const error = ref('')

async function load() {
  const base = camId.value ? { camera_id: camId.value } : {}
  try {
    summary.value = await api.statsSummary({ days: 30, ...base })
    hourly.value = (await api.statsHourly({ date: hourDate.value, ...base })).hours
    if (viewMode.value === 'week') {
      weekly.value = (await api.statsWeekly({ weeks: weekCount.value, ...base })).weeks
    } else if (rangeStart.value && rangeEnd.value) {
      daily.value = (await api.statsDaily({
        start_ts: rangeStart.value, end_ts: rangeEnd.value, ...base })).days
    } else {
      daily.value = (await api.statsDaily({ days: days.value, ...base })).days
    }
    error.value = ''
  } catch (e) {
    // 失败保留旧图表数据，只提示
    error.value = String(e.message || e)
  } finally {
    loading.value = false
  }
}
function setView(m) {
  viewMode.value = m
  days.value = m === 'week' ? 7 : days.value
  if (m === 'week') weekCount.value = 4
  load()
}
onMounted(async () => {
  cams.value = await api.cameras()
  load()
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
  return '/api/export/evidence.zip?' + p
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
      <a :href="`/api/export/events.csv?days=${days}${camId ? '&camera_id=' + camId : ''}`" download>
        <button class="ghost">导出事件 CSV</button>
      </a>
      <a :href="`/api/export/stats.csv?days=${days}`" download>
        <button class="ghost">导出统计 CSV</button>
      </a>
      <a :href="`/api/export/monthly.pdf?month=${hourDate.slice(0, 7)}`" download>
        <button class="ghost">月报 PDF</button>
      </a>
      <a :href="evidenceZipUrl" download>
        <button class="ghost">导出证据包 ZIP</button>
      </a>
    </div>
  </div>

  <AsyncState :loading="loading" :error="error" :notice="summary ? '刷新失败，仍显示上一次数据' : ''"
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
</template>
