<script setup>
// 人脸管理：开关状态只读展示（开关只能改 .env 重启，不提供界面开关——防误开）。
// 合规：注册必须勾选"已取得本人同意"；一键清除会连同意记录一起删。
import { onMounted, ref } from 'vue'
import { api } from '../api'

const status = ref({ enabled: false, configured: false, threshold: 0.75 })
const persons = ref([])
const form = ref({ subject: '', consent: false })
const photoFile = ref(null)
const msg = ref('')
const busy = ref(false)

async function load() {
  try {
    status.value = await api.faceStatus()
    if (status.value.enabled && status.value.configured) persons.value = await api.facePersons()
  } catch (e) { msg.value = String(e.message || e) }
}
onMounted(load)

function pickFile(e) { photoFile.value = e.target.files[0] || null }

async function register() {
  msg.value = ''
  if (!form.value.subject || !photoFile.value) { msg.value = '请填人员名并选择照片'; return }
  if (!form.value.consent) { msg.value = '必须先取得本人同意并勾选（合规要求）'; return }
  busy.value = true
  try {
    const fd = new FormData()
    fd.append('subject', form.value.subject)
    fd.append('consent', 'true')
    fd.append('photo', photoFile.value)
    await api.faceRegister(fd)
    msg.value = `已注册：${form.value.subject}`
    form.value = { subject: '', consent: false }
    photoFile.value = null
    persons.value = await api.facePersons()
  } catch (e) { msg.value = String(e.message || e) }
  busy.value = false
}

async function delPerson(subject) {
  msg.value = ''
  try { await api.faceDeletePerson(subject); persons.value = await api.facePersons() }
  catch (e) { msg.value = String(e.message || e) }
}

async function clearAll() {
  if (!confirm('确认清除全部人脸数据与同意记录？此操作不可恢复。')) return
  msg.value = ''
  try { await api.faceClearAll(); persons.value = await api.facePersons() }
  catch (e) { msg.value = String(e.message || e) }
}
</script>

<template>
  <div class="panel">
    <div class="row">
      <b>人脸比对模块</b>
      <span :class="status.enabled ? 'lv2' : 'lv1'" style="border-radius:10px; padding:1px 8px; font-size:12px">
        {{ status.enabled ? '已开启' : '未开启（默认）' }}
      </span>
      <span class="muted">服务状态：{{ status.configured ? '已配置' : '未配置（缺 LS_COMPREFACE_API_KEY）' }}</span>
      <span class="muted">识别阈值：{{ status.threshold }}</span>
    </div>
    <p class="muted" style="margin-top:6px">
      开关只能通过 .env 的 LS_FACE_ENABLED 修改后重启，界面不提供开关（防误开）。
      开启前必读 README「人脸比对合规」章节：需单独取得本人同意、现场设置提示标识、超 10 万人需备案。
    </p>
  </div>

  <template v-if="status.enabled && status.configured">
    <div class="panel">
      <h3 style="margin-bottom:10px">注册人员</h3>
      <div class="row">
        <input v-model="form.subject" placeholder="人员名" style="width:140px" />
        <input type="file" accept="image/*" @change="pickFile" />
        <label class="muted"><input type="checkbox" v-model="form.consent" style="margin:0 4px 0 0" />已取得本人同意</label>
        <button :disabled="busy" @click="register">注册</button>
      </div>
      <p v-if="msg" class="muted">{{ msg }}</p>
    </div>

    <div class="panel">
      <div class="row">
        <b>已注册人员（{{ persons.length }}）</b>
        <button class="ghost" @click="load">刷新</button>
        <button class="ghost" @click="clearAll">一键清除全部（含同意记录）</button>
      </div>
      <table v-if="persons.length">
        <tr><th>人员</th><th>同意登记时间</th><th></th></tr>
        <tr v-for="p in persons" :key="p.subject">
          <td>{{ p.subject }}</td>
          <td class="muted">{{ p.consent_ts || '（无本地记录）' }}</td>
          <td><button class="ghost" @click="delPerson(p.subject)">删除</button></td>
        </tr>
      </table>
      <p v-else class="muted">暂无注册人员</p>
    </div>
  </template>
</template>
