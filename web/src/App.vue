<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { api, setBrowserSessionMode } from './api'
import AccessLogin from './components/AccessLogin.vue'
import DevicePair from './components/DevicePair.vue'
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
const logoUrl = '/icons/icon-512-app.png'
// T-709：值班大屏路由（无 vue-router 的轻量判定）——/wall 全屏无导航
// LS-054：只读值守分享链接——URL 携带 readonly_token 时校验后锁定只读大屏
const shareStatus = ref('')
function activateShare() {
  // 支持 /wall?readonly_token=... 与 #/wall?readonly_token=...
  const fromSearch = new URLSearchParams(window.location.search).get('readonly_token')
  const hash = window.location.hash
  const qi = hash.indexOf('?')
  const fromHash = qi >= 0 ? new URLSearchParams(hash.slice(qi + 1)).get('readonly_token') : null
  const token = (fromSearch || fromHash || sessionStorage.getItem('ls_readonly_token') || '').trim()
  if (token) {
    sessionStorage.setItem('ls_readonly_token', token)
    shareStatus.value = 'checking'
    if (fromSearch || fromHash) window.history.replaceState(null, '', '/#/wall')
  }
  return token
}
let readonlyToken = activateShare()
let shareTimer
let accessTimer
let accessRequest
let lastAccessCheck = 0
const access = ref(null)
const accessError = ref('')
const signOutError = ref('')
const wallPath = ref(isWallPath())
const recoveryPath = ref(location.hash === '#/recover')
const pairingRoute = ref(readPairingRoute())
const isWall = computed(() => Boolean(shareStatus.value) || wallPath.value)
function isWallPath() {
  return location.pathname.replace(/\/+$/, '') === '/wall' || location.hash.split('?')[0] === '#/wall'
}
function readPairingRoute() {
  const [path, query] = location.hash.split('?')
  if (path !== '#/pair') return { active: false, code: '' }
  const code = new URLSearchParams(query).get('code') || ''
  if (query) history.replaceState(null, '', '/#/pair')
  return { active: true, code: /^[0-9]{6}$/.test(code) ? code : '' }
}
function go(key) {
  active.value = key
  window.history.replaceState(null, '', `#/${key}`)
}
// 浏览器前进/后退时同步高亮（replaceState 不产生历史，仅外部 hash 变化触发）
function onHashChange() {
  normalizeHash()
  active.value = hashTab()
  wallPath.value = isWallPath()
  recoveryPath.value = location.hash === '#/recover'
  pairingRoute.value = readPairingRoute()
  if (new URLSearchParams(location.hash.split('?')[1]).has('readonly_token')) {
    readonlyToken = activateShare()
    verifyShare()
  }
}

// LS-054：分享令牌有效性校验（过期/吊销则清除只读态并提示）
async function verifyShare() {
  if (!readonlyToken) return
  try {
    const r = await api.verifyShareLink(readonlyToken)
    if (r?.valid === true) shareStatus.value = 'valid'
    else {
      shareStatus.value = 'invalid'
      sessionStorage.removeItem('ls_readonly_token')
      readonlyToken = ''
    }
  } catch {
    shareStatus.value = 'unavailable'
  }
}

function refreshAccess() {
  if (shareStatus.value || accessRequest) return accessRequest
  lastAccessCheck = Date.now()
  accessRequest = api.accessStatus()
    .then(status => {
      if (typeof status?.required !== 'boolean' || typeof status.signed_in !== 'boolean') {
        throw new Error('服务未返回访问状态。')
      }
      setBrowserSessionMode(status.required || status.signed_in || !!status.device || status.device_expired === true)
      access.value = status
      accessError.value = ''
    })
    .catch(err => { accessError.value = String(err.message || err) })
    .finally(() => { accessRequest = null })
  return accessRequest
}

async function signOut() {
  signOutError.value = ''
  try {
    if (access.value?.device || access.value?.device_expired) {
      await api.deviceLogout()
      location.hash = '#/dash'
    } else await api.accessLogout()
    await accessRequest
    await refreshAccess()
  } catch (err) { signOutError.value = String(err.message || err) }
}

async function signedIn() {
  await accessRequest
  await refreshAccess()
  if (access.value?.signed_in && recoveryPath.value) location.hash = '#/settings/access'
}

async function devicePaired() {
  await accessRequest
  await refreshAccess()
  if (access.value?.device) location.hash = '#/wall'
}

function leaveRecovery() {
  location.hash = access.value?.required && !access.value?.signed_in ? '#/dash' : '#/settings/access'
}

function onVisibilityChange() {
  if (!document.hidden) refreshAccess()
}

onMounted(() => {
  window.addEventListener('hashchange', onHashChange)
  window.addEventListener('ls-access-changed', refreshAccess)
  document.addEventListener('visibilitychange', onVisibilityChange)
  if (readonlyToken) verifyShare()
  else refreshAccess()
  shareTimer = setInterval(verifyShare, 15000)
  accessTimer = setInterval(() => {
    if (document.hidden || !access.value) return
    const interval = access.value.device ? 5000 : 30000
    if (Date.now() - lastAccessCheck >= interval) refreshAccess()
  }, 5000)
})
onBeforeUnmount(() => {
  window.removeEventListener('hashchange', onHashChange)
  window.removeEventListener('ls-access-changed', refreshAccess)
  document.removeEventListener('visibilitychange', onVisibilityChange)
  clearInterval(shareTimer)
  clearInterval(accessTimer)
  setBrowserSessionMode(false)
})
</script>

<template>
  <p v-if="signOutError" class="sign-out-error" role="alert">{{ signOutError }}</p>
  <div v-if="shareStatus && shareStatus !== 'valid'" class="card" role="status">
    {{ { checking: '正在验证分享链接…', invalid: '分享链接已过期或被吊销。', unavailable: '暂时无法验证分享链接。' }[shareStatus] }}
  </div>
  <Wall v-else-if="shareStatus === 'valid'" :readonly="true" />
  <main v-else-if="!access" class="access-loading">
    <div class="panel">
      <p v-if="accessError" role="alert">{{ accessError }}</p>
      <p v-else role="status">正在连接 LabSentinel…</p>
      <button v-if="accessError" type="button" class="ghost" @click="refreshAccess">重试</button>
    </div>
  </main>
  <DevicePair v-else-if="pairingRoute.active" :key="pairingRoute.code"
              :code="pairingRoute.code" :can-pair="access.password_supported" @paired="devicePaired" />
  <main v-else-if="access.device_expired" class="access-loading">
    <div class="panel">
      <h2>设备配对已失效</h2>
      <div class="row">
        <a href="#/pair">重新配对</a>
        <button type="button" class="ghost" @click="signOut">返回控制台</button>
      </div>
    </div>
  </main>
  <Wall v-else-if="access.device" :key="`${access.device.id}/${access.device.scope_version}`"
        :device="access.device" :readonly="true" @sign-out="signOut" />
  <AccessLogin v-else-if="(access.required && !access.signed_in) || (recoveryPath && access.local)"
               :key="recoveryPath ? 'recovery' : 'login'" :start-recovery="recoveryPath && access.local"
               :local="access.local" :password-supported="access.password_supported"
               @signed-in="signedIn" @leave-recovery="leaveRecovery" />
  <Wall v-else-if="isWall" :signed-in="access.signed_in" @sign-out="signOut" />
  <div v-else class="layout">
    <nav>
      <h1><img class="logo" :src="logoUrl" alt="LabSentinel 图标" />LabSentinel</h1>
      <a v-for="t in tabs" :key="t.key" :class="{ active: active === t.key }"
         :href="`#/${t.key}`" @click.prevent="go(t.key)">
        <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3"
             stroke-linecap="round" stroke-linejoin="round" v-html="t.icon"></svg>
        <span>{{ t.label }}</span>
      </a>
      <button v-if="access.signed_in" type="button" class="ghost sign-out" @click="signOut">退出登录</button>
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
.access-loading { min-height: 100vh; max-width: none; display: grid; place-items: center; }
.access-loading button { margin-top: 8px; }
.sign-out { margin: auto 4px 4px; }
.sign-out-error { position: fixed; right: 14px; bottom: 14px; z-index: 25; padding: 10px 12px;
  background: var(--dialog-bg); color: var(--sys-critical); border-radius: var(--control-radius); }
@media (max-width: 760px) { .sign-out { margin: 0 0 0 auto; white-space: nowrap; } }
</style>
