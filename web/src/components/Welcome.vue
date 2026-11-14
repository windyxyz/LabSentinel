<script setup>
// RA5-UX：首次打开引导向导——接入方式/模型选择/通知方式三步完成必要配置。
// 触发：localStorage 无完成标记且 /api/setup/status 存在未完成步骤。
// 完成后写入标记不再弹出；「跳过」同样写入（尊重用户选择）。
import { computed, onMounted, ref } from 'vue'
import { api, setupDone } from '../api'
import FsPicker from './FsPicker.vue'

const emit = defineEmits(['done'])
const step = ref(0)
const busy = ref(false)
const msg = ref('')

// 步骤 2：接入方式（相机表单）
const form = ref({ name: '', source: '', type: 'folder', group_name: '' })
const showFs = ref(false)
const fsTitle = ref('选择截图文件夹')
function pickSource() {
  fsTitle.value = '选择截图文件夹'
  showFs.value = true
}
function onPicked(p) {
  form.value.source = p
}

// 步骤 3：模型选择（本地清单）
const models = ref([])
const pickedModel = ref('')

const status = ref(null)

async function loadStatus() {
  try {
    status.value = (await api.setupStatus()).steps
  } catch {
    status.value = null
  }
}

const needsSetup = computed(() => {
  if (!status.value) return false
  const s = status.value
  return !(s.camera_added && s.model_selected)
})

onMounted(async () => {
  if (setupDone.isDone()) {
    return
  }
  await loadStatus()
  if (status.value && needsSetup.value) {
    step.value = 1
    try {
      models.value = (await api.modelsLocal?.())?.models || []
    } catch {}
  }
})

function skipAll() {
  setupDone.mark()
  emit('done')
}

function next() {
  step.value += 1
}

async function addCamera() {
  if (!form.value.name || !form.value.source) {
    msg.value = '名称与路径必填'
    return
  }
  busy.value = true
  try {
    await api.addCamera({ ...form.value })
    msg.value = ''
    next()
  } catch (e) {
    msg.value = String(e.message || e)
  } finally {
    busy.value = false
  }
}

async function pickModel(p) {
  busy.value = true
  try {
    await api.setModelActive(p.path)
    pickedModel.value = p.path
    busy.value = false
    next()
  } catch (e) {
    busy.value = false
    msg.value = String(e.message || e)
  }
}

function finish() {
  setupDone.mark()
  emit('done')
}
</script>

<template>
  <div v-if="step > 0" style="position:fixed; inset:0; background:rgba(0,0,0,.5); z-index:1200; display:flex; align-items:center; justify-content:center">
    <div style="background:var(--panel, #16202e); color:inherit; border-radius:10px; padding:20px; width:min(640px, 94vw); max-height:88vh; overflow:auto">
      <div v-if="step === 1">
        <h2 style="margin:0 0 6px">欢迎使用 LabSentinel</h2>
        <p class="muted">三步完成必要配置：接入摄像头 → 选择检测模型 → 设置通知方式。全程可跳过，后续在对应页面随时配置。</p>
        <div class="wsteps">
          <div class="wstep" @click="next"><b>1 · 接入摄像头</b><p>添加 RTSP 网络摄像头或接入截图文件夹。</p></div>
          <div class="wstep" @click="next"><b>2 · 选择检测模型</b><p>本地训练模型或外部联网模型均可，随时可换。</p></div>
          <div class="wstep" @click="next"><b>3 · 设置通知方式</b><p>企业微信/钉钉/飞书等渠道，事件发生后即时推送。</p></div>
        </div>
        <div style="display:flex; gap:8px; justify-content:flex-end; margin-top:12px">
          <button class="ghost" @click="skipAll">跳过</button>
          <button class="primary" @click="next">开始</button>
        </div>
      </div>

      <div v-else-if="step === 2">
        <h2 style="margin:0 0 6px">① 接入摄像头</h2>
        <div class="row" style="margin:8px 0">
          <input v-model="form.name" placeholder="名称，如 车间东侧" style="width:140px" />
          <select v-model="form.type">
            <option value="rtsp">RTSP 网络摄像头</option>
            <option value="folder">图片文件夹</option>
            <option value="file">视频文件</option>
          </select>
        </div>
        <div class="row" style="margin:8px 0">
          <input v-model="form.source" placeholder="接入地址（RTSP 地址 / 图片文件夹路径）" style="flex:1" />
          <button v-if="form.type === 'folder'" class="ghost" @click="showFs = true">浏览…</button>
        </div>
        <FsPicker v-if="showFs" title="选择图片文件夹" @pick="(p) => (form.source = p)" @close="showFs = false" />
        <p v-if="msg" class="muted">{{ msg }}</p>
        <div style="display:flex; gap:8px; justify-content:flex-end; margin-top:12px">
          <button class="ghost" @click="step = 1">上一步</button>
          <button class="primary" :disabled="busy" @click="addCamera">添加并下一步</button>
        </div>
      </div>

      <div v-else-if="step === 3">
        <h2 style="margin:0 0 6px">② 选择检测模型</h2>
        <p class="muted">本地模型来自 models/ 目录；也可跳过（后续在「模型管理」页随时选择），或改用外部联网模型（见「模型管理」页说明）。</p>
        <div v-for="m in models" :key="m.path" class="wstep" @click="pickModel(m.path)">
          <b>{{ m.name }}</b>
          <p class="muted">{{ Math.round(m.size_bytes / 1048576) }} MB · {{ m.path }}</p>
        </div>
        <p v-if="!models.length" class="muted">未发现本地模型文件——可跳过，后续将模型放入数据目录 models/ 下再选。</p>
        <div style="display:flex; gap:8px; justify-content:flex-end; margin-top:12px">
          <button class="ghost" @click="step = 4">跳过</button>
          <button class="ghost" @click="step = 1">上一步</button>
        </div>
      </div>

      <div v-else-if="step === 4">
        <h2 style="margin:0 0 6px">③ 设置通知方式</h2>
        <p class="muted">
          事件发生后可通过企业微信/钉钉/飞书/Server 酱/PushPlus/邮件等渠道即时推送。
          请前往「告警配置」页填写渠道参数；不配置也可正常使用（事件仍会入库与展示）。
        </p>
        <div style="display:flex; gap:8px; justify-content:flex-end; margin-top:12px">
          <button class="primary" @click="finish">完成</button>
        </div>
      </div>

      <div v-else-if="step === 5">
        <h2 style="margin:0 0 6px">配置完成 🎉</h2>
        <p class="muted">已进入仪表盘。摄像头可在「摄像头」页启停，事件在「事件查询」页检索，随时可回这里调整配置。</p>
        <div style="display:flex; justify-content:flex-end; margin-top:12px">
          <button class="primary" @click="finish">开始使用</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.wsteps { display:flex; flex-direction:column; gap:8px; margin-top:10px; }
.wstep { border:1px solid var(--divider, #ddd); border-radius:8px; padding:10px 12px; cursor:pointer; }
.wstep:hover { background:var(--divider, #f0f0f0); }
.wstep p { margin:4px 0 0; color:var(--muted, #888); font-size:13px; }
.primary { background:var(--accent, #1b3366); color:#fff; border:none; border-radius:6px; padding:6px 14px; cursor:pointer; }
.ghost { background:none; border:1px solid var(--divider, #ccc); border-radius:6px; padding:5px 12px; cursor:pointer; }
.muted { color:var(--muted, #888); }
.row { display:flex; gap:8px; align-items:center; }
.row input, .row select { padding:5px 8px; }
</style>
