<script setup>
import { onMounted, reactive, ref } from 'vue'
import { api } from '../api'
import AsyncState from '../components/AsyncState.vue'

const CHANNELS = [
  {
    key: 'wecom', name: '企业微信', desc: '群机器人 Webhook（最简单）',
    fields: [{ k: 'webhook', label: 'Webhook URL', kind: 'url', ph: 'https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=...' }],
    guide: ['企业微信群 → 右上角 … → 群机器人 → 添加机器人 → 复制 Webhook 地址。',
            '该地址含密钥，页面只显示掩码；留空保存表示不修改。'],
  },
  {
    key: 'dingtalk', name: '钉钉', desc: '群自定义机器人 Webhook',
    fields: [{ k: 'webhook', label: 'Webhook URL', kind: 'url', ph: 'https://oapi.dingtalk.com/robot/send?access_token=...' }],
    guide: ['钉钉群 → 群设置 → 机器人 → 添加自定义机器人 → 复制 Webhook。',
            '安全设置请选「自定义关键词」并填 LabSentinel（当前版本不支持加签密钥）。'],
  },
  {
    key: 'feishu', name: '飞书', desc: '群自定义机器人 Webhook',
    fields: [{ k: 'webhook', label: 'Webhook URL', kind: 'url', ph: 'https://open.feishu.cn/open-apis/bot/v2/hook/...' }],
    guide: ['飞书群 → 设置 → 群机器人 → 添加自定义机器人 → 复制 Webhook 地址。'],
  },
  {
    key: 'serverchan', name: 'Server酱（微信）', desc: '通过 Server酱 推送到微信',
    fields: [{ k: 'sendkey', label: 'SendKey', kind: 'secret', ph: 'SCT...' }],
    guide: ['打开 sct.ftqq.com 登录 → SendKey → 复制。',
            '免费额度有限，重要告警建议同时配置企业微信/邮件。'],
  },
  {
    key: 'pushplus', name: 'PushPlus（微信）', desc: '通过 PushPlus 推送到微信',
    fields: [{ k: 'token', label: 'Token', kind: 'secret', ph: 'PushPlus token' }],
    guide: ['打开 pushplus.plus 登录 → 一对一推送 → 复制 token。'],
  },
  {
    key: 'qmsg', name: 'Qmsg酱（QQ）', desc: '推送到指定 QQ',
    fields: [{ k: 'key', label: 'Qmsg Key', kind: 'secret', ph: 'Qmsg key' },
             { k: 'qq', label: '接收 QQ', kind: 'text', ph: '123456789' }],
    guide: ['打开 qmsg.zendee.cn 登录 → 复制 Key；接收 QQ 填自己的 QQ 号。',
            'Key 与 QQ 必须同时填写。'],
  },
  {
    key: 'email', name: '邮件（SMTP）', desc: 'QQ/163/企业邮箱等',
    fields: [
      { k: 'smtp_host', label: 'SMTP 服务器', kind: 'text', ph: 'smtp.qq.com' },
      { k: 'smtp_port', label: '端口', kind: 'number', ph: '465' },
      { k: 'smtp_user', label: '发件邮箱', kind: 'text', ph: 'sender@qq.com' },
      { k: 'smtp_pass', label: 'SMTP 授权码', kind: 'secret', ph: '不是登录密码，是邮箱授权码' },
      { k: 'to', label: '收件邮箱', kind: 'text', ph: 'receiver@example.com' },
    ],
    guide: ['QQ 邮箱：设置 → 账户 → POP3/SMTP 服务 → 生成授权码；服务器 smtp.qq.com，端口 465（SSL）。',
            '163 邮箱：设置 → POP3/SMTP/IMAP → 开启并获取授权码；服务器 smtp.163.com，端口 465。',
            '发件邮箱与 SMTP 用户名通常相同；授权码不是邮箱登录密码。'],
  },
  {
    key: 'webhook', name: '通用 Webhook', desc: '推送到自建服务/n8n/机器人',
    fields: [
      { k: 'url', label: 'Webhook URL', kind: 'url', ph: 'https://your-server/hook' },
      { k: 'secret', label: 'HMAC 密钥（可选）', kind: 'secret', ph: '留空=不签名' },
      { k: 'allow_http', label: '允许 HTTP（仅内网）', kind: 'bool' },
    ],
    guide: ['服务端会 POST JSON（version/title/text/level/ts/event_id 等）。',
            '配置密钥后每个请求带 X-LS-Signature: sha256=<hex>；接收端用同一密钥验签。',
            'HTTP 只建议在可信内网使用，公网请用 HTTPS。'],
  },
]

const status = ref(null)
const forms = reactive({})
const loading = ref(true)
const error = ref('')
const msg = reactive({})
const testing = reactive({})
const testMsg = reactive({})

function initForms() {
  for (const ch of CHANNELS) {
    const s = status.value?.channels?.[ch.key] || {}
    const f = {}
    for (const fd of ch.fields) {
      if (fd.kind === 'bool') f[fd.k] = !!s[fd.k]
      else if (fd.kind === 'secret' || fd.kind === 'url') f[fd.k] = ''
      else f[fd.k] = s[fd.k] ?? ''
    }
    forms[ch.key] = f
    msg[ch.key] = ''
    testMsg[ch.key] = ''
  }
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    status.value = await api.alertConfig()
    initForms()
  } catch (e) {
    error.value = String(e.message || e)
  } finally {
    loading.value = false
  }
}

async function save(ch) {
  const payload = { [ch.key]: {} }
  for (const fd of ch.fields) {
    const v = forms[ch.key][fd.k]
    if (fd.kind === 'bool') {
      payload[ch.key][fd.k] = !!v
    } else if (v !== '' && v !== null && v !== undefined) {
      payload[ch.key][fd.k] = fd.kind === 'number' ? Number(v) : String(v).trim()
    }
  }
  msg[ch.key] = '保存中…'
  try {
    status.value = await api.saveAlertConfig(payload)
    initForms()
    msg[ch.key] = '已保存并生效（无需重启）'
  } catch (e) {
    msg[ch.key] = String(e.message || e)
  }
}

async function reset(ch) {
  if (!confirm(`清除「${ch.name}」在网页保存的配置，回退到启动时 .env？`)) return
  try {
    status.value = await api.resetAlertConfig(ch.key)
    initForms()
    msg[ch.key] = '已回退到 .env 配置'
  } catch (e) { msg[ch.key] = String(e.message || e) }
}

async function test(ch) {
  testing[ch.key] = true
  testMsg[ch.key] = '发送中…'
  try {
    const r = await api.alertTest(ch.key)
    const x = r[0] || {}
    testMsg[ch.key] = x.ok ? '测试消息已发送成功' : `测试失败：${x.error || '未知错误'}`
  } catch (e) {
    testMsg[ch.key] = `测试失败：${e.message || e}`
  }
  testing[ch.key] = false
}

function sourceText(s) {
  if (s?.source === 'db') return '网页保存'
  if (s?.source === 'env') return '.env'
  return '未配置'
}

onMounted(load)
</script>

<template>
  <div class="panel">
    <h3 style="margin-bottom:6px">告警渠道配置</h3>
    <p class="muted" style="margin:0 0 10px">
      在这里填写各渠道凭据，保存后立即生效，不需要改 .env 或重启。
      密钥/完整 Webhook 不会回显；输入框留空保存 = 保持原值；「清除」= 回退到启动时 .env。
      配置保存在本机数据库，随备份一起保存。</p>

    <AsyncState :loading="loading" :error="error" @retry="load">
      <div class="cards">
        <div v-for="ch in CHANNELS" :key="ch.key" class="card channel">
          <div class="row" style="justify-content:space-between; align-items:flex-start">
            <div>
              <b>{{ ch.name }}</b>
              <div class="muted" style="font-size:12px">{{ ch.desc }}</div>
            </div>
            <span :class="status?.channels?.[ch.key]?.configured ? 'st-ok' : 'st-idle'">
              {{ status?.channels?.[ch.key]?.configured ? '已配置' : '未配置' }}
              <span class="muted" style="font-weight:400">· {{ sourceText(status?.channels?.[ch.key]) }}</span>
            </span>
          </div>

          <div class="fields">
            <label v-for="fd in ch.fields" :key="fd.k" class="field">
              <span class="muted">{{ fd.label }}</span>
              <input v-if="fd.kind === 'bool'" type="checkbox" v-model="forms[ch.key][fd.k]" />
              <input v-else-if="fd.kind === 'number'" type="number" min="1" max="65535"
                     v-model="forms[ch.key][fd.k]" :placeholder="fd.ph" />
              <input v-else :type="fd.kind === 'secret' ? 'password' : 'text'"
                     v-model="forms[ch.key][fd.k]"
                     :placeholder="status?.channels?.[ch.key]?.[`has_${fd.k}`]
                       ? '已配置（留空保持不变）' : fd.ph" autocomplete="off" />
              <span v-if="(fd.kind === 'secret' || fd.kind === 'url') && status?.channels?.[ch.key]?.[`has_${fd.k}`]"
                    class="muted" style="font-size:11px">
                当前：{{ status.channels[ch.key][fd.k] || '已设置（不显示）' }}</span>
            </label>
          </div>

          <div class="row" style="margin-top:8px; align-items:center">
            <button @click="save(ch)">保存</button>
            <button class="ghost" :disabled="testing[ch.key]" @click="test(ch)">
              {{ testing[ch.key] ? '发送中…' : '发送测试' }}</button>
            <button class="ghost" @click="reset(ch)">清除</button>
            <span v-if="msg[ch.key]" class="muted" style="font-size:12px">{{ msg[ch.key] }}</span>
            <span v-if="testMsg[ch.key]" class="muted" style="font-size:12px">{{ testMsg[ch.key] }}</span>
          </div>

          <details style="margin-top:8px">
            <summary class="muted" style="cursor:pointer; font-size:12px">配置指引</summary>
            <ul class="muted" style="font-size:12px; margin:6px 0 0 18px">
              <li v-for="(g, i) in ch.guide" :key="i">{{ g }}</li>
            </ul>
          </details>
        </div>
      </div>
    </AsyncState>
  </div>
</template>

<style scoped>
.panel { overflow-x: auto; }
.cards { flex-wrap: wrap; align-items: flex-start; }
.channel { min-width: 360px; max-width: 560px; flex: 1 1 420px; }
.fields { display: flex; flex-direction: column; gap: 6px; margin-top: 10px; }
.field { display: flex; flex-direction: column; gap: 3px; font-size: 12px; }
.field input:not([type="checkbox"]) { min-height: 34px; }
.st-ok { color: #16a34a; font-weight: 600; }
.st-idle { color: #6b7280; }
</style>
