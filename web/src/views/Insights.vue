<script setup>
// UX1（2026-11-20）：仪表盘——「今日概览 / 报表」双视图（原报表一级 tab 并入）。
// Dashboard.vue 与 Reports.vue 零逻辑搬移，原样嵌入。
import { onMounted, ref } from 'vue'
import Dashboard from './Dashboard.vue'
import Reports from './Reports.vue'

const seg = ref('dash')
const segs = [
  { id: 'dash', label: '今日概览', comp: Dashboard },
  { id: 'reports', label: '报表', comp: Reports },
]
onMounted(() => {
  const sub = window.location.hash.split('/')[2]
  if (segs.some((s) => s.id === sub)) seg.value = sub
})
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
    <component :is="segs.find((s) => s.id === seg).comp" />
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
