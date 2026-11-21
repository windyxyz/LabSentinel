<script setup>
// UX1（2026-11-20）：监控源——摄像头 / 图片文件夹 / 安全区与规则 三合一。
// 三个子页零逻辑搬移（原视图组件原样嵌入），仅加一层分段切换；
// 旧一级 tab（摄像头/图片识别/安全区配置）由 App.vue 的 hash 重定向进入对应分段。
import { onBeforeUnmount, onMounted, ref } from 'vue'
import Cameras from './Cameras.vue'
import FolderImages from './FolderImages.vue'
import Zones from './Zones.vue'

const seg = ref('cams')
const segs = [
  { id: 'cams', label: '摄像头', comp: Cameras,
    icon: '<rect x="2" y="5" width="9" height="7" rx="1.5"/><path d="M11 8.2l3-1.8v5.2l-3-1.8"/>' },
  { id: 'images', label: '图片文件夹', comp: FolderImages,
    icon: '<rect x="2" y="3" width="12" height="10" rx="1.5"/><circle cx="5.5" cy="6.5" r="1.2"/><path d="M2.5 11l3.5-3 2.5 2 2-1.8 3 2.8"/>' },
  { id: 'zones', label: '安全区与规则', comp: Zones,
    icon: '<path d="M8 2l5.5 3.8v5.4L8 14.5 2.5 11.2V5.8z"/>' },
]
// 深链分段：#/sources/images 等（App.vue 归一化后只保留 tab，分段在此解析）
onMounted(() => {
  const sub = window.location.hash.split('/')[2]
  if (segs.some((s) => s.id === sub)) seg.value = sub
  // 分段跟随浏览器 hash（同 Insights：SPA 内 hash 直进同步）
  window.addEventListener('hashchange', syncFromHash)
})
function syncFromHash() {
  const sub = window.location.hash.split('/')[2]
  if (segs.some((s) => s.id === sub)) seg.value = sub
}
onBeforeUnmount(() => window.removeEventListener('hashchange', syncFromHash))
function pick(id) {
  seg.value = id
  // 分段同步进 hash（刷新/分享保留），不触发 App 层重定向
  const tab = window.location.hash.split('/')[1] || 'sources'
  window.history.replaceState(null, '', `#/${tab}/${id}`)
}
</script>

<template>
  <div>
    <div class="seg" role="tablist" aria-label="监控源类型">
      <button v-for="s in segs" :key="s.id" role="tab" :aria-selected="seg === s.id"
              class="seg-btn" :class="{ active: seg === s.id }" @click="pick(s.id)">
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3"
             stroke-linecap="round" stroke-linejoin="round" v-html="s.icon"></svg>
        <span>{{ s.label }}</span>
      </button>
    </div>
    <!-- 显式分支渲染（同 Insights：规避生产动态组件切换空渲染） -->
    <Cameras v-if="seg === 'cams'" />
    <FolderImages v-else-if="seg === 'images'" />
    <Zones v-else />
  </div>
</template>

<style scoped>
.seg {
  display: inline-flex; gap: 4px; padding: 4px;
  background: var(--card-bg); border: 1px solid var(--stroke-default);
  border-radius: var(--control-radius); margin-bottom: 12px;
}
.seg-btn {
  display: inline-flex; align-items: center; gap: 7px; min-height: 30px; padding: 0 12px;
  border: none; background: transparent; border-radius: calc(var(--control-radius) - 2px);
  color: var(--text-secondary); cursor: pointer; font-size: 13px;
  transition: background-color var(--fast-duration) var(--fast-out-slow-in),
              color var(--fast-duration) var(--fast-out-slow-in);
}
.seg-btn:hover { background: var(--subtle-hover); color: var(--text-primary); }
.seg-btn.active { background: var(--ctrl-fill); color: var(--text-primary); font-weight: 500; }
.seg-btn svg { width: 14px; height: 14px; opacity: 0.9; }
</style>
