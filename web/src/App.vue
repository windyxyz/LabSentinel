<script setup>
import { ref } from 'vue'
import Insights from './views/Insights.vue'
import Sources from './views/Sources.vue'
import Events from './views/Events.vue'
import AlertSettings from './views/AlertSettings.vue'
import Settings from './views/Settings.vue'
import Wall from './views/Wall.vue'
import Welcome from './components/Welcome.vue'

// UX1（2026-11-20）：信息架构简化——主导航 11 → 5（+仪表盘内报表 = 使用面 6 入口）。
// 低频管理面（模型/设备/人脸/插件/令牌）收进设置中心；图片识别与安全区配置并入
// 「监控源」；报表并入「仪表盘」双视图。
// hash 深链 + 旧路径兼容：#/cams/#/images/#/zones → #/sources（分段）；
// #/reports → #/dash/reports；#/models/#/devices/#/face → #/settings（对应分区）。
const tabs = [
  { key: 'dash', label: '仪表盘', comp: Insights,
    icon: '<rect x="2" y="2" width="5" height="5" rx="1"/><rect x="9" y="2" width="5" height="5" rx="1"/><rect x="2" y="9" width="5" height="5" rx="1"/><rect x="9" y="9" width="5" height="5" rx="1"/>' },
  { key: 'sources', label: '监控源', comp: Sources,
    icon: '<rect x="2" y="5" width="9" height="7" rx="1.5"/><path d="M11 8.2l3-1.8v5.2l-3-1.8"/><path d="M3 3h6"/>' },
  { key: 'events', label: '事件', comp: Events,
    icon: '<path d="M3 4h10M3 8h10M3 12h7"/>' },
  { key: 'alerts', label: '告警', comp: AlertSettings,
    icon: '<path d="M8 2.5a3.5 3.5 0 0 1 3.5 3.5v3l1.5 2.5H3l1.5-2.5v-3A3.5 3.5 0 0 1 8 2.5z"/><path d="M6.5 13a1.8 1.8 0 0 0 3 0"/>' },
  { key: 'settings', label: '设置', comp: Settings,
    icon: '<path d="M3 5h10M3 11h10"/><circle cx="6" cy="5" r="1.6"/><circle cx="10" cy="11" r="1.6"/>' },
]
// 旧 tab key → 新位置（tab + 子分段/设置分区，replace 重定向不允许 404）
const LEGACY = {
  cams: '#/sources/cams', images: '#/sources/images', zones: '#/sources/zones',
  reports: '#/dash/reports', face: '#/settings/face', models: '#/settings/detect',
  devices: '#/settings/devices',
}
function normalizeHash() {
  const h = window.location.hash.replace(/^#\/?/, '')
  const key = h.split('/')[0]
  if (LEGACY[key]) window.history.replaceState(null, '', LEGACY[key])
}
normalizeHash()
function hashTab() {
  const key = window.location.hash.replace(/^#\/?/, '').split('/')[0]
  return tabs.some((t) => t.key === key) ? key : 'dash'
}
const active = ref(hashTab())
const showWelcome = ref(true)
// T-709：值班大屏路由（无 vue-router 的轻量判定）——/wall 全屏无导航
const isWall = window.location.pathname.replace(/\/+$/, '') === '/wall'
function go(key) {
  active.value = key
  window.history.replaceState(null, '', `#/${key}`)
}
// 浏览器前进/后退时同步高亮（replaceState 不产生历史，仅外部 hash 变化触发）
window.addEventListener('hashchange', () => {
  normalizeHash()
  active.value = hashTab()
})
</script>

<template>
  <Wall v-if="isWall" />
  <div v-else class="layout">
    <nav>
      <h1><img class="logo" src="/icons/icon-512-app.png" alt="LabSentinel 图标" />LabSentinel</h1>
      <a v-for="t in tabs" :key="t.key" :class="{ active: active === t.key }"
         :href="`#/${t.key}`" @click.prevent="go(t.key)">
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3"
             stroke-linecap="round" stroke-linejoin="round" v-html="t.icon"></svg>
        <span>{{ t.label }}</span>
      </a>
      <div style="flex:1"></div>
      <div class="muted" style="padding:10px 12px;font-size:12px">实验室哨兵 · M1</div>
    </nav>
    <main>
      <!-- 显式分支渲染（生产构建动态组件切换空渲染规避，2026-11-21） -->
      <Insights v-if="active === 'dash'" />
      <Sources v-else-if="active === 'sources'" />
      <Events v-else-if="active === 'events'" />
      <AlertSettings v-else-if="active === 'alerts'" />
      <Settings v-else />
    </main>
    <Welcome v-if="showWelcome" @done="() => (showWelcome = false)" />
  </div>
</template>

<style scoped>
nav a { cursor: pointer; }
nav .logo {
  width: 26px; height: 26px;
  border-radius: 6px; object-fit: contain;
  flex-shrink: 0;
}
</style>
