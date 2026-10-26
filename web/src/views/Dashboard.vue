<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import { api, connectEvents, TYPE_TEXT } from '../api'

const wsState = ref('连接中')
const sys = ref(null)
const feed = ref([])

onMounted(async () => {
  sys.value = await api.system().catch(() => null)
  const close = connectEvents(
    (ev) => { feed.value = [ev, ...feed.value].slice(0, 100) },
    (s) => (wsState.value = s),
  )
  onUnmounted(close)
})
</script>

<template>
  <div class="cards" v-if="sys">
    <div class="card"><div class="muted">后端</div><div class="num">{{ sys.backend }}</div></div>
    <div class="card"><div class="muted">今日事件</div><div class="num">{{ Object.values(sys.today_counts || {}).reduce((a, b) => a + b, 0) }}</div></div>
    <div class="card"><div class="muted">未处置</div><div class="num" :style="{ color: sys.unhandled_today > 0 ? 'var(--l2)' : undefined }">{{ sys.unhandled_today ?? 0 }}</div></div>
    <div class="card"><div class="muted">警告(L2)</div><div class="num">{{ sys.today_counts?.['2'] || 0 }}</div></div>
    <div class="card"><div class="muted">严重(L3)</div><div class="num">{{ sys.today_counts?.['3'] || 0 }}</div></div>
    <div class="card"><div class="muted">实时通道</div><div class="num" :style="{ fontSize: '18px', paddingTop: '6px' }">{{ wsState }}</div></div>
  </div>

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
