<script setup>
import { ref } from 'vue'
import { api } from '../api'

const props = defineProps({ code: String, canPair: Boolean })
const emit = defineEmits(['paired'])
const code = ref(props.code || '')
const name = ref('')
const busy = ref(false)
const error = ref('')

async function submit() {
  if (busy.value) return
  busy.value = true
  error.value = ''
  try {
    const result = await api.claimPairing({ code: code.value.trim(), device_name: name.value.trim(), platform: 'web', browser: true })
    if (result?.ok !== true) throw new Error('请使用「设备与配对」为浏览器生成配对码。')
    emit('paired')
  } catch (err) { error.value = String(err.message || err) }
  finally { busy.value = false }
}
</script>

<template>
  <main class="pair-page">
    <section class="panel pair-panel" aria-labelledby="pair-heading">
      <h1 id="pair-heading">配对设备</h1>
      <form v-if="canPair" @submit.prevent="submit">
        <label>设备名称 <input v-model="name" autocomplete="off" maxlength="64" placeholder="例如值班室平板" required /></label>
        <label>配对码 <input v-model="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required /></label>
        <p v-if="error" role="alert" class="st-err">{{ error }}</p>
        <button type="submit" :disabled="busy">{{ busy ? '正在配对…' : '配对并进入值守' }}</button>
      </form>
      <p v-else class="muted">请使用管理员提供的 HTTPS 地址配对局域网设备。</p>
      <a class="back" href="#/dash">返回控制台</a>
    </section>
  </main>
</template>

<style scoped>
.pair-page { display: grid; min-height: 100vh; place-items: center; max-width: none; }
.pair-panel { width: min(100%, 400px); }
h1 { margin-bottom: 12px; font-size: 22px; }
form { display: grid; gap: 10px; }
label { display: grid; gap: 4px; }
input { min-height: 40px; margin: 0; }
button { justify-self: start; }
.back { display: inline-block; margin-top: 12px; color: var(--accent-strong); }
</style>
