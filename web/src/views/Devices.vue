<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { api } from '../api'
import AsyncState from '../components/AsyncState.vue'

const devices = ref([])
const cameras = ref([])
const loading = ref(true)
const loadError = ref('')
const cameraListUnavailable = ref(false)
const busy = ref(false)
const message = ref('')
const editing = ref(null)
const confirming = ref(null)
const pairing = ref(null)
const remaining = ref(0)
const address = ref(location.origin)
const defaults = () => ({ role: 'read_only', all_cameras: true, camera_ids: [], groups: [], expires_days: 30 })
const form = ref(defaults())
const groups = computed(() => [...new Set([...cameras.value.map(camera => camera.group_name).filter(Boolean), ...form.value.groups])].sort())
let timer

async function load() {
  loading.value = true
  loadError.value = ''
  try {
    const [rows, sources] = await Promise.all([api.listDevices(), api.cameras().catch(() => null)])
    if (!Array.isArray(rows)) throw new Error('服务未返回设备列表。')
    devices.value = rows.filter(device => device && device.platform !== 'share')
    cameraListUnavailable.value = !Array.isArray(sources)
    if (Array.isArray(sources)) cameras.value = sources.filter(camera => camera && camera.type !== 'folder')
  } catch (err) { loadError.value = String(err.message || err) }
  finally { loading.value = false }
}

function edit(device) {
  const policy = device.policy
  editing.value = device.id
  form.value = { name: device.name, role: policy.role, all_cameras: policy.all_cameras === true,
    camera_ids: Array.isArray(policy.camera_ids) ? [...policy.camera_ids] : [],
    groups: Array.isArray(policy.groups) ? [...policy.groups] : [],
    expires_days: Math.max(1, Math.min(365, Math.ceil((policy.expires_at - Date.now() / 1000) / 86400))) }
  message.value = ''
}

function finishEdit() {
  editing.value = null
  form.value = defaults()
}

function pairingOrigin() {
  let url
  try { url = new URL(address.value.trim()) } catch { throw new Error('请输入完整的 LabSentinel 访问地址。') }
  const loopback = ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
  if (url.username || url.password || !(url.protocol === 'https:' || (url.protocol === 'http:' && loopback))) {
    throw new Error('跨设备配对请填写 HTTPS 访问地址。')
  }
  return url.origin
}

function startCountdown(expires) {
  clearInterval(timer)
  const update = () => {
    remaining.value = Math.max(0, Math.ceil((expires - Date.now()) / 1000))
    if (remaining.value === 0) clearInterval(timer)
  }
  timer = setInterval(update, 1000)
  update()
}

async function save() {
  if (busy.value) return
  busy.value = true
  message.value = ''
  try {
    if (editing.value !== null) {
      await api.editDevice(editing.value, form.value)
      finishEdit()
      await load()
      message.value = '设备授权已更新。'
      return
    }
    const origin = pairingOrigin()
    if (pairing.value && remaining.value > 0) await api.cancelPairing(pairing.value.code)
    pairing.value = null
    const result = await api.createPairing(form.value)
    if (!/^[0-9]{6}$/.test(result?.code) || !Number.isFinite(result.expires_in_s)) {
      throw new Error('服务未返回有效的配对码。')
    }
    const link = `${origin}/#/pair?code=${result.code}`
    pairing.value = { code: result.code, link, qr: '' }
    startCountdown(Date.now() + result.expires_in_s * 1000)
    const { default: qr } = await import('qrcode')
    pairing.value.qr = await qr.toDataURL(link, { width: 184, margin: 3 })
  } catch (err) { message.value = String(err.message || err) }
  finally { busy.value = false }
}

async function cancelCode() {
  if (!pairing.value || busy.value) return
  busy.value = true
  try {
    await api.cancelPairing(pairing.value.code)
    pairing.value = null
    clearInterval(timer)
    message.value = '配对码已撤销。'
  } catch (err) { message.value = String(err.message || err) }
  finally { busy.value = false }
}

async function revoke(device) {
  if (busy.value) return
  busy.value = true
  try {
    await api.revokeDevice(device.id)
    confirming.value = null
    if (editing.value === device.id) finishEdit()
    await load()
    message.value = '设备访问已撤销。'
  } catch (err) { message.value = String(err.message || err) }
  finally { busy.value = false }
}

function scopeText(policy) {
  if (!policy) return '旧版令牌'
  if (policy.all_cameras) return '所有摄像头'
  const names = (Array.isArray(policy.camera_ids) ? policy.camera_ids : [])
    .map(id => cameras.value.find(camera => camera.id === id)?.name || `摄像头 #${id}`)
  const selectedGroups = Array.isArray(policy.groups) ? policy.groups.map(group => `${group} 组`) : []
  return [...names, ...selectedGroups].join('、') || '没有授权摄像头'
}

const date = (ts) => ts ? new Date(typeof ts === 'number' ? ts * 1000 : ts).toLocaleString('zh-CN') : '从未访问'
onMounted(load)
onBeforeUnmount(() => clearInterval(timer))
</script>

<template>
  <div class="devices-page">
    <header class="device-toolbar">
      <h2>设备与配对</h2>
      <a href="#/pair">输入配对码</a>
    </header>
    <AsyncState :loading="loading" :error="loadError" @retry="load">
      <div class="device-layout">
        <form class="device-form" @submit.prevent="save">
          <h3>{{ editing === null ? '配对新设备' : '编辑设备授权' }}</h3>
          <label v-if="editing === null">设备访问地址
            <input v-model="address" type="url" required />
            <small class="muted">手机和平板请填写它们可以打开的 HTTPS 地址。</small>
          </label>
          <label v-else>设备名称 <input v-model="form.name" maxlength="64" required /></label>
          <label>权限
            <select v-model="form.role">
              <option value="read_only">只读值守</option>
              <option value="triage">值守与事件处置</option>
            </select>
          </label>
          <label>摄像头范围
            <select v-model="form.all_cameras">
              <option :value="true">所有摄像头</option>
              <option :value="false">指定摄像头与分组</option>
            </select>
          </label>
          <fieldset v-if="!form.all_cameras" class="scope-options">
            <legend>允许访问</legend>
            <label v-for="group in groups" :key="group"><input v-model="form.groups" type="checkbox" :value="group" />{{ group }} 组</label>
            <label v-for="camera in cameras" :key="camera.id"><input v-model="form.camera_ids" type="checkbox" :value="camera.id" />{{ camera.name }}</label>
            <p v-if="cameraListUnavailable" class="muted">暂时无法读取摄像头列表，可点「刷新」重试。</p>
            <p v-else-if="!cameras.length" class="muted">添加摄像头后可以选定访问范围。</p>
          </fieldset>
          <label>有效期（天） <input v-model.number="form.expires_days" type="number" min="1" max="365" required /></label>
          <div class="row device-actions">
            <button type="submit" :disabled="busy">{{ editing === null ? '生成配对码' : '保存授权' }}</button>
            <button v-if="editing !== null" type="button" class="ghost" :disabled="busy" @click="finishEdit">取消编辑</button>
          </div>
          <div v-if="pairing && editing === null" class="pairing-box">
            <template v-if="remaining > 0">
              <img v-if="pairing.qr" :src="pairing.qr" alt="设备配对二维码" />
              <div>
                <p>配对码 <strong class="pair-code">{{ pairing.code }}</strong></p>
                <p class="muted">{{ remaining }} 秒后到期，可认领一次。</p>
                <label class="pair-url">配对链接 <input :value="pairing.link" readonly @focus="$event.target.select()" /></label>
                <button type="button" class="ghost" :disabled="busy" @click="cancelCode">撤销配对码</button>
              </div>
            </template>
            <p v-else class="muted">配对码已过期。</p>
          </div>
        </form>

        <section class="device-list" aria-labelledby="paired-devices-heading">
          <div class="device-toolbar">
            <h3 id="paired-devices-heading">已配对设备</h3>
            <button type="button" class="ghost" :disabled="busy" @click="load">刷新</button>
          </div>
          <p v-if="!devices.length" class="muted">暂无配对设备。</p>
          <article v-for="device in devices" :key="device.id" class="device-entry">
            <header class="device-toolbar">
              <strong>{{ device.name }}</strong>
              <span class="muted">{{ device.revoked ? '已撤销' : device.expired ? '已到期' : device.policy ? device.policy.role === 'triage' ? '事件处置' : '只读值守' : '旧版设备' }}</span>
            </header>
            <p>{{ scopeText(device.policy) }}</p>
            <p class="muted">最近访问 {{ date(device.last_seen_ts) }}<template v-if="device.policy"> · 到期 {{ date(device.policy.expires_at) }}</template></p>
            <div v-if="!device.revoked" class="row device-actions">
              <button v-if="device.policy && confirming !== device.id" type="button" class="ghost" :disabled="busy" @click="edit(device)">编辑授权</button>
              <template v-if="confirming === device.id">
                <button type="button" class="cancel-device" :disabled="busy" @click="revoke(device)">确认撤销</button>
                <button type="button" class="ghost" :disabled="busy" @click="confirming = null">取消</button>
              </template>
              <button v-else type="button" class="ghost" :disabled="busy" @click="confirming = device.id">撤销设备</button>
            </div>
          </article>
        </section>
      </div>
    </AsyncState>
    <p v-if="message" class="device-status" role="status">{{ message }}</p>
  </div>
</template>

<style scoped>
h2 { font-size: 18px; }
.device-toolbar { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px; }
.device-toolbar a { color: var(--accent-strong); }
.device-layout { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 16px; margin-top: 12px; }
.device-form, .device-list { display: grid; gap: 10px; min-width: 0; align-content: start; }
.device-form > label, .pair-url { display: grid; gap: 4px; }
.device-form input, .device-form select { min-width: 0; margin: 0; }
input[type="number"] { width: 90px; }
.scope-options { display: grid; gap: 4px; max-height: 180px; overflow: auto; border: 1px solid var(--stroke-secondary); padding: 5px 10px; min-width: 0; }
.scope-options label { display: flex; gap: 8px; align-items: center; }
.pairing-box { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 10px; border: 1px solid var(--stroke-secondary); border-radius: var(--control-radius); }
.pairing-box img { width: 148px; height: 148px; }
.pairing-box > div { flex: 1 1 160px; min-width: 0; }
.pair-code { font-variant-numeric: tabular-nums; letter-spacing: .14em; font-size: 22px; }
.pair-url { margin: 6px 0; }
.device-entry { border: 1px solid var(--stroke-secondary); border-radius: var(--control-radius); padding: 10px; overflow-wrap: anywhere; }
.device-entry > p { margin-top: 3px; }
.device-entry .device-actions { margin-top: 7px; }
.device-actions { margin-bottom: 0; }
.device-status { margin-top: 8px; }
.cancel-device { background: var(--sys-critical-bg); color: var(--sys-critical); border-color: var(--sys-critical); }
.cancel-device:hover { background: var(--sys-critical-bg); }
@media (max-width: 1050px) { .device-layout { grid-template-columns: 1fr; } }
</style>
