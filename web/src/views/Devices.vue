<script setup>
/** R2-203：设备管理——生成配对码、设备列表、吊销（T-620/Q-11 三级令牌）。 */
import { onMounted, ref } from 'vue'
import { api } from '../api'

const devices = ref([])
const pairing = ref(null) // {code, expires_in_s}
const busy = ref(false)
const msg = ref('')
const confirmId = ref(null)

async function load() {
  try {
    devices.value = await api.listDevices()
  } catch (e) {
    msg.value = String(e.message || e)
  }
}

async function newPairing() {
  busy.value = true
  msg.value = ''
  try {
    pairing.value = await api.createPairing()
  } catch (e) {
    msg.value = `生成失败: ${e.message || e}`
  } finally {
    busy.value = false
  }
}

async function revoke(id) {
  busy.value = true
  try {
    await api.revokeDevice(id)
    confirmId.value = null
    await load()
    msg.value = '已吊销'
  } catch (e) {
    msg.value = `吊销失败: ${e.message || e}`
  } finally {
    busy.value = false
  }
}

const fmtTime = (ts) => (ts ? new Date(ts).toLocaleString() : '从未在线')

onMounted(load)
</script>

<template>
  <div class="devices-page">
    <h2>设备管理</h2>
    <div class="card">
      <h3>配对新设备</h3>
      <p class="muted">
        在手机/平板上输入 6 位配对码完成配对；配对码 5 分钟有效、用一次即作废。
        设备令牌拥有「查看 + 事件处置」权限，不能修改配置。
      </p>
      <button class="btn" :disabled="busy" @click="newPairing">生成配对码</button>
      <div v-if="pairing" class="pairing-box">
        <div class="code">{{ pairing.code }}</div>
        <p class="muted">{{ pairing.expires_in_s }} 秒内有效；只显示一次，关闭页面后无法找回。</p>
      </div>
    </div>

    <div class="card">
      <h3>已配对设备（{{ devices.length }}）</h3>
      <p v-if="!devices.length" class="muted">还没有配对过设备。</p>
      <table v-else class="tbl">
        <thead>
          <tr><th>名称</th><th>平台</th><th>最后在线</th><th>状态</th><th></th></tr>
        </thead>
        <tbody>
          <tr v-for="d in devices" :key="d.id" :class="{ revoked: d.revoked }">
            <td>{{ d.name }}</td>
            <td>{{ d.platform || '—' }}</td>
            <td>{{ fmtTime(d.last_seen_ts) }}</td>
            <td>
              <span :class="d.revoked ? 'st-idle' : 'st-ok'">{{ d.revoked ? '已吊销' : '正常' }}</span>
            </td>
            <td>
              <button v-if="!d.revoked" class="btn btn-danger" :disabled="busy"
                      @click="confirmId = d.id">吊销</button>
              <template v-if="confirmId === d.id">
                <button class="btn btn-danger" :disabled="busy" @click="revoke(d.id)">确认吊销</button>
                <button class="btn" @click="confirmId = null">取消</button>
              </template>
            </td>
          </tr>
        </tbody>
      </table>
      <p v-if="msg" class="msg">{{ msg }}</p>
    </div>
  </div>
</template>

<style scoped>
.pairing-box { margin-top: 12px; text-align: center; }
.pairing-box .code {
  font-size: 40px; letter-spacing: 12px; font-weight: 700;
  padding: 10px 20px; background: var(--bg-soft, #1a2230);
  border-radius: 8px; display: inline-block; user-select: all;
}
.tbl { width: 100%; border-collapse: collapse; margin-top: 8px; }
.tbl th, .tbl td { text-align: left; padding: 6px 10px; border-bottom: 1px solid var(--border, #2a3547); }
.tbl tr.revoked { opacity: 0.55; }
.btn-danger { color: #ff6b6b; }
.msg { margin-top: 8px; }
</style>
