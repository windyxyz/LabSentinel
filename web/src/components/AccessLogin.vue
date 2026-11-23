<script setup>
import { ref } from 'vue'
import { api } from '../api'

const props = defineProps({ local: Boolean, passwordSupported: Boolean, startRecovery: Boolean })
const emit = defineEmits(['signed-in', 'leave-recovery'])
const logoUrl = '/icons/icon-512-app.png'
const recovering = ref(props.startRecovery)
const password = ref('')
const recoveryCode = ref('')
const confirmPassword = ref('')
const busy = ref(false)
const error = ref('')

function toggleRecovery() {
  if (props.startRecovery) { emit('leave-recovery'); return }
  recovering.value = !recovering.value
  error.value = ''
  password.value = ''
  recoveryCode.value = ''
  confirmPassword.value = ''
}

async function submit() {
  if (busy.value) return
  error.value = ''
  if (recovering.value && password.value !== confirmPassword.value) {
    error.value = '两次输入的密码不一致。'
    return
  }
  busy.value = true
  try {
    if (recovering.value) await api.recoverAccess({ code: recoveryCode.value.trim(), password: password.value })
    else await api.accessLogin(password.value)
    password.value = ''
    recoveryCode.value = ''
    confirmPassword.value = ''
    emit('signed-in')
  } catch (err) { error.value = String(err.message || err) }
  finally { busy.value = false }
}
</script>

<template>
  <main class="login-page">
    <section class="panel login-panel" aria-labelledby="login-heading">
      <div class="brand"><img :src="logoUrl" alt="" />LabSentinel</div>
      <h1 id="login-heading">{{ recovering ? '重设访问密码' : '登录' }}</h1>
      <form v-if="passwordSupported" @submit.prevent="submit">
        <template v-if="recovering">
          <p class="muted">在服务所在电脑的安装目录打开 PowerShell。运行下列命令获取十分钟内有效的恢复码。</p>
          <code>.\labsentinel-server.exe --console --access-recovery-code</code>
          <label>恢复码 <input v-model="recoveryCode" autocomplete="off" spellcheck="false" required /></label>
          <label>新密码 <input v-model="password" type="password" autocomplete="new-password" minlength="12" required /></label>
          <label>确认新密码 <input v-model="confirmPassword" type="password" autocomplete="new-password" required /></label>
        </template>
        <label v-else>访问密码 <input v-model="password" type="password" autocomplete="current-password" required autofocus /></label>
        <p v-if="error" role="alert" class="st-err">{{ error }}</p>
        <div class="row login-actions">
          <button type="submit" :disabled="busy">{{ busy ? '验证中…' : recovering ? '重设并登录' : '登录' }}</button>
          <button v-if="local" type="button" class="ghost" :disabled="busy" @click="toggleRecovery">
            {{ recovering ? '返回' : '忘记密码' }}
          </button>
        </div>
      </form>
      <p v-else class="muted">请使用管理员提供的 HTTPS 地址登录。</p>
      <a class="pair-link" href="#/pair">使用配对码</a>
    </section>
  </main>
</template>

<style scoped>
.login-page { display: grid; min-height: 100vh; max-width: none; place-items: center; }
.login-panel { width: min(100%, 410px); padding: 22px; }
.brand { display: flex; gap: 9px; align-items: center; font-weight: 600; }
.brand img { width: 26px; height: 26px; }
h1 { font-size: 22px; margin: 10px 0 16px; }
form { display: grid; gap: 10px; }
label { display: grid; gap: 4px; }
input { width: 100%; margin: 0; min-height: 40px; }
code { overflow-wrap: anywhere; background: var(--ctrl-fill); padding: 7px; border-radius: var(--control-radius); }
.login-actions { margin-bottom: 0; }
.pair-link { display: inline-block; margin-top: 12px; color: var(--accent-strong); }
</style>
