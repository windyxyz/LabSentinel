<script setup>
import { ref } from 'vue'
import Dashboard from './views/Dashboard.vue'
import Cameras from './views/Cameras.vue'
import Zones from './views/Zones.vue'
import Events from './views/Events.vue'
import Reports from './views/Reports.vue'
import Face from './views/Face.vue'

// 图标为 Fluent 风格线性 SVG（16px），不依赖图标字体/库
const tabs = [
  { key: 'dash', label: '仪表盘', comp: Dashboard,
    icon: '<rect x="2" y="2" width="5" height="5" rx="1"/><rect x="9" y="2" width="5" height="5" rx="1"/><rect x="2" y="9" width="5" height="5" rx="1"/><rect x="9" y="9" width="5" height="5" rx="1"/>' },
  { key: 'cams', label: '摄像头', comp: Cameras,
    icon: '<rect x="2" y="5" width="9" height="7" rx="1.5"/><path d="M11 8.2l3-1.8v5.2l-3-1.8"/>' },
  { key: 'zones', label: '安全区配置', comp: Zones,
    icon: '<path d="M8 2l5.5 3.8v5.4L8 14.5 2.5 11.2V5.8z"/>' },
  { key: 'events', label: '事件查询', comp: Events,
    icon: '<path d="M3 4h10M3 8h10M3 12h7"/>' },
  { key: 'reports', label: '报表', comp: Reports,
    icon: '<path d="M3 13.5V9M8 13.5V4M13 13.5V10"/><path d="M2 14.5h12"/>' },
  { key: 'face', label: '人脸管理', comp: Face,
    icon: '<circle cx="8" cy="5.5" r="2.8"/><path d="M2.5 13.8c.9-2.7 3-4 5.5-4s4.6 1.3 5.5 4"/>' },
]
const active = ref('dash')
</script>

<template>
  <div class="layout">
    <nav>
      <h1>LabSentinel</h1>
      <a v-for="t in tabs" :key="t.key" :class="{ active: active === t.key }" @click="active = t.key">
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3"
             stroke-linecap="round" stroke-linejoin="round" v-html="t.icon"></svg>
        <span>{{ t.label }}</span>
      </a>
      <div style="flex:1"></div>
      <div class="muted" style="padding:10px 12px;font-size:12px">实验室哨兵 · M1</div>
    </nav>
    <main>
      <component :is="tabs.find(t => t.key === active).comp" />
    </main>
  </div>
</template>
