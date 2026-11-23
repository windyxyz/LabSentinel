<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { api, connectEvents, TYPE_TEXT } from '../api'
import { notifyEvent, ensurePermission, permissionState, notifyEnabled, setNotifyEnabled } from '../notify'
import AsyncState from '../components/AsyncState.vue'

const wsState = ref('连接中')
// 任务115：桌面通知状态与开关
const notifPerm = ref(permissionState())
const notifOn = ref(notifyEnabled())

async function enableNotify() {
  notifPerm.value = await ensurePermission()
  if (notifPerm.value === 'granted') {
    setNotifyEnabled(true)
    notifOn.value = true
  }
}
function toggleNotify() {
  setNotifyEnabled(!notifOn.value)
  notifOn.value = !notifOn.value
}

const releaseUrl = 'https://github.com/windyxyz/LabSentinel/releases/latest'
const sys = ref(null)
const monitoring = computed(() => {
  const counts = { total: 0, running: 0, starting: 0, stopped: 0, offline: 0 }
  const cameras = sys.value?.cameras
  if (!Array.isArray(cameras)) return null
  for (const camera of cameras) {
    if (!camera || typeof camera.id !== 'number' || camera.type === 'folder') continue
    counts.total++
    if (camera.offline_since && camera.status !== 'stopped') { counts.offline++; continue }
    switch (camera.status) {
      case 'running': counts.running++; break
      case 'starting':
      case 'reconnecting': counts.starting++; break
      case 'stopped':
      case 'completed': counts.stopped++; break
      default: counts.offline++
    }
  }
  return counts
})
const feed = ref([])
const loading = ref(true)
const error = ref('')
const notice = ref('')
let pollTimer = null
let inFlight = null
let maintainedAt = 0
let closeEvents
let mounted = false
const maint = ref(null)

async function refreshSystem() {
  error.value = ''
  try {
    const s = await api.system()
    sys.value = s          // 轮询失败时保留旧值（notice 提示），成功才覆盖
    notice.value = ''
  } catch (e) {
    if (sys.value) notice.value = '刷新失败，正在重试…（仍显示上一次数据）'
    else error.value = String(e.message || e)
  } finally {
    loading.value = false
  }
}

function refreshMaintenance() {
  if (Date.now() - maintainedAt < 30000) return
  maintainedAt = Date.now()
  return api.maintenanceStatus().then((status) => { maint.value = status }).catch(() => {})
}

function scheduleRefresh() {
  clearTimeout(pollTimer)
  if (mounted && !document.hidden) pollTimer = setTimeout(reload, 5000)
}

function reload() {
  if (!inFlight) inFlight = Promise.all([refreshSystem(), refreshMaintenance()]).finally(() => {
    inFlight = null
    scheduleRefresh()
  })
  return inFlight
}

function syncVisibility() {
  clearTimeout(pollTimer)
  if (!document.hidden) reload()
}

onMounted(() => {
  mounted = true
  closeEvents = connectEvents(
    (ev) => {
      feed.value = [ev, ...feed.value].slice(0, 100)
      notifyEvent(ev)  // 任务115：页面隐藏且有权限时弹桌面通知（L2/L3，tag 去重）
    },
    (s) => (wsState.value = s),
  )
  document.addEventListener('visibilitychange', syncVisibility)
  reload()
})
onBeforeUnmount(() => {
  mounted = false
  closeEvents?.()
  clearTimeout(pollTimer)
  document.removeEventListener('visibilitychange', syncVisibility)
})
</script>

<template>
  <AsyncState :loading="loading" :error="error" :notice="notice" @retry="reload">
    <div class="cards" v-if="sys">
      <div class="card monitor-card">
        <div class="muted">监控状态</div>
        <template v-if="monitoring?.total">
          <div class="num">{{ monitoring.running }} / {{ monitoring.total }}<span class="muted stat-unit">路运行</span></div>
          <div class="muted stat-detail">
            {{ monitoring.starting }} 路连接中 · {{ monitoring.offline }} 路离线 · {{ monitoring.stopped }} 路停止
          </div>
        </template>
        <p v-else class="muted stat-detail">{{ monitoring ? '尚未添加摄像头' : '暂时无法读取摄像头状态' }}</p>
        <a :href="monitoring?.offline ? '#/sources/cams?status=offline' : '#/sources/cams'" class="stat-link">
          {{ monitoring?.total === 0 ? '添加摄像头' : monitoring?.offline ? '查看离线摄像头' : '查看摄像头' }}
        </a>
      </div>
      <div class="card"><div class="muted">今日事件</div><div class="num">{{ Object.values(sys.today_counts || {}).reduce((a, b) => a + b, 0) }}</div></div>
      <div class="card">
        <div class="muted">今日待处置</div>
        <div class="num" :style="{ color: sys.unhandled_today > 0 ? 'var(--l2)' : undefined }">{{ sys.unhandled_today ?? 0 }}</div>
        <a href="#/events?status=new&today=1" class="stat-link">查看待处置事件</a>
      </div>
      <div class="card"><div class="muted">警告(L2)</div><div class="num">{{ sys.today_counts?.['2'] || 0 }}</div></div>
      <div class="card"><div class="muted">严重(L3)</div><div class="num">{{ sys.today_counts?.['3'] || 0 }}</div></div>
      <div class="card"><div class="muted">实时通道</div><div class="num" :style="{ fontSize: '18px', paddingTop: '6px' }">{{ wsState }}</div></div>
      <div class="card">
        <div class="muted">当前版本</div>
        <div class="num version">{{ sys.version || '—' }}</div>
        <a class="stat-link" :href="releaseUrl" target="_blank" rel="noopener noreferrer">查看更新</a>
      </div>
      <div class="card" :title="`通知权限：${notifPerm}；页面在后台时 L2/L3 弹桌面通知`">
        <div class="muted">桌面通知</div>
        <div style="padding-top:6px">
          <button v-if="notifPerm !== 'granted'" class="ghost" @click="enableNotify">开启桌面通知</button>
          <label v-else class="muted" style="font-size:13px">
            <input type="checkbox" :checked="notifOn" @change="toggleNotify" style="margin:0 4px 0 0" />
            {{ notifOn ? '已开启' : '已暂停' }}</label>
        </div>
      </div>
      <div v-if="maint" class="card" :style="maint.low_disk ? 'border-color:#dc2626; background:rgba(220,38,38,.08)' : undefined"
           :title="maint.forecast_days != null ? `按当前增速约 ${maint.forecast_days} 天后磁盘吃紧（估算值）` : '数据目录所在磁盘'">
        <div class="muted">磁盘{{ maint.low_disk ? ' ⚠ 磁盘不足，将触发紧急清理' : '' }}</div>
        <div class="num" :style="maint.low_disk ? { color: '#dc2626' } : undefined">
          {{ maint.disk_free_gb?.toFixed(1) ?? '-' }} / {{ maint.disk_total_gb?.toFixed(1) ?? '-' }}GB</div>
        <div v-if="maint.forecast_days != null && !maint.low_disk" class="muted" style="font-size:11px">
          按当前增速约 {{ maint.forecast_days }} 天（估算）</div>
      </div>
    </div>
  </AsyncState>

  <div class="panel">
    <h3 style="margin-bottom:10px">实时事件</h3>
    <table v-if="feed.length">
      <tbody>
        <tr v-for="(e, i) in feed" :key="e.id ?? i" :class="{ 'new-item': i === 0 }">
          <td>{{ e.ts }}</td>
          <td>摄像头#{{ e.camera_id }}</td>
          <td>{{ TYPE_TEXT[e.type] || e.type }}</td>
          <td><span :class="'lv' + e.level">L{{ e.level }}</span></td>
          <td class="muted">{{ e.detail }}</td>
        </tr>
      </tbody>
    </table>
    <p v-else-if="sys" class="muted">暂无实时事件 —— 去摄像头页添加/启动摄像头，触发行为会实时出现在这里</p>
    <p v-else class="muted">等待事件…（配置安全区与规则后，触发行为会实时出现在这里）</p>
  </div>
</template>

<style scoped>
/* 任务17B：375px 小屏适配（scoped，不动全局样式） */
.cards { flex-wrap: wrap; }
.card { min-width: 140px; }
.monitor-card { flex: 1.6 1 210px; }
.stat-unit { font-size: 13px; font-weight: 400; margin-left: 5px; }
.stat-detail { font-size: 12px; margin-top: 4px; }
.stat-link { display: inline-block; margin-top: 6px; color: var(--accent-strong); }
.card .version { font-size: 20px; }
.panel { overflow-x: auto; }
button { min-height: 40px; }
</style>
