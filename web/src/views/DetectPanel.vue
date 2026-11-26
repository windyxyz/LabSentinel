<script setup>
// UX2（2026-11-20）：设置 → 模型与检测——三模式卡片（仅本地 / 仅 API /
// 本地+API 协作）+ API 服务配置 + 协作策略与用量。
// 0.4.0：API 服务支持多接入档案（保存多个上游随时切换）+ 连通性测试 +
// 自动拉取上游模型列表。
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'
import Models from './Models.vue'

const loading = ref(true)
const mode = ref('local')
const cfg = ref(null)
const usage = ref(null)
const msg = ref('')
const busy = ref(false)

// ---- API 接入档案 ----
const profiles = ref([])
const activeId = ref('')
const editId = ref('') // '' = 新建
const pName = ref('')
const provider = ref('openai_compat')
const url = ref('')
const model = ref('')
const apiKey = ref('') // 输入框：留空=不变更；掩码 ***
const apiKeySet = ref(false)
const timeoutS = ref(30)
const allowRemote = ref(false)
const modelOptions = ref([])
const fetchBusy = ref(false)
const profileBusy = ref(false)

// ---- 协作策略卡 ----
const trigger = ref('both')
const dailyBudget = ref(200)
const personWindow = ref(300)
const onNegative = ref('keep')

const testBusy = ref(false)
const testResult = ref(null)
const apiCardEl = ref(null)
const urlInputEl = ref(null)
const runtimeEp = ref('dml')
const runtimeAdapter = ref(null)
const runtimeGpuId = ref('auto')
const manualVulkanId = ref('')
const vulkanEnabled = computed(() => ['auto_vulkan', 'vulkan'].includes(runtimeEp.value))
const vulkanAvailable = computed(() => cfg.value?.runtime?.vulkan_available === true)
const cameraDevices = computed(() => Array.isArray(cfg.value?.runtime?.camera_devices) ? cfg.value.runtime.camera_devices : [])
const adapters = computed(() => Array.isArray(cfg.value?.runtime?.adapters) ? cfg.value.runtime.adapters : [])
const directMlAdapters = computed(() => adapters.value.filter(adapter => Number.isInteger(adapter?.dxgi_index)))
const selectedAdapter = computed(() => directMlAdapters.value.find(adapter => adapter.dxgi_index === runtimeAdapter.value))
const physicalAdapters = computed(() => adapters.value.filter(adapter =>
  Number.isInteger(adapter?.vulkan_index) && typeof adapter?.gpu_id === 'string'
))
const physicalAdapter = computed(() => physicalAdapters.value.find(adapter => adapter.gpu_id === runtimeGpuId.value))
const sameDirectMlAdapter = computed(() => vulkanEnabled.value && (
  Number.isInteger(physicalAdapter.value?.dxgi_index)
    && physicalAdapter.value.dxgi_index === cfg.value?.runtime?.effective_adapter_index
))
const sensitivityPresets = [
  { id: 'conservative', label: '保守' },
  { id: 'balanced', label: '平衡' },
  { id: 'sensitive', label: '敏感' },
]

// ---- LS-050：自定义 VLM 提示词 + 性能开关（Motion Gate）----
const vlmPrompt = ref('')
const motionGate = ref(false)
const motionThreshold = ref(2)
const motionRecording = ref(false)

const MODES = [
  { id: 'local', title: '仅本地', desc: '零成本离线——ONNX 本地推理，数据不出本机', icon: '<rect x="2.5" y="4" width="11" height="8" rx="1.5"/><path d="M5 13.5h6"/>' },
  { id: 'api', title: '仅 API', desc: '全量精判——所有画面交外部联网模型判定', icon: '<circle cx="8" cy="8" r="2.2"/><path d="M8 2v2.4M8 11.6V14M2 8h2.4M11.6 8H14"/>' },
  { id: 'cascade', title: '本地 + API 协作 ⭐', desc: '省费用推荐——本地先扫人，可疑帧才交 API 精判', icon: '<path d="M2.5 4h4v8h-4zM9.5 4h4v8h-4z"/><path d="M6.5 8h3"/>' },
]

const activeProfile = computed(() => profiles.value.find((p) => p.id === activeId.value) || null)
const isRemoteUrl = computed(() => {
  const m = /^https?:\/\/([^/:]+)/.exec(url.value.trim())
  if (!m) return false
  const h = m[1].toLowerCase()
  return !['127.0.0.1', 'localhost', '::1', '[::1]'].includes(h)
})

function fillForm(p) {
  if (!p) {
    pName.value = ''
    provider.value = 'openai_compat'
    url.value = ''
    model.value = ''
    apiKey.value = ''
    apiKeySet.value = false
    timeoutS.value = 30
    allowRemote.value = false
    modelOptions.value = []
    return
  }
  pName.value = p.name || ''
  provider.value = p.provider || 'openai_compat'
  url.value = p.url || ''
  model.value = p.model || ''
  apiKey.value = ''
  apiKeySet.value = !!p.api_key_set
  timeoutS.value = p.timeout_s || 30
  allowRemote.value = !!p.allow_remote
  modelOptions.value = []
}

function selectProfile(id) {
  editId.value = id
  const p = profiles.value.find((x) => x.id === id)
  fillForm(p)
  msg.value = ''
}

function newProfile() {
  editId.value = ''
  fillForm(null)
  msg.value = ''
}

async function load() {
  loading.value = true
  try {
    const d = await api.detectConfig()
    mode.value = d.mode
    cfg.value = d
    trigger.value = d.cascade?.trigger || 'both'
    dailyBudget.value = d.cascade?.daily_budget ?? 200
    personWindow.value = d.cascade?.person_window_s ?? 300
    onNegative.value = d.cascade?.on_negative || 'keep'
    usage.value = d.usage
    runtimeEp.value = d.runtime?.backend || d.runtime?.ep || 'dml'
    runtimeAdapter.value = Number.isInteger(d.runtime?.adapter_index) ? d.runtime.adapter_index : null
    runtimeGpuId.value = typeof d.runtime?.gpu_id === 'string' ? d.runtime.gpu_id : 'auto'
    manualVulkanId.value = runtimeGpuId.value.startsWith('vulkan:') ? runtimeGpuId.value : ''
    vlmPrompt.value = d.vlm_prompt || ''
    motionGate.value = !!d.perf?.motion_gate_enabled
    motionThreshold.value = d.perf?.motion_threshold ?? 2
    motionRecording.value = d.perf?.motion_event_recording === true
    // 档案接口失败（旧服务端/网络抖动）时回退旧单配置，页面仍可用
    let pf = { profiles: [], active: '' }
    try {
      pf = await api.detectProfiles()
    } catch { /* 回退到旧配置视图 */ }
    profiles.value = Array.isArray(pf?.profiles) ? pf.profiles : []
    activeId.value = pf?.active || ''
    if (profiles.value.length) {
      // 编辑当前启用档案；若之前正在编辑另一个则保持选择
      const keep = profiles.value.find((p) => p.id === editId.value)
      selectProfile((keep || activeProfile.value || profiles.value[0]).id)
    } else {
      editId.value = ''
      fillForm(null)
      url.value = d.vlm?.url || ''
      provider.value = d.vlm?.provider || 'openai_compat'
      model.value = d.vlm?.model || ''
      apiKeySet.value = !!d.vlm?.api_key_set
      timeoutS.value = d.vlm?.timeout_s || 30
      allowRemote.value = !!d.vlm?.allow_remote
    }
    msg.value = ''
  } catch (e) {
    msg.value = String(e.message || e)
  }
  loading.value = false
}

async function save(payload, okText = '已保存') {
  busy.value = true
  msg.value = ''
  try {
    const r = await api.saveDetectConfig(payload)
    await load()
    if (r.restarted > 0) okText += `（${r.restarted} 路跟随全局的相机已重启生效）`
    msg.value = okText
  } catch (e) {
    msg.value = String(e.message || e)
  }
  busy.value = false
}

function pickMode(id) {
  if (id === mode.value) return
  if (id !== 'local' && (!url.value || (isRemoteUrl.value && !allowRemote.value))) {
    msg.value = '先在下方「API 服务」填写地址并保存（远程地址需勾选合规确认），然后再点模式卡片切换'
    try {
      apiCardEl.value?.scrollIntoView?.({ behavior: 'smooth', block: 'center' })
      setTimeout(() => urlInputEl.value?.focus(), 350)
    } catch { /* 非浏览器环境忽略 */ }
    return
  }
  save({ mode: id }, id === 'cascade' ? '已切换为本地+API 协作' : '已切换检测模式')
}

async function saveProfile(activate = true) {
  if (!url.value.trim()) {
    msg.value = '请填写 API 地址'
    return
  }
  profileBusy.value = true
  msg.value = ''
  try {
    const body = {
      name: pName.value.trim() || 'API 服务',
      provider: provider.value,
      url: url.value.trim(),
      model: model.value.trim(),
      timeout_s: timeoutS.value,
      allow_remote: allowRemote.value,
      activate,
    }
    if (editId.value) body.id = editId.value
    if (apiKey.value.trim()) body.api_key = apiKey.value.trim()
    const r = await api.saveDetectProfile(body)
    await load()
    selectProfile(r.profile?.id || editId.value)
    msg.value = activate
      ? `已保存并启用${r.restarted ? `（${r.restarted} 路相机已重启）` : ''}`
      : '档案已保存'
  } catch (e) {
    msg.value = String(e.message || e)
  } finally {
    profileBusy.value = false
  }
}

async function useProfile() {
  if (!editId.value) {
    msg.value = '先保存该档案'
    return
  }
  profileBusy.value = true
  try {
    const r = await api.activateDetectProfile(editId.value)
    activeId.value = r.active
    await load()
    msg.value = `已切换为「${pName.value || r.active}」${r.restarted ? `（${r.restarted} 路相机已重启）` : ''}`
  } catch (e) {
    msg.value = String(e.message || e)
  } finally {
    profileBusy.value = false
  }
}

async function removeProfile() {
  if (!editId.value) return
  if (!confirm(`删除接入「${pName.value || editId.value}」？`)) return
  profileBusy.value = true
  try {
    await api.deleteDetectProfile(editId.value)
    editId.value = ''
    await load()
    msg.value = '档案已删除'
  } catch (e) {
    msg.value = String(e.message || e)
  } finally {
    profileBusy.value = false
  }
}

async function fetchModels() {
  if (!url.value.trim()) {
    msg.value = '请先填写 API 地址'
    return
  }
  fetchBusy.value = true
  testResult.value = null
  msg.value = ''
  try {
    const r = await api.detectModels({
      id: editId.value || undefined,
      provider: provider.value,
      url: url.value.trim(),
      api_key: apiKey.value.trim() || undefined,
      timeout_s: timeoutS.value,
    })
    modelOptions.value = r.models || []
    testResult.value = { ok: true, elapsed_ms: r.elapsed_ms, count: r.count }
    if (!model.value && modelOptions.value.length) model.value = modelOptions.value[0]
    msg.value = `连接成功：拉到 ${r.count} 个模型（${r.elapsed_ms}ms）`
  } catch (e) {
    testResult.value = { ok: false, detail: String(e.message || e) }
    msg.value = `连接失败：${e.message || e}`
  } finally {
    fetchBusy.value = false
  }
}

async function clearKey() {
  if (!editId.value) {
    msg.value = '先保存档案后再清除密钥'
    return
  }
  profileBusy.value = true
  try {
    await api.saveDetectProfile({ id: editId.value, api_key: '', activate: false })
    apiKey.value = ''
    await load()
    msg.value = '密钥已清除'
  } catch (e) {
    msg.value = String(e.message || e)
  } finally {
    profileBusy.value = false
  }
}

function saveCascade() {
  save({ cascade: {
    trigger: trigger.value,
    daily_budget: dailyBudget.value,
    person_window_s: personWindow.value,
    on_negative: onNegative.value,
  } }, '协作策略已保存')
}

async function runTest() {
  testBusy.value = true
  testResult.value = null
  try {
    testResult.value = await api.detectTest()
  } catch (e) {
    testResult.value = { ok: false, detail: String(e.message || e) }
  }
  testBusy.value = false
  load()
}

// 推理设备切换（GPU/CPU）：保存后全部运行中相机软重启（模型重新加载）
async function saveRuntime(patch) {
  busy.value = true
  msg.value = ''
  try {
    const r = await api.saveDetectConfig({ runtime: patch })
    await load()
    msg.value = `已保存推理设备设置，${r.restarted || 0} 路分析源已重新启动。`
  } catch (e) {
    await load()
    msg.value = String(e.message || e)
  }
  busy.value = false
}

async function refreshRuntimeDevices() {
  try {
    const current = await api.detectConfig()
    if (cfg.value?.runtime && current?.runtime) {
      cfg.value.runtime.camera_devices = current.runtime.camera_devices
    }
  } catch (e) {
    msg.value = String(e.message || e)
  }
}

// LS-050：自定义 VLM 提示词（留空=恢复内置默认），即时生效
async function saveVlmPrompt() {
  busy.value = true
  msg.value = ''
  try {
    await api.saveDetectConfig({ vlm_prompt: vlmPrompt.value })
    await load()
    msg.value = vlmPrompt.value.trim() ? '已保存自定义提示词（后续 API 判定生效）' : '已恢复内置默认提示词'
  } catch (e) { msg.value = String(e.message || e) }
  busy.value = false
}

// LS-050：Motion Gate 性能开关（即时生效，无需重启）
async function savePerf() {
  busy.value = true
  msg.value = ''
  try {
    await api.saveDetectConfig({
      perf: { motion_gate_enabled: motionGate.value, motion_threshold: Number(motionThreshold.value), motion_event_recording: motionRecording.value },
    })
    await load()
    msg.value = '性能设置已保存'
  } catch (e) { msg.value = String(e.message || e) }
  busy.value = false
}

const localRuntime = ref(null)
const bundledPeople = ref([])
const personSelection = ref('')
const personMessage = ref('')
const personBusy = ref(false)
const localModel = ref('')
const localProjector = ref('')
const localLayers = ref(0)
const localDevice = ref('auto')
const localBudget = ref(0)
const localReserve = ref(1024)
const localDevices = ref([])
const localDeviceMessage = ref('')
const localDeviceBusy = ref(false)
const localBusy = ref(false)
const localMessage = ref('')
const localStates = { stopped: '已停止', starting: '加载中', cpu: 'CPU 已就绪', cpu_fallback: '已回退至 CPU', gpu_requested: '已加载，已请求 GPU offload' }

async function refreshLocal(loadSettings = false) {
  try {
    localRuntime.value = await api.localVlm()
    if (loadSettings && localRuntime.value?.gpu_settings) {
      const settings = localRuntime.value.gpu_settings
      localDevice.value = typeof settings.vulkan_device === 'string' ? settings.vulkan_device : 'auto'
      localBudget.value = Number.isInteger(settings.vram_budget_mb) ? settings.vram_budget_mb : 0
      localReserve.value = Number.isInteger(settings.cv_vram_reserve_mb) ? settings.cv_vram_reserve_mb : 1024
    }
  } catch { localRuntime.value = null }
}

async function listLocalDevices() {
  localDeviceBusy.value = true
  localDeviceMessage.value = ''
  try {
    const result = await api.localVlm(true)
    localDevices.value = Array.isArray(result?.vulkan_devices) ? result.vulkan_devices.filter(device =>
      typeof device?.id === 'string' && typeof device?.name === 'string' && Number.isFinite(device?.free_mb)
    ) : []
    localDeviceMessage.value = result?.device_error || (localDevices.value.length ? '' : '本机视觉运行时未列出可用的 Vulkan 显卡。')
  } catch (error) { localDeviceMessage.value = error.message }
  finally { localDeviceBusy.value = false }
}

async function refreshPersonModels() {
  const result = await api.modelsLocal().catch(() => null)
  bundledPeople.value = Array.isArray(result?.models) ? result.models.filter(model => model?.kind === 'bundled_person' && typeof model.path === 'string') : []
  const current = typeof result?.active_person_model === 'string' ? result.active_person_model : ''
  personSelection.value = bundledPeople.value.some(model => model.path === current) ? current : (current ? 'custom' : '')
}

async function selectPersonModel(path) {
  personBusy.value = true
  try {
    await api.setModelActive(path)
    personMessage.value = '人员检测模型已切换。'
  } catch (error) { personMessage.value = error.message }
  finally { await refreshPersonModels(); personBusy.value = false }
}

async function changeLocal(start) {
  localBusy.value = true
  localMessage.value = ''
  try {
    if (start) {
      await api.startLocalVlm({
        model: localModel.value.trim(), projector: localProjector.value.trim(),
        gpu_layers: Number(localLayers.value), vulkan_device: localDevice.value,
        vram_budget_mb: Number(localBudget.value), cv_vram_reserve_mb: Number(localReserve.value),
      })
      localMessage.value = '本机档案已保存，可在 API 服务中启用。'
      await load()
    } else {
      await api.stopLocalVlm()
      localMessage.value = '离线视觉服务已停止。'
    }
  } catch (error) { localMessage.value = error.message }
  finally { await refreshLocal(); localBusy.value = false }
}

onMounted(() => { load(); refreshLocal(true); refreshPersonModels() })
</script>

<template>
  <div class="detect-panel">
    <p v-if="loading" class="muted">加载中…</p>
    <template v-else>
      <!-- 检测模式：三卡片单选 -->
      <div class="mode-row" role="radiogroup" aria-label="检测模式">
        <button v-for="m in MODES" :key="m.id" role="radio" :aria-checked="mode === m.id"
                class="mode-card" :class="{ active: mode === m.id, rec: m.id === 'cascade' }"
                @click="pickMode(m.id)">
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"
               stroke-linecap="round" stroke-linejoin="round" v-html="m.icon"></svg>
          <b>{{ m.title }}</b>
          <span class="muted">{{ m.desc }}</span>
        </button>
      </div>
      <p v-if="usage" class="muted usage-line"
         title="「API 复核」= 本地+API 协作模式下，可疑画面发给外部联网模型精判的次数（仅本地模式恒为 0）。每日上限可自定义：改完即存，耗尽自动降级仅本地，次日恢复。">
        今日 API 复核：<b>{{ usage.count }}</b> 次 / 每日上限
        <input v-model.number="dailyBudget" type="number" min="0" step="10" aria-label="每日 API 复核上限"
               style="width:76px; margin:0 4px" @change="saveCascade" />
        次（0 = 不限，改完即存）
        <span v-if="mode === 'local'" class="muted">（仅本地模式不调 API，计数恒为 0）</span>
        <span v-if="usage.exhausted" class="lv3"> · 今日上限已用尽——已自动降级为仅本地，次日自动恢复</span>
      </p>
      <p v-if="msg" class="muted">{{ msg }}</p>

      <!-- 本地模型（吸收原「模型管理」页）+ 推理设备切换 -->
      <div class="card">
        <h3>本地模型</h3>
        <div class="row" style="align-items:center; margin-bottom:10px">
          <span class="muted" style="flex-shrink:0">推理设备：</span>
          <label style="cursor:pointer"><input type="radio" value="dml" v-model="runtimeEp" :disabled="busy"
            @change="saveRuntime({ backend: 'dml' })" /> DirectML（默认）</label>
          <label v-if="vulkanAvailable || vulkanEnabled" style="cursor:pointer"><input type="radio" value="auto_vulkan"
            :checked="vulkanEnabled" :disabled="busy || !vulkanAvailable" @change="saveRuntime({ backend: 'auto_vulkan' })" /> Vulkan（灰度）</label>
          <label style="cursor:pointer; margin-left:16px"><input type="radio" value="cpu" v-model="runtimeEp" :disabled="busy"
            @change="saveRuntime({ backend: 'cpu' })" /> 仅 CPU</label>
          <span v-if="runtimeEp === 'dml'" class="muted" style="font-size:12px">
            GPU 初始化失败会自动回落 CPU（看服务日志）</span>
        </div>
        <p v-if="vulkanEnabled" class="muted">随包人员和 PPE 模型优先使用 Vulkan，失败依次回退到 DirectML 和 CPU。其他模型沿用 ONNX 推理。</p>
        <p v-if="vulkanEnabled && !vulkanAvailable" class="muted">当前安装包没有 Vulkan 灰度运行时，分析源使用 ONNX。</p>
        <div v-if="vulkanEnabled && vulkanAvailable" class="row">
          <label>Vulkan 设备
            <select v-model="runtimeGpuId" :disabled="busy" @change="saveRuntime({ gpu_id: runtimeGpuId })">
              <option value="auto">自动选择</option>
              <option v-for="adapter in physicalAdapters" :key="adapter.gpu_id" :value="adapter.gpu_id">{{ adapter.name }}</option>
              <option v-if="runtimeGpuId !== 'auto' && !physicalAdapter" :value="runtimeGpuId" disabled>
                {{ runtimeGpuId.startsWith('vulkan:') ? `ncnn 设备 ${runtimeGpuId}` : '已选显卡当前不可用' }}
              </option>
            </select>
          </label>
          <span v-if="sameDirectMlAdapter" class="muted">DirectML 回退使用同一张显卡</span>
          <details>
            <summary>按 ncnn 编号选择</summary>
            <label>ncnn 设备 <input v-model.trim="manualVulkanId" placeholder="vulkan:0" :disabled="busy" /></label>
            <button type="button" :disabled="busy || !manualVulkanId" @click="saveRuntime({ gpu_id: manualVulkanId })">保存设备编号</button>
          </details>
        </div>
        <div v-if="runtimeEp !== 'cpu' && !sameDirectMlAdapter && (directMlAdapters.length || runtimeAdapter !== null)" class="row">
          <label>DirectML 显卡
            <select v-model="runtimeAdapter" :disabled="busy" @change="saveRuntime({ adapter_index: runtimeAdapter })">
              <option :value="null">自动选择</option>
              <option v-for="adapter in directMlAdapters" :key="adapter.dxgi_index" :value="adapter.dxgi_index">{{ adapter.name }}</option>
              <option v-if="!selectedAdapter && runtimeAdapter !== null" :value="runtimeAdapter" disabled>已配置显卡当前不可用</option>
            </select>
          </label>
          <span v-if="selectedAdapter?.budget_bytes" class="muted">
            显存预算 {{ (selectedAdapter.budget_bytes / (1024 ** 3)).toFixed(1) }} GB
          </span>
          <span v-if="selectedAdapter?.vulkan_index != null" class="muted">支持 Vulkan</span>
        </div>
        <div class="row">
          <span class="muted">分析源的实际推理设备</span>
          <button type="button" class="ghost" @click="refreshRuntimeDevices">刷新设备状态</button>
        </div>
        <ul v-if="cameraDevices.length">
          <li v-for="device in cameraDevices" :key="device.camera_id">
            #{{ device.camera_id }} · 人员 {{ device.person || '未加载' }} · PPE {{ device.ppe || '未加载' }}
          </li>
        </ul>
        <p v-else class="muted">启动分析源后可查看实际设备。</p>
        <div class="row" role="group" aria-label="检测灵敏度">
          <span>检测灵敏度</span>
          <button v-for="preset in sensitivityPresets" :key="preset.id" type="button" class="ghost"
                  :aria-pressed="cfg?.sensitivity === preset.id" :disabled="busy || cfg?.sensitivity === preset.id"
                  @click="save({ sensitivity: preset.id }, '已应用' + preset.label + '灵敏度')">{{ preset.label }}</button>
        </div>
        <Models />
      </div>

      <!-- API 服务（多接入档案） -->
      <div v-if="bundledPeople.length" class="panel">
        <label>人员检测模型
          <select :value="personSelection" :disabled="personBusy" @change="selectPersonModel($event.target.value)">
            <option value="">跟随默认配置</option>
            <option v-if="personSelection === 'custom'" value="custom" disabled>自定义模型</option>
            <option v-for="model in bundledPeople" :key="model.path" :value="model.path">{{ model.name }}{{ model.trial ? '（试用）' : '' }}</option>
          </select>
        </label>
        <p v-if="personMessage" role="status">{{ personMessage }}</p>
      </div>
      <div v-if="localRuntime" class="panel">
        <h3>离线视觉服务</h3>
        <p class="muted">{{ localStates[localRuntime.status] || '等待状态' }}。将官方 llama.cpp 运行时放入软件的 tools/llama 目录，将 GGUF 权重和视觉投影放在数据目录 models/local-vlm 下。</p>
        <div class="row">
          <label>模型文件 <input v-model="localModel" placeholder="视觉模型.gguf" /></label>
          <label>视觉投影 <input v-model="localProjector" placeholder="mmproj-F16.gguf" /></label>
          <label>GPU 层数 <input v-model.number="localLayers" type="number" min="0" max="128" /></label>
          <button type="button" :disabled="localBusy || !localRuntime.runtime_available || !localModel.trim() || !localProjector.trim()" @click="changeLocal(true)">启动</button>
          <button type="button" class="ghost" :disabled="localBusy || localRuntime.status === 'stopped'" @click="changeLocal(false)">停止</button>
        </div>
        <div v-if="localLayers > 0" class="row">
          <label>视觉模型显卡
            <select v-model="localDevice" :disabled="localBusy">
              <option value="auto">自动选择</option>
              <option v-if="localDevice !== 'auto' && !localDevices.some(device => device.id === localDevice)" :value="localDevice">{{ localDevice }}</option>
              <option v-for="device in localDevices" :key="device.id" :value="device.id">{{ device.id }} · {{ device.name }}（可用 {{ device.free_mb }} MiB）</option>
            </select>
          </label>
          <button type="button" class="ghost" :disabled="localDeviceBusy || localBusy || !localRuntime.runtime_available" @click="listLocalDevices">{{ localDeviceBusy ? '枚举中…' : '列出显卡' }}</button>
          <label>视觉显存预算（MiB，0 自动） <input v-model.number="localBudget" type="number" min="0" max="1048576" :disabled="localBusy" /></label>
          <label>检测预留显存（MiB） <input v-model.number="localReserve" type="number" min="0" max="1048576" :disabled="localBusy" /></label>
        </div>
        <p v-if="localDeviceMessage" role="status">{{ localDeviceMessage }}</p>
        <p v-if="localRuntime.gpu" class="muted">
          {{ localRuntime.gpu.device.name }} · {{ localRuntime.gpu.device.id }} · 视觉预算 {{ localRuntime.gpu.usable_mb }} MiB ·
          检测预留 {{ localRuntime.gpu.cv_reserve_mb }} MiB · 计划 {{ localRuntime.gpu.layers }} 层
        </p>
        <p v-if="localRuntime.fallback_reason" role="status">{{ localRuntime.fallback_reason }}</p>
        <p v-if="!localRuntime.runtime_available" class="muted">等待安装 llama.cpp 运行时。</p>
        <p v-if="localBusy || localMessage" role="status">{{ localBusy ? '正在处理离线服务…' : localMessage }}</p>
      </div>
      <div class="card" ref="apiCardEl">
        <h3>API 服务
          <span v-if="mode === 'local'" class="muted" style="font-size:12px; font-weight:400">
            （当前仅本地模式；保存并启用后，点上方「仅 API」或「本地 + API 协作」卡片即生效）</span>
        </h3>
        <p class="muted" style="margin-top:0; font-size:12.5px">
          可保存多个 API 服务，随时切换；填写地址和密钥后点「获取模型」自动列出上游模型。
        </p>
        <div class="row" style="align-items:center; flex-wrap:wrap; margin-bottom:8px">
          <select :value="editId" aria-label="已保存的 API 服务" style="max-width:240px"
                  @change="selectProfile($event.target.value)">
            <option value="">＋ 新增服务…</option>
            <option v-for="p in profiles" :key="p.id" :value="p.id">
              {{ p.name }}{{ p.id === activeId ? '（使用中）' : '' }}
            </option>
          </select>
          <button class="ghost" @click="newProfile">新增</button>
          <button v-if="editId" class="ghost" :disabled="profileBusy || editId === activeId"
                  @click="useProfile">设为当前</button>
          <button v-if="editId" class="ghost" :disabled="profileBusy" @click="removeProfile">删除</button>
          <span class="muted" style="font-size:12px">
            当前启用：<b>{{ activeProfile?.name || '未配置' }}</b></span>
        </div>
        <div class="row" style="align-items:center; flex-wrap:wrap">
          <input v-model="pName" placeholder="名称，如 公司 GPT / 本机 Ollama" aria-label="服务名称"
                 style="width:190px" />
          <select v-model="provider" aria-label="服务协议" style="max-width:170px">
            <option value="openai_compat">OpenAI 兼容（通用）</option>
            <option value="ollama">Ollama（本机）</option>
            <option value="gemini">Gemini（原生视频）</option>
          </select>
          <input v-model="url" ref="urlInputEl"
                 placeholder="API 地址，如 https://api.example.com（OpenAI 兼容填根地址）"
                 aria-label="API 地址" style="flex:1; min-width:240px" />
        </div>
        <div class="row" style="align-items:center; margin-top:8px; flex-wrap:wrap">
          <input v-model="model" list="upstream-models" placeholder="模型名（可点右侧自动获取）"
                 aria-label="模型名" style="width:240px" />
          <datalist id="upstream-models">
            <option v-for="m in modelOptions" :key="m" :value="m" />
          </datalist>
          <button class="ghost" :disabled="fetchBusy" @click="fetchModels">
            {{ fetchBusy ? '获取中…' : '获取模型' }}</button>
          <input v-model="apiKey" type="password" :placeholder="apiKeySet ? '已保存密钥（留空不变更）' : 'API Key'"
                 aria-label="API Key" style="width:220px" autocomplete="new-password" />
          <input v-model.number="timeoutS" type="number" min="1" max="120" aria-label="超时秒数"
                 style="width:80px" title="超时秒数" />
          <span class="muted">秒</span>
          <button v-if="apiKeySet && editId" class="ghost" @click="clearKey">清除密钥</button>
        </div>
        <label v-if="isRemoteUrl" class="remote-confirm" style="display:block; margin-top:8px">
          <input type="checkbox" v-model="allowRemote" />
          我确认允许将监控截图发送到该外部服务（合规确认；回环地址无需确认）
        </label>
        <div class="row" style="margin-top:10px; align-items:center; flex-wrap:wrap">
          <button :disabled="profileBusy" @click="saveProfile(true)">保存并启用</button>
          <button class="ghost" :disabled="profileBusy" @click="saveProfile(false)">仅保存</button>
          <button class="ghost" :disabled="fetchBusy" @click="fetchModels">
            {{ fetchBusy ? '测试中…' : '测试连通性' }}</button>
          <button class="ghost" :disabled="testBusy" title="用一张测试图走完整判定流程（需先保存并启用，且模式为仅 API/协作）"
                  @click="runTest">{{ testBusy ? '判定中…' : '测试判定' }}</button>
          <span v-if="testResult && testResult.ok" class="st-ok">
            连接成功（{{ testResult.elapsed_ms }}ms）
            <template v-if="testResult.count != null">，可用模型 {{ testResult.count }} 个</template>
            <template v-if="testResult.verdict"> —— 判定：{{ JSON.stringify(testResult.verdict) }}</template>
          </span>
          <span v-else-if="testResult" class="lv3">失败：{{ testResult.detail }}</span>
        </div>
      </div>

      <!-- LS-050：性能开关（Motion Gate） -->
      <div class="card">
        <h3>性能选项</h3>
        <label class="muted" style="display:block;margin:6px 0">
          <input type="checkbox" v-model="motionGate" /> 启用运动预筛（Motion Gate）
        </label>
        <label class="muted" style="display:block;margin:6px 0">
          <input type="checkbox" v-model="motionRecording" /> 实时机位有明显运动时保存短片
        </label>
        <div class="row" style="align-items:center;flex-wrap:wrap">
          <label class="muted">
            帧差阈值 <b>{{ motionThreshold }}</b>
            <input type="range" min="0" max="20" step="0.5" v-model.number="motionThreshold"
                   style="vertical-align:middle" />（越大越容易跳过，建议 1~3）
          </label>
          <button class="ghost" :disabled="busy" @click="savePerf">保存性能设置</button>
        </div>
        <p class="muted" style="font-size:12px;margin-bottom:0">
          无人在场且画面静止时跳过 YOLO/实验服推理（省 70%+ 算力）；有活跃目标或每 10 帧心跳强制全检，绝不漏判。跳过比例见仪表盘「后端」卡。</p>
      </div>

      <!-- LS-050：自定义 VLM 提示词 -->
      <div class="card">
        <h3>外部模型提示词（VLM）</h3>
        <p class="muted" style="margin-top:0;font-size:12.5px">
          自定义发给上游视觉模型的分析提示词；留空使用内置实验室安全模板。需输出 JSON：persons / no_ppe / violations / summary。</p>
        <textarea v-model="vlmPrompt" rows="4" aria-label="VLM 提示词"
                  placeholder="留空 = 使用内置默认提示词"
                  style="width:100%; box-sizing:border-box; font-size:13px"></textarea>
        <div class="row" style="margin-top:8px">
          <button class="ghost" :disabled="busy" @click="saveVlmPrompt">保存提示词</button>
          <button class="ghost" :disabled="busy" @click="vlmPrompt = ''; saveVlmPrompt()">恢复默认</button>
        </div>
      </div>

      <!-- 协作策略（cascade 模式展示） -->
      <div v-show="mode === 'cascade'" class="card">
        <h3>协作策略</h3>
        <p class="muted" style="margin-top:0">
          每个人在复核窗口内最多提交一张图。再次出现或出现另一种违规时，仍照常记录本地事件。</p>
        <div class="row" style="align-items:center; flex-wrap:wrap">
          <label class="muted">触发条件
            <select v-model="trigger" aria-label="触发条件">
              <option value="both">人员或规则命中</option>
              <option value="persons">仅人员出现</option>
              <option value="rules">仅规则命中</option>
            </select>
          </label>
          <label class="muted">每日 API 上限
            <input v-model.number="dailyBudget" type="number" min="0" step="10" aria-label="每日预算" style="width:90px" />
          </label>
          <span class="muted">0 = 不限</span>
          <label class="muted">同人复核间隔（秒）
            <input v-model.number="personWindow" type="number" min="0" max="3600" step="1" aria-label="同人复核间隔" style="width:90px" />
          </label>
          <label class="muted">VLM 否定时
            <select v-model="onNegative" aria-label="否定策略">
              <option value="keep">照常推送（保守）</option>
              <option value="suppress">抑制推送（更省打扰）</option>
            </select>
          </label>
          <button :disabled="busy" @click="saveCascade">保存策略</button>
        </div>
        <p class="muted" style="font-size:12px; margin-bottom:0">
          跨轨迹按已录入的本机人脸匹配。未匹配的人员按当前轨迹计算。间隔填 0 可关闭按人限频。</p>
      </div>
    </template>
  </div>
</template>

<style scoped>
.mode-row { display: flex; gap: 10px; flex-wrap: wrap; }
.mode-card {
  flex: 1; min-width: 180px; display: flex; flex-direction: column; gap: 5px; align-items: flex-start;
  padding: 12px 14px; cursor: pointer; text-align: left;
  background: var(--card-bg); color: var(--text-primary);
  border: 1px solid var(--stroke-default); border-radius: var(--control-radius);
  transition: border-color var(--fast-duration) var(--fast-out-slow-in),
              background-color var(--fast-duration) var(--fast-out-slow-in);
}
.mode-card:hover { border-color: var(--accent); }
.mode-card.active { border-color: var(--accent); background: var(--ctrl-fill); }
.mode-card.rec b::after { content: "推荐"; margin-left: 6px; font-size: 11px; font-weight: 500;
  color: var(--accent); border: 1px solid var(--accent); border-radius: 4px; padding: 0 5px; }
.mode-card svg { width: 20px; height: 20px; color: var(--accent); }
.usage-line { margin: 8px 2px 0; }
.remote-confirm { color: var(--text-secondary); font-size: 12.5px; }
.detect-panel .card { margin-top: 14px; }
.detect-panel .card h3 { margin-top: 0; }
</style>
