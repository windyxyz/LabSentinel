<script setup>
// UX1（2026-11-20）：仪表盘——「今日概览 / 报表」双视图（原报表一级 tab 并入）。
// Dashboard.vue 与 Reports.vue 零逻辑搬移，原样嵌入。
import { onBeforeUnmount, onMounted, ref } from 'vue'
import Dashboard from './Dashboard.vue'
import ReportsView from './ReportsView.vue'

const seg = ref('dash')
const segs = [
  { id: 'dash', label: '今日概览',  },
  { id: 'reports', label: '报表',  },
]
onMounted(() => {
  const sub = window.location.hash.split('/')[2]
  if (segs.some((s) => s.id === sub)) seg.value = sub
  // 分段跟随浏览器 hash（SPA 内 hash 直进/前进后退同步——此前只读一次，
  // goto 同页换 hash 不重挂载，报表/概览无法切换：用户实测 bug）
  window.addEventListener('hashchange', syncFromHash)
})
function syncFromHash() {
  const sub = window.location.hash.split('/')[2]
  if (segs.some((s) => s.id === sub)) seg.value = sub
}
onBeforeUnmount(() => window.removeEventListener('hashchange', syncFromHash))
function pick(id) {
  seg.value = id
  const tab = window.location.hash.split('/')[1] || 'dash'
  window.history.replaceState(null, '', `#/${tab}/${id}`)
}
</script>

<template>
  <div>
    <div class="seg" role="tablist" aria-label="仪表盘视图">
      <button v-for="s in segs" :key="s.id" role="tab" :aria-selected="seg === s.id"
              class="seg-btn" :class="{ active: seg === s.id }" @click="pick(s.id)">
        {{ s.label }}
      </button>
    </div>
    <!-- 显式分支渲染（生产构建下多根视图经动态 component 切换会渲染成空——
         2026-11-21 用户实测「报表/安全区闪黑」，Vue 3.5.44 生产 patch 规避） -->
    <Dashboard v-if="seg === 'dash'" />
    <ReportsView v-else />
  </div>
</template>

<style scoped>
.seg {
  display: inline-flex; gap: 4px; padding: 4px;
  background: var(--card-bg); border: 1px solid var(--stroke-default);
  border-radius: var(--control-radius); margin-bottom: 12px;
}
.seg-btn {
  min-height: 30px; padding: 0 14px;
  border: none; background: transparent; border-radius: calc(var(--control-radius) - 2px);
  color: var(--text-secondary); cursor: pointer; font-size: 13px;
  transition: background-color var(--fast-duration) var(--fast-out-slow-in),
              color var(--fast-duration) var(--fast-out-slow-in);
}
.seg-btn:hover { background: var(--subtle-hover); color: var(--text-primary); }
.seg-btn.active { background: var(--ctrl-fill); color: var(--text-primary); font-weight: 500; }
</style>
