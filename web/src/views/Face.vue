<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { api } from '../api'
import AsyncState from '../components/AsyncState.vue'

const MODE_LABELS = { off: '关闭', local: '本地 YuNet + SFace', api: 'CompreFace API' }
const LOCAL_PROFILE = 'local-sface'
const NEW_PROFILE = { name: '', url: '', api_key: '', threshold: 0.75, timeout_s: 5, enabled: true }
const config = ref(null)
const people = ref([])
const legacyPeople = ref([])
const loading = ref(true)
const busy = ref(false)
const error = ref('')
const notice = ref('')
const profileDraft = ref({ ...NEW_PROFILE })
const editId = ref('')
const subject = ref('')
const consent = ref(false)
const photos = ref([])
const recognition = ref(null)
const preview = ref(null)
const capturing = ref(false)
const profiles = computed(() => Array.isArray(config.value?.apis) ? config.value.apis : [])
const cameras = computed(() => Array.isArray(config.value?.cameras) ? config.value.cameras : [])
const selectedApi = computed(() => profiles.value.find(profile => profile.id === config.value?.active?.api))
const activeMode = computed(() => config.value?.active?.mode || 'off')
const activeReady = computed(() => config.value?.status?.enabled && config.value?.status?.configured)
let stream = null
let captureRequest = 0
let mounted = true
watch(subject, () => { consent.value = false })

function setConfig(value) {
  if (!value || typeof value.active?.mode !== 'string' || !value.local || !Array.isArray(value.apis)) {
    throw new Error('人脸模型配置格式异常')
  }
  config.value = value
}

async function loadPeople() {
  const result = await api.faceLibrary()
  people.value = Array.isArray(result) ? result : []
  legacyPeople.value = []
  if (selectedApi.value?.id.startsWith('legacy-') && activeMode.value === 'api') {
    try {
      const old = await api.facePersons()
      legacyPeople.value = Array.isArray(old) ? old.filter(person => person && !person.id) : []
    } catch { /* 继续展示本机登记与撤回状态 */ }
  }
}

async function load() {
  error.value = ''
  try {
    setConfig(await api.faceConfig())
    adjustProfile(selectedApi.value?.id || '')
    await loadPeople()
  } catch (err) { error.value = String(err.message || err) }
  finally { loading.value = false }
}

async function run(action) {
  if (busy.value) return
  busy.value = true
  error.value = ''
  notice.value = ''
  try { await action() }
  catch (err) { error.value = String(err.message || err) }
  finally { busy.value = false }
}

function apply(patch, message) {
  return run(async () => {
    const before = config.value?.active
    setConfig(await api.saveFaceConfig(patch))
    recognition.value = null
    if (before?.mode !== activeMode.value || before?.api !== config.value.active.api) {
      stopCamera()
      consent.value = false
    }
    notice.value = message
    await loadPeople()
  })
}

function adjustProfile(id) {
  editId.value = id
  const profile = profiles.value.find(item => item.id === id)
  profileDraft.value = profile ? { ...profile, api_key: '' } : { ...NEW_PROFILE }
}

function saveProfile() {
  return run(async () => {
    const result = await api.saveFaceConfig({ profile: { ...profileDraft.value, id: editId.value || undefined } })
    setConfig(result)
    adjustProfile(result.saved_api || result.active.api)
    notice.value = 'API 模型已保存'
    await loadPeople()
  })
}

function testProfile() {
  return run(async () => {
    const result = await api.testFaceApi(editId.value)
    if (!result?.ok) throw new Error('API 连接测试失败')
    notice.value = `连接成功 · ${result.elapsed_ms} ms · ${result.subjects} 个远程人员条目`
  })
}

function install() {
  return run(async () => {
    setConfig(await api.installFaceLocal())
    notice.value = '本地模型已下载并通过文件校验'
  })
}

function setCamera(camera, permission, alertOnMatch = camera.alert_on_match) {
  return run(async () => {
    await api.setFaceCamera(camera.id, permission, alertOnMatch)
    setConfig(await api.faceConfig())
    notice.value = `已保存「${camera.name}」的人脸识别范围`
  })
}

function appendPhotos(files) {
  error.value = ''
  for (const file of files) {
    if (photos.value.length === 5) {
      error.value = '一次最多录入 5 张照片'
      break
    }
    if (!file || file.size === 0 || file.size > 5 * 1024 * 1024) {
      error.value = '每张照片大小应小于 5 MB'
      continue
    }
    photos.value.push({ file, url: URL.createObjectURL(file) })
  }
  recognition.value = null
}

function pickFiles(event) {
  appendPhotos(Array.from(event.target.files || []))
  event.target.value = ''
}

function removePhoto(index) {
  const [photo] = photos.value.splice(index, 1)
  if (photo) URL.revokeObjectURL(photo.url)
}

function clearPhotos() {
  for (const photo of photos.value) URL.revokeObjectURL(photo.url)
  photos.value = []
}

async function openCamera() {
  error.value = ''
  if (!navigator.mediaDevices?.getUserMedia) {
    error.value = '当前浏览器无法打开相机，可上传照片。浏览器拍照需要本机地址或 HTTPS。'
    return
  }
  const request = ++captureRequest
  try {
    const opened = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'user' }, audio: false })
    if (!mounted || request !== captureRequest) {
      opened.getTracks().forEach(track => track.stop())
      return
    }
    stream = opened
    capturing.value = true
    await nextTick()
    if (preview.value) preview.value.srcObject = stream
  } catch (err) {
    if (mounted && request === captureRequest) error.value = `相机打开失败：${err.message || err}`
  }
}

function stopCamera() {
  captureRequest++
  stream?.getTracks().forEach(track => track.stop())
  stream = null
  capturing.value = false
}

function capturePhoto() {
  const video = preview.value
  if (!video?.videoWidth) return
  const canvas = document.createElement('canvas')
  const scale = Math.min(1, 1280 / Math.max(video.videoWidth, video.videoHeight))
  canvas.width = Math.max(1, Math.round(video.videoWidth * scale))
  canvas.height = Math.max(1, Math.round(video.videoHeight * scale))
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  ctx.drawImage(video, 0, 0, canvas.width, canvas.height)
  const request = captureRequest
  canvas.toBlob(blob => {
    if (blob && mounted && request === captureRequest) {
      appendPhotos([new File([blob], 'camera.jpg', { type: 'image/jpeg' })])
    }
  }, 'image/jpeg', 0.9)
}

function photoForm(selected) {
  const form = new FormData()
  form.set('consent', String(consent.value))
  for (const photo of selected) form.append('photo', photo.file)
  return form
}

function register() {
  return run(async () => {
    const name = subject.value.trim()
    if (!name || !photos.value.length || !consent.value) throw new Error('请填写姓名、添加照片并确认已取得本人同意')
    const form = photoForm(photos.value)
    form.set('subject', name)
    const result = await api.faceRegister(form)
    if (!result?.ok) throw new Error('人员录入未完成')
    notice.value = `已录入 ${name}，共 ${result.samples_added} 张照片`
    await loadPeople()
  })
}

function recognizePhoto() {
  return run(async () => {
    const last = photos.value.at(-1)
    if (!last || !consent.value) throw new Error('请选择照片并确认试识别授权')
    recognition.value = await api.recognizeFace(photoForm([last]))
  })
}

function removePerson(person, legacy = false) {
  if (!person.removing && !confirm(`撤回「${person.subject}」的录入并删除人脸模板？`)) return
  return run(async () => {
    try {
      if (legacy) await api.faceDeletePerson(person.subject)
      else await api.removeFacePerson(person.id)
      notice.value = '登记已撤回'
    } finally { await loadPeople() }
  })
}

function clearLibrary() {
  if (!confirm('撤回人员库里的全部登记并删除模板？')) return
  return run(async () => {
    const result = await api.faceClearAll()
    await loadPeople()
    if (!result?.ok) throw new Error(`仍有 ${result?.pending ?? 0} 项待删除，可从人员库重试`)
    notice.value = '已撤回全部登记'
  })
}

function profileName(person) {
  if (person.profile === LOCAL_PROFILE) return MODE_LABELS.local
  return profiles.value.find(profile => profile.id === person.profile)?.name || MODE_LABELS.api
}

function expiry(date) {
  return Number.isFinite(date) ? new Date(date * 1000).toLocaleDateString('zh-CN') : '—'
}

onMounted(load)
onBeforeUnmount(() => {
  mounted = false
  stopCamera()
  clearPhotos()
})
</script>

<template>
  <AsyncState :loading="loading" :error="config ? '' : error" @retry="load">
    <section v-if="config" class="face-management">
      <div class="panel">
        <h3>人脸识别</h3>
        <div class="row face-modes" role="group" aria-label="识别模式">
          <button v-for="(label, mode) in MODE_LABELS" :key="mode" type="button"
            :class="{ ghost: activeMode !== mode }" :aria-pressed="activeMode === mode"
            :disabled="busy || (mode === 'local' && !config.local.enabled) || (mode === 'api' && !selectedApi?.enabled)"
            @click="apply({ mode }, `已切换为${label}`)">{{ label }}</button>
          <button v-if="config.previous" type="button" class="ghost" :disabled="busy"
            @click="apply({ restore_previous: true }, '已切回上一个模型')">切回上一个模型</button>
        </div>
        <p v-if="activeMode === 'api'" class="muted">正在使用 {{ selectedApi?.name || MODE_LABELS.api }}</p>
        <p v-else-if="activeMode === 'local' && !config.local.installed" class="muted">下载本地模型后即可录入和试识别。</p>
        <p v-if="error" role="alert" class="st-err">{{ error }}</p>
        <p v-if="notice" role="status" class="muted">{{ notice }}</p>
      </div>

      <details class="panel" open>
        <summary>本地模型</summary>
        <div class="row">
          <label><input type="checkbox" :checked="config.local.enabled" :disabled="busy"
            @change="apply({ local_enabled: $event.target.checked }, '已更新本地模型状态')" /> 启用本地模型</label>
          <label>相似度阈值 <input type="number" step="0.01" min="0" max="1" :value="config.local.threshold" :disabled="busy"
            @change="apply({ local_threshold: Number($event.target.value) }, '本地阈值已保存')" /></label>
          <button v-if="!config.local.installed" type="button" class="ghost" :disabled="busy" @click="install">下载 YuNet + SFace（约 39 MB）</button>
          <span v-else class="muted">模型已就绪<template v-if="activeMode === 'local' && Array.isArray(config.local.devices)"> · {{ config.local.devices.join(' / ') }}</template></span>
        </div>
      </details>

      <details class="panel">
        <summary>API 模型</summary>
        <div class="row">
          <label v-if="profiles.length">当前 API
            <select :value="config.active.api" :disabled="busy" @change="apply({ api: $event.target.value }, '已选择 API 模型')">
              <option v-for="profile in profiles" :key="profile.id" :value="profile.id">
                {{ profile.name }}{{ profile.enabled ? '' : '（已停用）' }}
              </option>
            </select>
          </label>
          <label>编辑档案
            <select :value="editId" :disabled="busy" @change="adjustProfile($event.target.value)">
              <option value="">新增 CompreFace 档案</option>
              <option v-for="profile in profiles" :key="profile.id" :value="profile.id">{{ profile.name }}</option>
            </select>
          </label>
        </div>
        <form class="model-form" @submit.prevent="saveProfile">
          <label>模型名称<input v-model="profileDraft.name" maxlength="80" required /></label>
          <label>服务地址<input v-model="profileDraft.url" type="url" placeholder="https://face.example.com" required /></label>
          <label>API 密钥<input v-model="profileDraft.api_key" type="password" autocomplete="off"
            :placeholder="editId ? '留空保留密钥' : 'CompreFace API 密钥'" :required="!editId" /></label>
          <label>相似度阈值<input v-model.number="profileDraft.threshold" type="number" min="0" max="1" step="0.01" required /></label>
          <label>连接超时（秒）<input v-model.number="profileDraft.timeout_s" type="number" min="1" max="15" required /></label>
          <label class="inline"><input v-model="profileDraft.enabled" type="checkbox" /> 启用此模型</label>
          <div class="row actions">
            <button :disabled="busy" type="submit">保存 API 模型</button>
            <button v-if="editId" :disabled="busy" type="button" class="ghost" @click="testProfile">测试连接</button>
          </div>
        </form>
        <p class="muted">远程服务会收到录入和试识别的照片。请在 CompreFace 配置照片保留方式。</p>
      </details>

      <details v-if="cameras.length" class="panel">
        <summary>相机人脸识别与告警</summary>
        <p class="muted">开启匹配告警后，识别到已录入人员会生成事件。姓名和匹配分数仅管理员在事件详情中可见。</p>
        <div v-for="camera in cameras" :key="camera.id" class="camera-access">
          <label :for="`face-camera-${camera.id}`">{{ camera.name }}</label>
          <select :id="`face-camera-${camera.id}`" :value="camera.permission" :disabled="busy"
            @change="setCamera(camera, $event.target.value)">
            <option value="off">关闭</option>
            <option value="local">仅限本地识别</option>
            <option value="local_or_api">允许本地识别和向所选 API 发送事件人脸区域</option>
          </select>
          <label><input type="checkbox" :checked="camera.alert_on_match"
            :disabled="busy || camera.permission === 'off'"
            @change="setCamera(camera, camera.permission, $event.target.checked)" /> 匹配已录入人员时告警</label>
        </div>
      </details>

      <section class="panel">
        <div class="section-heading">
          <h3>录入与试识别</h3>
          <label>新录入保留天数
            <input type="number" min="1" max="365" :value="config.retention_days" :disabled="busy"
              @change="apply({ retention_days: Number($event.target.value) }, '样本保留期已保存')" />
          </label>
        </div>
        <p v-if="!activeReady" class="muted">启用配置完整的模型后可录入。</p>
        <template v-else>
          <p class="muted">每张照片只留一人。正面和略微转头各拍一张，保持光线均匀。</p>
          <div class="row">
            <label>照片 <input type="file" accept="image/jpeg,image/png" multiple @change="pickFiles" /></label>
            <button v-if="!capturing" type="button" class="ghost" @click="openCamera">打开相机</button>
            <button v-else type="button" class="ghost" @click="stopCamera">关闭相机</button>
          </div>
          <div v-if="capturing" class="capture-preview">
            <video ref="preview" autoplay playsinline muted />
            <button type="button" class="ghost" @click="capturePhoto">拍一张</button>
          </div>
          <ul v-if="photos.length" class="face-photos">
            <li v-for="(photo, index) in photos" :key="photo.url">
              <img :src="photo.url" :alt="`照片 ${index + 1}`" />
              <button type="button" class="ghost" :aria-label="`移除照片 ${index + 1}`" @click="removePhoto(index)">移除</button>
            </li>
          </ul>
          <form class="enroll-form" @submit.prevent="register">
            <label>姓名 <input v-model.trim="subject" maxlength="80" /></label>
            <label class="inline"><input v-model="consent" type="checkbox" /> 已取得本人的录入或试识别同意</label>
            <div class="row">
              <button type="submit" :disabled="busy || !photos.length || !consent || !subject">录入人员</button>
              <button type="button" class="ghost" :disabled="busy || !photos.length || !consent" @click="recognizePhoto">试识别最后一张</button>
            </div>
          </form>
          <p v-if="recognition" role="status">
            {{ recognition.matched?.subject ? `匹配：${recognition.matched.subject} · 相似度 ${Number(recognition.matched.similarity).toFixed(2)}` : '未匹配到已同意的人员' }}
            · {{ recognition.elapsed_ms }} ms
          </p>
        </template>
      </section>

      <section class="panel">
        <div class="section-heading">
          <h3>人员库</h3>
          <button v-if="people.length" type="button" class="ghost" :disabled="busy" @click="clearLibrary">撤回全部登记</button>
        </div>
        <p v-if="config.pending_remote_cleanup" class="muted">{{ config.pending_remote_cleanup }} 项旧远程样本待清理，服务会重试。</p>
        <p v-if="!people.length && !legacyPeople.length" class="muted">暂无人员登记。</p>
        <div v-if="people.length || legacyPeople.length" class="people-table">
          <table>
            <thead><tr><th>人员</th><th>模型</th><th>样本</th><th>登记日期</th><th>到期日期</th><th>操作</th></tr></thead>
            <tbody>
              <tr v-for="person in people" :key="person.id">
                <td>{{ person.subject }}</td><td>{{ profileName(person) }}</td><td>{{ person.samples }}</td>
                <td>{{ person.consent_ts?.slice(0, 10) }}</td><td>{{ expiry(person.expires_at) }}</td>
                <td><button type="button" class="ghost" :disabled="busy" @click="removePerson(person)">
                  {{ person.removing ? '重试远程删除' : '撤回并删除' }}
                </button></td>
              </tr>
              <tr v-for="person in legacyPeople" :key="person.subject">
                <td>{{ person.subject }}</td><td>旧版 CompreFace</td><td>—</td>
                <td>{{ person.consent_ts?.slice(0, 10) || '—' }}</td><td>需重新录入</td>
                <td><button type="button" class="ghost" :disabled="busy" @click="removePerson(person, true)">删除</button></td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </section>
  </AsyncState>
</template>

<style scoped>
.face-management { display: grid; gap: 12px; }
.face-management .panel { margin: 0; min-width: 0; }
.face-management .row, .section-heading { gap: 8px; align-items: center; flex-wrap: wrap; }
.face-management h3, .face-management p { margin: 6px 0; }
.face-management summary { cursor: pointer; font-weight: 650; margin-bottom: 8px; }
.face-modes button { padding: 5px 12px; }
.model-form { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; margin-top: 12px; }
.model-form label { display: grid; gap: 4px; }
.model-form .actions { grid-column: 1 / -1; }
.face-management .inline { display: flex; align-items: center; gap: 6px; }
.face-management input[type="number"] { width: 88px; }
.section-heading, .camera-access { display: flex; justify-content: space-between; }
.camera-access { gap: 10px; flex-wrap: wrap; align-items: center; padding: 6px 0; }
.camera-access select, input[type="file"] { max-width: 100%; }
.capture-preview, .enroll-form { display: grid; gap: 8px; justify-items: start; margin: 10px 0; }
.capture-preview video { max-width: min(100%, 320px); border-radius: 8px; }
.face-photos { display: flex; flex-wrap: wrap; gap: 8px; padding: 0; list-style: none; }
.face-photos li { display: grid; width: 92px; gap: 4px; }
.face-photos img { width: 92px; height: 92px; object-fit: cover; border-radius: 6px; }
.people-table { overflow-x: auto; }
@media (max-width: 560px) {
  .model-form { grid-template-columns: minmax(0, 1fr); }
}
</style>
