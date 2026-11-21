<script setup>
import { onMounted, reactive, ref } from 'vue'
import { api } from '../api'
import AsyncState from '../components/AsyncState.vue'

const CHANNELS = [
  {
    key: 'wecom', name: '企业微信', desc: '群机器人 Webhook（免费无额度限制，需要企业微信群）',
    fields: [{ k: 'webhook', label: 'Webhook URL', kind: 'url', ph: 'https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=...' }],
    guide: ['① 手机企业微信群 → 右上角 … → 群机器人 → 添加机器人（任意名字，无需管理员）。',
            '② 复制机器人 Webhook 地址（https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=…一长串）→ 粘贴到这里 → 保存。',
            '③ 点「发送测试」——群里应立刻收到一条「LabSentinel 渠道自检测试」。',
            '收不到？检查：机器人是否还在群里（被移除会 404）；地址是否复制完整（key= 后一整串）；网络能否访问 qyapi.weixin.qq.com。',
            '没有企业微信？个人可免费注册（我 → 创建企业，人数不限），建一个只有自己的群即可。'],
  },
  {
    key: 'dingtalk', name: '钉钉', desc: '群自定义机器人 Webhook',
    fields: [{ k: 'webhook', label: 'Webhook URL', kind: 'url', ph: 'https://oapi.dingtalk.com/robot/send?access_token=...' }],
    guide: ['① 钉钉群 → 群设置 → 机器人 → 添加机器人 → 选「自定义」（含 Webhook 图标）。',
            '② 安全设置选「自定义关键词」→ 填 LabSentinel（必须，当前版本不支持加签）。',
            '③ 复制 Webhook 地址粘贴到这里 → 保存 → 点「发送测试」。',
            '收不到？关键词必须含在消息文本里（测试消息含 LabSentinel ✓）；机器人被移除会失败。'],
  },
  {
    key: 'feishu', name: '飞书', desc: '群自定义机器人 Webhook',
    fields: [{ k: 'webhook', label: 'Webhook URL', kind: 'url', ph: 'https://open.feishu.cn/open-apis/bot/v2/hook/...' }],
    guide: ['① 飞书群 → 右上角 … → 设置 → 群机器人 → 添加机器人 → 「自定义机器人」。',
            '② 安全设置勾「自定义关键词」→ 填 LabSentinel。',
            '③ 复制 Webhook 地址粘贴到这里 → 保存 → 点「发送测试」。'],
  },
  {
    key: 'serverchan', name: 'Server酱（微信）', desc: '推送到微信（扫码登录官网取 Key，最省事）',
    fields: [{ k: 'sendkey', label: 'SendKey', kind: 'secret', ph: 'SCT...' }],
    guide: ['① 点下方按钮打开 sct.ftqq.com → 用微信扫码登录（无需注册）。',
            '② 登录后首页即显示 SendKey（SCT 开头）→ 复制粘贴到这里 → 保存。',
            '③ 点「发送测试」——「Server酱」公众号应收到测试消息。',
            '收不到？确认已关注「方糖」公众号（消息经它下发）；免费版每天 5 条上限，超了当天会静默；检查微信是否开了免打扰。',
            '重要告警建议同时配邮件渠道（不限量、可靠）。'],
    link: { href: 'https://sct.ftqq.com/', label: '打开 Server酱官网获取 SendKey' },
  },
  {
    key: 'pushplus', name: 'PushPlus（微信）', desc: '推送到微信（微信扫码登录官网取 Token）',
    fields: [{ k: 'token', label: 'Token', kind: 'secret', ph: 'PushPlus token' }],
    guide: ['① 点下方按钮打开 pushplus.plus → 微信扫码登录。',
            '② 按官网提示关注其公众号（不关注收不到消息）。',
            '③ 「一对一推送」页复制你的 Token → 粘贴到这里 → 保存。',
            '④ 点「发送测试」——公众号应收到测试消息。',
            '收不到？先确认已关注公众号；再看官网「发送记录」页是否有投递记录与失败原因。'],
    link: { href: 'https://www.pushplus.plus/', label: '打开 PushPlus 官网获取 Token' },
  },
  {
    key: 'qmsg', name: 'Qmsg酱（QQ）', desc: '推送到指定 QQ',
    fields: [{ k: 'key', label: 'Qmsg Key', kind: 'secret', ph: 'Qmsg key' },
             { k: 'qq', label: '接收 QQ', kind: 'text', ph: '123456789' }],
    guide: ['① 打开 qmsg.zendee.cn → QQ 授权登录 → 复制你的 Key。',
            '② 「接收 QQ」填你要接收消息的 QQ 号（一般填自己的）→ 与 Key 一起保存。',
            '收不到？Qmsg 机器人需是你的 QQ 好友；官网「消息记录」页可查投递失败原因。'],
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
    guide: ['QQ 邮箱取授权码：电脑登录 mail.qq.com → 设置 → 账户 → 往下翻找到「POP3/IMAP/SMTP…服务」→ 开启「SMTP 服务」→ 按提示发短信 → 得到 16 位授权码。',
            '163 邮箱取授权码：登录 mail.163.com → 设置 → POP3/SMTP/IMAP → 开启 SMTP → 弹窗里生成「授权密码」（只显示一次，务必当场复制）。',
            '填写对照：SMTP 服务器=smtp.qq.com（QQ）或 smtp.163.com（163）；端口 465；发件邮箱=你自己邮箱；SMTP 授权码=刚才那串（不是登录密码！）；收件邮箱=要接收告警的邮箱（可以是自己另一个邮箱）。',
            '保存后点「发送测试」——现在测试会用你网页保存的凭据真实发信（此前版本测试只读环境变量，导致「保存了也报未配置」，已修复）。',
            '常见失败对照：提示含 535 / authentication failed = 授权码错了或没开 SMTP 服务（重新生成授权码）；连接超时 = 端口填错（用 465 不是 25）或防火墙拦了；提示安全/STARTTLS = 端口改 587 试；测试成功但收件箱没有 = 去垃圾邮件箱找，并把发件邮箱加白名单。'],
  },
  {
    key: 'webhook', name: '通用 Webhook', desc: '推送到自建服务/bot——OpenClaw(微信 AI bot)、n8n、Bark、Telegram bot 等',
    fields: [
      { k: 'url', label: 'Webhook URL', kind: 'url', ph: 'https://your-server/hook（openclaw 的 webhook 触发端点）' },
      { k: 'secret', label: 'HMAC 密钥（可选）', kind: 'secret', ph: '留空=不签名' },
      { k: 'allow_http', label: '允许 HTTP（仅内网）', kind: 'bool' },
    ],
    guide: ['服务端会 POST JSON（version/title/text/level/ts/event_id 等）——text 已按「告警文案」模板渲染。',
            'OpenClaw（微信 AI bot）：把 openclaw 里配置的 webhook 触发端点填到 URL，告警即转发微信；收到后可在微信里继续问它详情。',
            '微信推送的更简单替代：Server酱 / PushPlus 渠道（扫码即用，无需自建 bot）。',
            '配置密钥后每个请求带 X-LS-Signature: sha256=<hex>；接收端用同一密钥验签。',
            'HTTP 只建议在可信内网使用，公网请用 HTTPS。',
            '收不到？先用浏览器/curl 访问你填的 URL 确认端点活着；本服务所在机器要能访问该地址（公网地址需本机可出网）。'],
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
    const raw = await api.alertTest(ch.key)
    // 后端形状兼容：数组（Python 契约）或 {results:[...]}（rust）
    const list = Array.isArray(raw) ? raw : (raw.results || [])
    const x = list.find((it) => it.channel === ch.key) || list[0] || {}
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
            <a v-if="ch.link" :href="ch.link.href" target="_blank" rel="noopener"
               style="margin-left:auto; font-size:12.5px">
              ↗ {{ ch.link.label }}</a>
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
