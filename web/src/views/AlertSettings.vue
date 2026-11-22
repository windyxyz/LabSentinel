<script setup>
import { computed, onMounted, reactive, ref } from 'vue'
import { api } from '../api'
import AsyncState from '../components/AsyncState.vue'

const CHANNELS = [
  {
    key: 'wecom', name: '企业微信', desc: '通过群机器人发送消息',
    fields: [{ k: 'webhook', label: 'Webhook URL', kind: 'url', ph: 'https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=...' }],
    guide: ['① 手机企业微信群 → 右上角 … → 群机器人 → 添加机器人（任意名字，无需管理员）。',
            '② 复制机器人 Webhook 地址（https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=…一长串）→ 粘贴到这里 → 保存。',
            '③ 点「发送测试」，在群里查看 LabSentinel 渠道自检消息。',
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
    key: 'serverchan', name: 'Server酱（微信）', desc: '通过 Server酱 将告警推送到微信',
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
      { k: 'to', label: '收件邮箱', kind: 'text', ph: 'receiver@example.com', wide: true },
    ],
    guide: ['QQ 邮箱取授权码：电脑登录 mail.qq.com → 设置 → 账户 → 往下翻找到「POP3/IMAP/SMTP…服务」→ 开启「SMTP 服务」→ 按提示发短信 → 得到 16 位授权码。',
            '163 邮箱取授权码：登录 mail.163.com → 设置 → POP3/SMTP/IMAP → 开启 SMTP → 弹窗里生成「授权密码」（只显示一次，务必当场复制）。',
            '填写对照：SMTP 服务器=smtp.qq.com（QQ）或 smtp.163.com（163）；端口 465；发件邮箱=你自己邮箱；SMTP 授权码=刚才那串（不是登录密码！）；收件邮箱=要接收告警的邮箱（可以是自己另一个邮箱）。',
            '保存后点「发送测试」，到收件邮箱查看测试邮件。',
            '遇到 535 或 authentication failed 时，请确认 SMTP 已开启，并检查授权码。连接超时请核对端口与防火墙。发送成功后可到垃圾邮件箱查找。'],
  },
  {
    key: 'webhook', name: '通用 Webhook', desc: '将告警作为 JSON 推送到自建服务',
    fields: [
      { k: 'url', label: 'Webhook URL', kind: 'url', ph: 'https://your-server/hook', wide: true },
      { k: 'secret', label: 'HMAC 密钥（可选）', kind: 'secret', ph: '设置后会为请求签名', wide: true },
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

const selected = ref(CHANNELS[0].key)
const channel = computed(() => CHANNELS.find(ch => ch.key === selected.value))
const channels = reactive({})
const forms = reactive({})
const loading = ref(true)
const error = ref('')
const msg = reactive({})
const saving = reactive({})
const testing = reactive({})
const testResult = reactive({})

function syncForm(ch, config) {
  const state = config?.channels?.[ch.key] || {}
  channels[ch.key] = state
  const draft = {}
  for (const field of ch.fields) {
    if (field.kind === 'bool') draft[field.k] = state[field.k] === true
    else if (field.kind === 'secret' || field.kind === 'url') draft[field.k] = ''
    else draft[field.k] = state[field.k] ?? ''
  }
  forms[ch.key] = draft
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const config = await api.alertConfig()
    for (const ch of CHANNELS) syncForm(ch, config)
  } catch (e) {
    error.value = String(e.message || e)
  } finally {
    loading.value = false
  }
}

async function save(ch) {
  if (saving[ch.key] || testing[ch.key]) return
  const payload = { [ch.key]: {} }
  for (const fd of ch.fields) {
    const v = forms[ch.key][fd.k]
    if (fd.kind === 'bool') {
      payload[ch.key][fd.k] = !!v
    } else if (v !== '' && v !== null && v !== undefined) {
      payload[ch.key][fd.k] = fd.kind === 'number' ? Number(v) : String(v).trim()
    }
  }
  saving[ch.key] = true
  msg[ch.key] = ''
  try {
    syncForm(ch, await api.saveAlertConfig(payload))
    delete testResult[ch.key]
    msg[ch.key] = '已保存，配置立即生效。'
  } catch (e) {
    msg[ch.key] = String(e.message || e)
  } finally {
    saving[ch.key] = false
  }
}

async function reset(ch) {
  if (saving[ch.key] || testing[ch.key]) return
  if (!confirm(`清除「${ch.name}」的网页配置？服务将采用启动时提供的设置。`)) return
  saving[ch.key] = true
  msg[ch.key] = ''
  try {
    syncForm(ch, await api.resetAlertConfig(ch.key))
    delete testResult[ch.key]
    msg[ch.key] = '已清除网页配置。'
  } catch (e) {
    msg[ch.key] = String(e.message || e)
  } finally {
    saving[ch.key] = false
  }
}

// UX（2026-11-21）：推送策略——告警随消息带截图 + 同人推送冷却
const pushPolicy = ref({ send_image: true, cooldown_s: 60 })
const pushBusy = ref(false)
const policyMsg = ref('')
async function loadPushPolicy() {
  try {
    const data = await api.pushPolicy()
    if (typeof data?.send_image === 'boolean' && typeof data.cooldown_s === 'number') pushPolicy.value = data
  } catch { /* 保留默认值 */ }
}
async function savePushPolicy() {
  pushBusy.value = true
  try {
    pushPolicy.value = await api.savePushPolicy({
      send_image: pushPolicy.value.send_image,
      cooldown_s: pushPolicy.value.cooldown_s,
    })
    policyMsg.value = '推送策略已保存（立即生效）'
  } catch (e) { policyMsg.value = String(e.message || e) }
  pushBusy.value = false
}

async function test(ch) {
  if (saving[ch.key] || testing[ch.key]) return
  testing[ch.key] = true
  msg[ch.key] = ''
  delete testResult[ch.key]
  try {
    const raw = await api.alertTest(ch.key)
    const list = Array.isArray(raw) ? raw : (Array.isArray(raw?.results) ? raw.results : [])
    const result = list.find(item => item?.channel === ch.key) || list[0]
    testResult[ch.key] = result?.ok === true
      ? { ok: true, text: '测试消息已发送。' }
      : { ok: false, text: result?.error || '服务未返回测试结果。' }
  } catch (e) {
    testResult[ch.key] = { ok: false, text: String(e.message || e) }
  } finally {
    testing[ch.key] = false
  }
}

function sourceText(s) {
  if (!s?.configured) return '未配置'
  if (s.source === 'db') return '已配置 · 网页保存'
  if (s.source === 'env') return '已配置 · 启动配置'
  return '已配置'
}

function channelState(key) {
  if (testing[key]) return '测试中'
  if (testResult[key]) return testResult[key].ok ? '测试成功' : '测试失败'
  if (channels[key]?.configured) return '已配置'
  return '未配置'
}

onMounted(() => { load(); loadPushPolicy() })
</script>

<template>
  <section class="panel alert-channels" aria-labelledby="alert-channels-title">
    <h2 id="alert-channels-title">告警渠道</h2>
    <AsyncState :loading="loading" :error="error" @retry="load">
      <div class="channel-layout">
        <div class="channel-list" role="group" aria-label="选择渠道">
          <button v-for="ch in CHANNELS" :key="ch.key" type="button" class="channel-choice"
                  :aria-pressed="selected === ch.key" @click="selected = ch.key">
            <span>{{ ch.name }}</span>
            <small :class="testResult[ch.key]?.ok === false ? 'st-err' : channels[ch.key]?.configured ? 'st-ok' : 'muted'">
              {{ channelState(ch.key) }}
            </small>
          </button>
        </div>
        <label class="channel-select">
          <span class="muted">选择渠道</span>
          <select v-model="selected">
            <option v-for="ch in CHANNELS" :key="ch.key" :value="ch.key">{{ ch.name }} · {{ channelState(ch.key) }}</option>
          </select>
        </label>

        <form :key="channel.key" class="channel-editor" @submit.prevent="save(channel)">
          <header class="channel-header">
            <div>
              <h3>{{ channel.name }}</h3>
              <p class="muted">{{ channel.desc }}</p>
            </div>
            <span :class="channels[channel.key]?.configured ? 'st-ok' : 'muted'">
              {{ sourceText(channels[channel.key]) }}
            </span>
          </header>

          <fieldset class="fields" :disabled="saving[channel.key]">
            <label v-for="field in channel.fields" :key="field.k" class="field"
                   :class="{ 'field-wide': field.wide || field.kind === 'bool' || channel.fields.length === 1, 'field-toggle': field.kind === 'bool' }">
              <template v-if="field.kind === 'bool'">
                <input type="checkbox" v-model="forms[channel.key][field.k]" />
                <span>{{ field.label }}</span>
              </template>
              <template v-else>
                <span>{{ field.label }}</span>
                <input v-if="field.kind === 'number'" type="number" min="1" max="65535"
                       v-model="forms[channel.key][field.k]" :placeholder="field.ph" />
                <input v-else :type="field.kind === 'secret' ? 'password' : field.kind"
                       v-model="forms[channel.key][field.k]" autocomplete="off"
                       :placeholder="!['secret', 'url'].includes(field.kind) || !channels[channel.key]?.[`has_${field.k}`]
                         ? field.ph : '已配置（留空保持原值）'" />
                <small v-if="field.kind === 'url' && channels[channel.key]?.[`has_${field.k}`]" class="muted saved-address">
                  {{ channels[channel.key][field.k] }}
                </small>
              </template>
            </label>
          </fieldset>

          <div class="channel-actions">
            <button type="submit" :disabled="saving[channel.key] || testing[channel.key]">
              {{ saving[channel.key] ? '保存中…' : '保存' }}
            </button>
            <button type="button" class="ghost" :disabled="saving[channel.key] || testing[channel.key]" @click="test(channel)">
              {{ testing[channel.key] ? '发送中…' : '发送测试' }}
            </button>
            <button type="button" class="ghost" :disabled="saving[channel.key] || testing[channel.key] || channels[channel.key]?.source !== 'db'"
                    @click="reset(channel)">清除网页配置</button>
          </div>
          <p v-if="msg[channel.key]" class="channel-message" role="status">{{ msg[channel.key] }}</p>
          <p v-if="testResult[channel.key]" class="channel-message" role="status"
             :class="testResult[channel.key].ok ? 'st-ok' : 'st-err'">
            {{ testResult[channel.key].ok ? testResult[channel.key].text : `测试失败 · ${testResult[channel.key].text}` }}
          </p>

          <details class="channel-guide">
            <summary>配置指引</summary>
            <ul class="muted">
              <li v-for="step in channel.guide" :key="step">{{ step }}</li>
            </ul>
            <a v-if="channel.link" :href="channel.link.href" target="_blank" rel="noopener noreferrer">{{ channel.link.label }}</a>
          </details>
        </form>
      </div>
    </AsyncState>
  </section>

  <section class="panel push-policy" aria-labelledby="push-policy-title">
    <h3 id="push-policy-title">推送策略</h3>
    <label class="field-toggle">
      <input v-model="pushPolicy.send_image" type="checkbox" />
      <span>随告警附截图（企业微信、Webhook）</span>
    </label>
    <label class="policy-cooldown">
      <span>重复推送冷却</span>
      <input v-model.number="pushPolicy.cooldown_s" type="number" min="0" max="3600" />
      <span>秒</span>
    </label>
    <p class="muted">冷却期间，同路摄像头的同类重复事件仍会记录。填 0 可关闭冷却。</p>
    <div class="channel-actions">
      <button :disabled="pushBusy" @click="savePushPolicy">{{ pushBusy ? '保存中…' : '保存推送策略' }}</button>
      <span v-if="policyMsg" role="status">{{ policyMsg }}</span>
    </div>
  </section>
</template>

<style scoped>
.alert-channels h2 { font-size: 18px; margin-bottom: 12px; }
.channel-layout { display: grid; grid-template-columns: minmax(170px, 225px) minmax(0, 1fr); gap: 18px; align-items: start; }
.channel-list { display: grid; gap: 4px; }
.channel-select { display: none; }
.channel-choice {
  justify-content: space-between; text-align: left; min-height: 37px;
  background: transparent; color: var(--text-primary); padding: 6px 9px;
}
.channel-choice small { font-size: 11px; flex-shrink: 0; }
.channel-choice:hover { background: var(--subtle-hover); }
.channel-choice[aria-pressed="true"] {
  background: color-mix(in srgb, var(--accent) 9%, var(--card-bg));
  border-color: var(--stroke-secondary); font-weight: 600;
}
.channel-editor { min-width: 0; padding: 0 0 0 18px; border-left: 1px solid var(--divider); }
.channel-header { display: flex; justify-content: space-between; align-items: baseline; flex-wrap: wrap; gap: 5px 16px; }
.channel-header p, .channel-header > span { font-size: 12px; }
.fields { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px 12px; border: 0; padding: 0; min-width: 0; margin: 14px 0 12px; }
.field { display: flex; flex-direction: column; gap: 4px; min-width: 0; font-size: 13px; }
.field-wide { grid-column: 1 / -1; }
.field-toggle { display: flex; flex-direction: row; gap: 8px; align-items: center; min-height: 32px; cursor: pointer; }
.field input, .channel-select select, .push-policy input { margin: 0; }
.field input:not([type="checkbox"]) { width: 100%; min-width: 0; min-height: 36px; }
.field input::placeholder { color: var(--text-secondary); opacity: 1; }
.saved-address, .channel-message, .channel-guide { overflow-wrap: anywhere; }
.channel-actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.channel-actions .ghost:disabled { color: var(--text-disabled); cursor: default; }
.channel-message { margin-top: 8px; }
.channel-guide { margin-top: 10px; color: var(--text-secondary); }
.channel-guide summary { cursor: pointer; }
.channel-guide ul { margin: 5px 0 8px 20px; }
.channel-guide a { color: var(--accent-strong); }
.push-policy h3 { margin-bottom: 5px; }
.policy-cooldown { display: flex; align-items: center; gap: 6px; margin-top: 4px; }
.policy-cooldown input { width: 75px; }
.push-policy p { font-size: 12px; margin: 5px 0 9px; }
@media (max-width: 870px) {
  .channel-layout { grid-template-columns: minmax(0, 1fr); gap: 14px; }
  .channel-list { display: none; }
  .channel-select { display: grid; gap: 4px; }
  .channel-editor { padding: 0; border-left: 0; }
}
@media (max-width: 465px) {
  .alert-channels, .push-policy { padding: 14px; }
  .fields { grid-template-columns: minmax(0, 1fr); }
  .channel-actions button { min-height: 40px; }
}
</style>
