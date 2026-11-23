<script setup>
import { onMounted, ref } from 'vue'
import { accessChanged, adminToken, api } from '../api'
import AsyncState from '../components/AsyncState.vue'

const settings = ref(null)
const loading = ref(true)
const error = ref('')
const message = ref('')
const busy = ref(false)
const local = ref(true)
const lan = ref(false)
const trustProxy = ref(false)
const currentPassword = ref('')
const password = ref('')
const confirmation = ref('')
const token = ref(adminToken.get())
const tokenMessage = ref('')

async function load() {
  loading.value = true
  error.value = ''
  try {
    const next = await api.accessSettings()
    if (typeof next?.password_set !== 'boolean') throw new Error('服务未返回访问设置。')
    settings.value = { ...next, sessions: Array.isArray(next.sessions) ? next.sessions : [] }
    local.value = next.password_set ? next.local === true : true
    lan.value = next.lan === true
    trustProxy.value = next.trust_proxy === true
  } catch (err) { error.value = String(err.message || err) }
  finally { loading.value = false }
}

async function save() {
  if (busy.value) return
  if (password.value !== confirmation.value) {
    message.value = '两次输入的密码不一致。'
    return
  }
  busy.value = true
  message.value = ''
  try {
    await api.saveAccess({
      local: local.value, lan: lan.value, trust_proxy: trustProxy.value,
      current_password: currentPassword.value || undefined, password: password.value || undefined,
    })
    currentPassword.value = password.value = confirmation.value = ''
    accessChanged()
    await load()
    message.value = '访问设置已生效。'
  } catch (err) { message.value = String(err.message || err) }
  finally { busy.value = false }
}

async function revokeOthers() {
  if (busy.value) return
  busy.value = true
  try {
    await api.revokeOtherSessions()
    await load()
    message.value = '已撤销其他浏览器的访问。'
  } catch (err) { message.value = String(err.message || err) }
  finally { busy.value = false }
}

function saveToken() {
  adminToken.set(token.value.trim())
  tokenMessage.value = adminToken.get() ? '已在当前浏览器保存 API 凭据。' : '已清除浏览器中的 API 凭据。'
  load()
}

function date(seconds) {
  return Number.isFinite(seconds) ? new Date(seconds * 1000).toLocaleString('zh-CN') : '—'
}

onMounted(load)
</script>

<template>
  <h2>访问控制</h2>
  <AsyncState :loading="loading" :error="error" @retry="load">
    <form class="access-form" @submit.prevent="save">
      <fieldset :disabled="busy">
        <legend>访问密码</legend>
        <label class="toggle"><input v-model="local" type="checkbox" />本机访问需要登录</label>
        <label class="toggle"><input v-model="lan" type="checkbox" />局域网访问需要 HTTPS 登录</label>
        <div class="credential-fields">
          <label v-if="settings.password_set">当前密码
            <input v-model="currentPassword" type="password" autocomplete="current-password" required />
          </label>
          <template v-if="local || lan">
            <label>{{ settings.password_set ? '新密码（可选）' : '访问密码' }}
              <input v-model="password" type="password" autocomplete="new-password" minlength="12" :required="!settings.password_set" />
            </label>
            <label>确认新密码
              <input v-model="confirmation" type="password" autocomplete="new-password" :required="!!password" />
            </label>
            <p class="muted">密码至少 12 个字符。保存后会撤销已有浏览器会话。</p>
          </template>
          <p v-else class="muted">保存后将移除访问密码并撤销已有会话。</p>
        </div>
        <details>
          <summary>HTTPS 反向代理</summary>
          <label class="toggle"><input v-model="trustProxy" type="checkbox" />信任本机 HTTPS 代理</label>
          <p class="muted">代理需与 LabSentinel 在同一台电脑上，保留外部 Host，并覆盖转发协议和客户端地址。</p>
        </details>
        <div class="row access-actions">
          <button type="submit">{{ busy ? '保存中…' : '保存访问设置' }}</button>
          <a v-if="settings.password_set && settings.from_local" href="#/recover">忘记密码</a>
        </div>
      </fieldset>
    </form>

    <section class="browser-sessions" aria-labelledby="sessions-heading">
      <h3 id="sessions-heading">浏览器会话</h3>
      <p v-if="!settings.sessions.length" class="muted">暂无已登录的浏览器。</p>
      <ul v-else>
        <li v-for="(session, index) in settings.sessions" :key="index">
          <b>{{ session.local ? '本机' : '局域网' }}<template v-if="session.current"> · 当前浏览器</template></b>
          <span class="muted">{{ date(session.started) }} 登录 · {{ date(session.expires) }} 到期</span>
        </li>
      </ul>
      <button v-if="settings.sessions.some(session => !session.current)" type="button" class="ghost"
              :disabled="busy" @click="revokeOthers">撤销其他会话</button>
    </section>
  </AsyncState>
  <p v-if="message" class="access-message" role="status">{{ message }}</p>

  <details class="api-credential">
    <summary>高级 · API 凭据</summary>
    <p class="muted">现有脚本和集成可以继续使用服务端管理令牌。本浏览器也可保存令牌供旧访问方式使用。</p>
    <form class="credential-form" @submit.prevent="saveToken">
      <label>管理令牌<input v-model="token" type="password" autocomplete="off" /></label>
      <button type="submit" class="ghost">保存 API 凭据</button>
    </form>
    <p v-if="tokenMessage" role="status">{{ tokenMessage }}</p>
  </details>
</template>

<style scoped>
h2 { font-size: 18px; margin-bottom: 12px; }
fieldset { display: grid; place-items: start; gap: 9px; border: 0; min-width: 0; }
legend { margin-bottom: 6px; padding: 0; font-weight: 600; }
.toggle { display: flex; align-items: center; gap: 6px; min-height: 32px; }
input { margin: 0; }
.credential-fields { width: min(100%, 420px); display: grid; gap: 8px; }
.credential-fields label, .credential-form label { display: grid; gap: 4px; }
.access-form details p { margin: 4px 0 8px; }
summary { cursor: pointer; }
.browser-sessions, .api-credential { border-top: 1px solid var(--divider); margin-top: 14px; padding-top: 12px; }
.browser-sessions ul { list-style: none; margin: 8px 0; }
.browser-sessions li { display: flex; flex-wrap: wrap; gap: 4px 12px; padding: 6px 0; }
.credential-form { display: flex; align-items: end; flex-wrap: wrap; gap: 8px; margin: 6px 0; }
.credential-form label { flex: 1 1 220px; }
.access-message { margin-top: 10px; }
.access-actions { margin: 0; }
.access-actions a { color: var(--accent-strong); }
</style>
