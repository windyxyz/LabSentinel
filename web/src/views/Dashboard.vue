<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import { api, connectEvents, TYPE_TEXT } from '../api'
import AsyncState from '../components/AsyncState.vue'

const wsState = ref('连接中')
const sys = ref(null)
const feed = ref([])
const loading = ref(true)
const error = ref('')
const notice = ref('')
let pollTimer = null
const maint = ref(null)  // 任务68：磁盘水位

async function reload() {
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
  try {
    maint.value = await api.maintenanceStatus()  // 任务68：磁盘水位（失败静默保留旧值）
  } catch { /* 静默 */ }
}

onMounted(async () => {
  await reload()
  pollTimer = setInterval(reload, 5000)  // 任务66：卡片 5s 轮询；失败保留旧值只出提示
  const close = connectEvents(
    (ev) => { feed.value = [ev, ...feed.value].slice(0, 100) },
    (s) => (wsState.value = s),
  )
  onUnmounted(close)
})
onUnmounted(() => clearInterval(pollTimer))
</script>

<template>
  <AsyncState :loading="loading" :error="error" :notice="notice" @retry="reload">
    <div class="cards" v-if="sys">
      <div class="card"><div class="muted">后端</div><div class="num">{{ sys.backend }}</div></div>
      <div class="card"><div class="muted">今日事件</div><div class="num">{{ Object.values(sys.today_counts || {}).reduce((a, b) => a + b, 0) }}</div></div>
      <div class="card"><div class="muted">未处置</div><div class="num" :style="{ color: sys.unhandled_today > 0 ? 'var(--l2)' : undefined }">{{ sys.unhandled_today ?? 0 }}</div></div>
      <div class="card"><div class="muted">警告(L2)</div><div class="num">{{ sys.today_counts?.['2'] || 0 }}</div></div>
      <div class="card"><div class="muted">严重(L3)</div><div class="num">{{ sys.today_counts?.['3'] || 0 }}</div></div>
      <div class="card"><div class="muted">实时通道</div><div class="num" :style="{ fontSize: '18px', paddingTop: '6px' }">{{ wsState }}</div></div>
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
    <h3 style="margin-bottom:10px">实时事件流（WebSocket）</h3>
    <table v-if="feed.length">
      <tr v-for="(e, i) in feed" :key="e.id ?? i" :class="{ 'new-item': i === 0 }">
        <td>{{ e.ts }}</td>
        <td>摄像头#{{ e.camera_id }}</td>
        <td>{{ TYPE_TEXT[e.type] || e.type }}</td>
        <td><span :class="'lv' + e.level">L{{ e.level }}</span></td>
        <td class="muted">{{ e.detail }}</td>
      </tr>
    </table>
    <p v-else-if="sys" class="muted">暂无实时事件 —— 去摄像头页添加/启动摄像头，触发行为会实时出现在这里</p>
    <p v-else class="muted">等待事件…（配置安全区与规则后，触发行为会实时出现在这里）</p>
  </div>
</template>

<style scoped>
/* 任务17B：375px 小屏适配（scoped，不动全局样式） */
.cards { flex-wrap: wrap; }
.card { min-width: 140px; }
.panel { overflow-x: auto; }
button { min-height: 40px; }
</style>
