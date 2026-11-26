<script setup>
import { computed, nextTick, onMounted, ref } from 'vue'
import { api } from '../api'
import AsyncState from '../components/AsyncState.vue'

const switches = [
  { key: 'enabled', label: '启用 MCP 服务' },
  { key: 'allow_lan', label: '允许局域网连接', description: '默认仅本机可连接。局域网连接需配置受信任的 HTTPS 反向代理。',
    confirmation: '允许局域网中的客户端通过受信任的 HTTPS 代理连接 MCP 服务？' },
  { key: 'allow_image_content', label: '允许返回事件缩略图', description: '客户端可在令牌授权的机位范围内读取事件缩略图。' },
  { key: 'allow_global_silence', label: '允许全局告警静默', description: '启用「创建告警静默」工具后，此权限允许客户端静默全部机位的告警。',
    confirmation: '允许获授权的客户端静默全部机位的告警？' },
]
const scopes = { read_only: '只读', policy: '查询与获准操作' }
const emptyForm = () => ({ name: '', scope: 'read_only', allCameras: true, cameraIds: [], ttlDays: '' })
const config = ref(null)
const configLoading = ref(true)
const configLoadError = ref('')
const savingConfig = ref(false)
const configStatus = ref('')
const configError = ref('')
const cameras = ref([])
const camerasLoading = ref(true)
const camerasError = ref('')
const tokens = ref([])
const tokensLoading = ref(true)
const tokensLoadError = ref('')
const tokenBusy = ref(false)
const tokenStatus = ref('')
const tokenError = ref('')
const form = ref(emptyForm())
const issued = ref(null)
const secretField = ref(null)
const copyStatus = ref('')

const endpoint = computed(() => `${window.location.origin}${config.value?.endpoint ?? ''}`)
const readTools = computed(() => config.value?.tools.filter(tool => !tool.write) ?? [])
const writeTools = computed(() => config.value?.tools.filter(tool => tool.write) ?? [])
const cameraNames = computed(() => new Map(cameras.value.map(camera => [camera.id, camera.name])))
const canIssue = computed(() => form.value.name.trim() && (form.value.allCameras || form.value.cameraIds.length > 0))

function parseConfig(next) {
  if (next?.endpoint !== '/mcp' || !switches.every(option => typeof next[option.key] === 'boolean') ||
      !Array.isArray(next.allowed_write_tools) || !next.allowed_write_tools.every(name => typeof name === 'string') ||
      !Array.isArray(next.tools) || !next.tools.every(tool =>
        typeof tool?.name === 'string' && typeof tool.label === 'string' && typeof tool.description === 'string' &&
        typeof tool.write === 'boolean' && typeof tool.high === 'boolean')) {
    throw new Error('服务未返回有效的 MCP 配置。')
  }
  return next
}

async function loadConfig() {
  configLoading.value = true
  configLoadError.value = ''
  try { config.value = parseConfig(await api.mcpConfig()) }
  catch (err) { configLoadError.value = err.message }
  finally { configLoading.value = false }
}

async function saveConfig(patch, confirmation = '', status = '设置已生效。') {
  if (savingConfig.value || (confirmation && !window.confirm(confirmation))) return
  savingConfig.value = true
  configStatus.value = configError.value = ''
  try {
    const payload = confirmation ? { ...patch, confirm_high_risk: true } : patch
    config.value = parseConfig(await api.saveMcpConfig(payload))
    configStatus.value = status
  } catch (err) { configError.value = err.message }
  finally { savingConfig.value = false }
}

function changeSwitch(event, option) {
  const checked = event.target.checked
  event.target.checked = config.value[option.key]
  saveConfig({ [option.key]: checked }, checked ? option.confirmation : '')
}

function changeTool(event, tool) {
  const checked = event.target.checked
  const previous = config.value.allowed_write_tools
  event.target.checked = previous.includes(tool.name)
  const allowed = checked ? [...previous, tool.name] : previous.filter(name => name !== tool.name)
  const confirmation = checked && tool.high ? `允许获授权的客户端执行「${tool.label}」？\n${tool.description}` : ''
  saveConfig({ allowed_write_tools: allowed }, confirmation)
}

function restoreReadOnly() {
  saveConfig({ allowed_write_tools: [] }, '', '已恢复严格只读。')
}

const isCameraId = id => Number.isSafeInteger(id) && id > 0
const isTokenId = id => (Number.isSafeInteger(id) && id > 0) || (typeof id === 'string' && id.trim() !== '')
const isOptionalTime = ts => ts === null || Number.isFinite(ts)

function isToken(row) {
  return isTokenId(row?.id) && typeof row.name === 'string' && typeof row.token_prefix === 'string' &&
    Object.hasOwn(scopes, row.scope) && typeof row.revoked === 'boolean' && Number.isFinite(row.created_at) &&
    isOptionalTime(row.last_used_at) && isOptionalTime(row.expires_at) &&
    (row.camera_ids === null || (Array.isArray(row.camera_ids) && row.camera_ids.every(isCameraId)))
}

async function loadTokens() {
  tokensLoading.value = true
  tokensLoadError.value = ''
  try {
    const rows = await api.mcpTokens()
    if (!Array.isArray(rows) || !rows.every(isToken)) throw new Error('服务未返回有效的 MCP 令牌列表。')
    tokens.value = rows
  } catch (err) { tokensLoadError.value = err.message }
  finally { tokensLoading.value = false }
}

async function loadCameras() {
  camerasLoading.value = true
  camerasError.value = ''
  try {
    const rows = await api.cameras()
    if (!Array.isArray(rows)) throw new Error('服务未返回机位列表。')
    cameras.value = rows.filter(camera => isCameraId(camera?.id))
      .map(({ id, name }) => ({ id, name: typeof name === 'string' && name.trim() ? name : `机位 #${id}` }))
  } catch (err) { camerasError.value = err.message }
  finally { camerasLoading.value = false }
}

async function issueToken() {
  if (tokenBusy.value || !canIssue.value || issued.value) return
  const { name, scope, allCameras, cameraIds, ttlDays } = form.value
  tokenBusy.value = true
  tokenStatus.value = tokenError.value = ''
  try {
    const result = await api.createMcpToken({
      name: name.trim(), scope, camera_ids: allCameras ? null : cameraIds, ttl_days: ttlDays === '' ? undefined : ttlDays,
    })
    if (!isTokenId(result?.id) || typeof result.token !== 'string' || !result.token.startsWith('lsmcp_')) {
      throw new Error('服务未返回有效的 MCP 令牌，请刷新列表核对签发结果。')
    }
    issued.value = { id: result.id, token: result.token }
    form.value = emptyForm()
    await nextTick()
    secretField.value?.focus()
    await loadTokens()
  } catch (err) { tokenError.value = err.message }
  finally { tokenBusy.value = false }
}

function hideToken() {
  issued.value = null
  copyStatus.value = ''
}

async function copyToken() {
  if (!issued.value) return
  try {
    await navigator.clipboard.writeText(issued.value.token)
    copyStatus.value = '已复制令牌。'
  } catch {
    secretField.value?.focus()
    secretField.value?.select()
    copyStatus.value = '已选中令牌，可使用复制快捷键。'
  }
}

async function revokeToken(id) {
  if (tokenBusy.value) return
  tokenBusy.value = true
  tokenStatus.value = tokenError.value = ''
  try {
    await api.revokeMcpToken(id)
    if (issued.value?.id === id) hideToken()
    tokenStatus.value = '令牌已撤销。'
    await loadTokens()
  } catch (err) { tokenError.value = err.message }
  finally { tokenBusy.value = false }
}

function cameraScope(ids) {
  if (ids === null) return '所有机位'
  return ids.map(id => cameraNames.value.get(id) ?? `机位 #${id}`).join('、') || '无授权机位'
}

function date(ts, fallback) {
  return ts === null ? fallback : new Date(ts * 1000).toLocaleString('zh-CN')
}

onMounted(() => {
  loadConfig()
  loadTokens()
  loadCameras()
})
</script>

<template>
  <h2>MCP 接入</h2>
  <AsyncState :loading="configLoading" :error="configLoadError" @retry="loadConfig">
    <section class="mcp-group" aria-labelledby="mcp-connection-heading">
      <h3 id="mcp-connection-heading">连接设置</h3>
      <p class="muted">客户端使用 Streamable HTTP，地址为 <code>{{ endpoint }}</code>，请求头为 <code>Authorization: Bearer &lt;令牌&gt;</code>。</p>
      <fieldset class="mcp-toggles" aria-label="服务选项" :disabled="savingConfig">
        <label v-for="option in switches" :key="option.key" class="mcp-toggle">
          <input type="checkbox" :checked="config[option.key]" @change="changeSwitch($event, option)" />
          <span>{{ option.label }}<small v-if="option.description" class="muted">{{ option.description }}</small></span>
        </label>
      </fieldset>
    </section>

    <section class="mcp-group" aria-labelledby="mcp-tools-heading">
      <div class="mcp-heading-row">
        <h3 id="mcp-tools-heading">工具权限</h3>
        <button type="button" class="ghost" :disabled="savingConfig || !config.allowed_write_tools.length" @click="restoreReadOnly">
          一键恢复严格只读
        </button>
      </div>
      <p class="muted">当前{{ config.allowed_write_tools.length ? `已授权 ${config.allowed_write_tools.length} 项写入工具` : '为严格只读' }}。写入操作还需使用「查询与获准操作」令牌。</p>
      <details>
        <summary>默认只读工具（{{ readTools.length }} 项）</summary>
        <ul class="mcp-read-tools">
          <li v-for="tool in readTools" :key="tool.name">{{ tool.label }} <code>{{ tool.name }}</code></li>
        </ul>
      </details>
      <fieldset class="mcp-toggles" aria-label="逐项授权写入工具" :disabled="savingConfig">
        <label v-for="tool in writeTools" :key="tool.name" class="mcp-toggle">
          <input type="checkbox" :checked="config.allowed_write_tools.includes(tool.name)" @change="changeTool($event, tool)" />
          <span>{{ tool.label }} <span v-if="tool.high" class="risk-label">高风险</span>
            <small class="muted">{{ tool.description }}</small>
          </span>
        </label>
      </fieldset>
      <p v-if="configStatus" role="status">{{ configStatus }}</p>
      <p v-if="configError" class="st-err" role="alert">{{ configError }}</p>
    </section>

    <section class="mcp-group" aria-labelledby="mcp-tokens-heading">
      <div class="mcp-heading-row">
        <h3 id="mcp-tokens-heading">客户端令牌</h3>
        <button type="button" class="ghost" :disabled="tokenBusy || tokensLoading" @click="loadTokens">刷新令牌列表</button>
      </div>
      <div v-if="issued" class="mcp-issued">
        <label for="mcp-issued-token">完整令牌仅在此处显示一次，请复制后妥善保存。</label>
        <div class="mcp-heading-row">
          <input id="mcp-issued-token" ref="secretField" :value="issued.token" readonly autocomplete="off" spellcheck="false" />
          <button type="button" @click="copyToken">复制令牌</button>
          <button type="button" class="ghost" @click="hideToken">关闭</button>
        </div>
        <p v-if="copyStatus" role="status">{{ copyStatus }}</p>
      </div>
      <form v-else class="mcp-token-form" @submit.prevent="issueToken">
        <fieldset :disabled="tokenBusy">
          <div class="mcp-form-fields">
            <label>令牌名称<input v-model.trim="form.name" required autocomplete="off" /></label>
            <label>权限
              <select v-model="form.scope">
                <option v-for="(label, scope) in scopes" :key="scope" :value="scope">{{ label }}</option>
              </select>
            </label>
            <label>机位范围
              <select v-model="form.allCameras">
                <option :value="true">所有机位</option>
                <option :value="false">所选机位</option>
              </select>
            </label>
            <label>有效期（天）
              <input v-model="form.ttlDays" type="number" min="1" max="365" step="1" placeholder="留空为不设期限" />
            </label>
          </div>
          <AsyncState v-if="!form.allCameras" :loading="camerasLoading" :error="camerasError"
                      :empty="!cameras.length" empty-text="暂无可选机位。" @retry="loadCameras">
            <fieldset class="mcp-camera-options">
              <legend>选择可访问的机位</legend>
              <label v-for="camera in cameras" :key="camera.id">
                <input v-model="form.cameraIds" type="checkbox" :value="camera.id" />{{ camera.name }}
              </label>
            </fieldset>
          </AsyncState>
          <button type="submit" :disabled="!canIssue">签发令牌</button>
        </fieldset>
      </form>
      <p v-if="tokenStatus" role="status">{{ tokenStatus }}</p>
      <p v-if="tokenError" class="st-err" role="alert">{{ tokenError }}</p>
      <AsyncState :loading="tokensLoading" :error="tokensLoadError" :empty="!tokens.length" empty-text="暂无 MCP 令牌。" @retry="loadTokens">
        <ul class="mcp-token-list">
          <li v-for="token in tokens" :key="token.id">
            <div class="mcp-heading-row">
              <strong>{{ token.name }} <code class="muted">{{ token.token_prefix }}…</code></strong>
              <span v-if="token.revoked" class="muted">已撤销</span>
              <button v-else type="button" class="ghost" :aria-label="`撤销令牌 ${token.name}`" :disabled="tokenBusy" @click="revokeToken(token.id)">撤销</button>
            </div>
            <p>{{ scopes[token.scope] }} · {{ cameraScope(token.camera_ids) }}</p>
            <p class="muted mcp-times">
              <span>签发于 {{ date(token.created_at) }}</span>
              <span>最近使用 {{ date(token.last_used_at, '从未使用') }}</span>
              <span>到期 {{ date(token.expires_at, '无预设期限') }}</span>
            </p>
          </li>
        </ul>
      </AsyncState>
    </section>
  </AsyncState>
</template>

<style scoped>
h2 { font-size: 18px; }
.mcp-group { display: grid; gap: 10px; padding-top: 14px; }
.mcp-group + .mcp-group { margin-top: 14px; border-top: 1px solid var(--divider); }
.mcp-heading-row { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 8px; }
fieldset { min-width: 0; border: 0; }
input, select { margin: 0; }
.mcp-toggles, .mcp-token-form > fieldset { display: grid; gap: 8px; }
.mcp-toggle { display: flex; align-items: start; gap: 8px; min-height: 32px; }
.mcp-toggle input { margin-top: 4px; }
.mcp-toggle small { display: block; }
.risk-label { color: var(--sys-caution); font-size: 12px; }
summary { cursor: pointer; }
code { font-family: var(--font-mono); overflow-wrap: anywhere; }
.mcp-read-tools { padding: 6px 0 0 20px; }
.mcp-form-fields { display: flex; align-items: end; flex-wrap: wrap; gap: 8px 12px; }
.mcp-form-fields label { display: grid; flex: 1 1 135px; gap: 4px; }
.mcp-form-fields input, .mcp-form-fields select { min-width: 0; width: 100%; }
.mcp-token-form button[type="submit"] { justify-self: start; }
.mcp-camera-options { display: flex; flex-wrap: wrap; gap: 6px 14px; }
.mcp-camera-options label { display: flex; align-items: center; gap: 6px; }
.mcp-camera-options legend { margin-bottom: 4px; }
.mcp-issued { display: grid; gap: 8px; padding: 10px; border: 1px solid var(--stroke-secondary); border-radius: var(--control-radius); }
.mcp-issued input { flex: 1 1 240px; min-width: 0; font-family: var(--font-mono); }
.mcp-token-list { list-style: none; }
.mcp-token-list li { display: grid; gap: 4px; border-top: 1px solid var(--divider); padding: 8px 0; overflow-wrap: anywhere; }
.mcp-times { display: flex; flex-wrap: wrap; gap: 2px 12px; font-size: 12px; }
</style>
