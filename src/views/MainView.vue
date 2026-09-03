<script setup lang="ts">
import { computed, ref } from 'vue'
import { Delete, Edit, Guide, Plus } from '@element-plus/icons-vue'
import { useServersStore } from '../stores/servers'
import { useForwardsStore } from '../stores/forwards'
import ServerEditorDialog from '../components/ServerEditorDialog.vue'
import ForwardEditorDialog from '../components/ForwardEditorDialog.vue'
import type { Forward, Server, UpsertServerInput } from '../types'

const servers = useServersStore()
const forwards = useForwardsStore()

const serverDialog = ref(false)
const editingServer = ref<Server | null>(null)
const forwardDialog = ref(false)
const editingForward = ref<Forward | null>(null)

const currentForwards = computed(() => (servers.selectedId ? forwards.forwardsOf(servers.selectedId) : []))

function blankForward(): Forward {
  return {
    id: '', server_id: servers.selectedId ?? '', name: '', kind: 'local',
    bind_addr: '127.0.0.1', bind_port: 0, target_host: '127.0.0.1', target_port: null, auto_start: false,
  }
}

const SERVER_STATUS_TEXT: Record<string, string> = {
  connected: '已连接', connecting: '连接中', reconnecting: '重连中', error: '错误',
}
const FORWARD_STATUS_TEXT: Record<string, string> = { running: '运行中', starting: '启动中', error: '错误' }
const FORWARD_KIND_TEXT: Record<string, string> = { local: '本地', remote: '远程', dynamic: 'SOCKS' }

function statusText(s?: string) {
  return SERVER_STATUS_TEXT[s ?? ''] ?? '未连接'
}
// 状态点颜色分级:绿=正常,黄=进行中/重连,红=错误,灰=未连接
function statusDotClass(s?: string) {
  switch (s) {
    case 'connected': case 'running': return 'ok'
    case 'connecting': case 'reconnecting': case 'starting': return 'pending'
    case 'error': return 'bad'
    default: return 'idle'
  }
}
// 连接/断开按钮的可见性:状态来源与 SERVER_STATUS_TEXT 同为 serverStatus。
// 重连中两个都显示——「连接」等于立即重试,「断开」等于放弃重连
function canConnect(id: string) {
  return !['connected', 'connecting'].includes(servers.serverStatus[id]?.status ?? '')
}
function canDisconnect(id: string) {
  return ['connected', 'connecting', 'reconnecting'].includes(servers.serverStatus[id]?.status ?? '')
}
function forwardStatusText(s?: string) {
  return FORWARD_STATUS_TEXT[s ?? ''] ?? '已停止'
}
function forwardKindText(kind: string) {
  return FORWARD_KIND_TEXT[kind] ?? kind
}

async function saveServer(input: UpsertServerInput) {
  try {
    await servers.save(input)
    serverDialog.value = false
  } catch {
    // 错误提示已由 api 层弹出;保持对话框打开,用户修正后可重试
  }
}
async function saveForward(f: Forward) {
  try {
    await forwards.save(f)
    forwardDialog.value = false
  } catch {
    // 同上:保持对话框打开
  }
}

defineExpose({
  openAddForward(serverId?: string) {
    if (serverId) servers.select(serverId)
    editingForward.value = blankForward()
    forwardDialog.value = true
  },
})
</script>

<template>
  <div class="main-view">
    <aside class="panel server-list">
      <div class="panel-header">
        <span class="panel-title">服务器</span>
        <el-button size="small" type="primary" :icon="Plus"
          @click="editingServer = null; serverDialog = true">添加</el-button>
      </div>
      <div class="server-items">
        <div v-for="s in servers.servers" :key="s.id"
          :class="['server-item', { active: s.id === servers.selectedId }]" @click="servers.select(s.id)">
          <div class="server-top">
            <span class="server-name">{{ s.name }}</span>
            <el-tooltip :content="servers.serverStatus[s.id]?.error ?? ''"
              :disabled="!servers.serverStatus[s.id]?.error">
              <span class="status-line">
                <span class="status-dot" :class="statusDotClass(servers.serverStatus[s.id]?.status)" />
                <span class="status-text">{{ statusText(servers.serverStatus[s.id]?.status) }}</span>
              </span>
            </el-tooltip>
          </div>
          <div class="server-sub">{{ s.username }}@{{ s.host }}:{{ s.port }}</div>
          <div class="server-actions">
            <el-button v-if="canConnect(s.id)" size="small" text type="primary"
              @click.stop="servers.connect(s.id)">连接</el-button>
            <el-button v-if="canDisconnect(s.id)" size="small" text
              @click.stop="servers.disconnect(s.id)">断开</el-button>
            <el-button size="small" text :icon="Edit"
              @click.stop="editingServer = s; serverDialog = true" aria-label="编辑服务器" />
            <el-popconfirm title="删除该服务器及其全部转发?" @confirm="servers.remove(s.id)">
              <template #reference>
                <el-button size="small" text type="danger" :icon="Delete" @click.stop aria-label="删除服务器" />
              </template>
            </el-popconfirm>
          </div>
        </div>
        <el-empty v-if="servers.servers.length === 0" description="还没有服务器" :image-size="80">
          <el-button type="primary" :icon="Plus" @click="editingServer = null; serverDialog = true">
            添加服务器
          </el-button>
        </el-empty>
      </div>
    </aside>

    <section class="panel forward-panel">
      <div class="panel-header">
        <span class="panel-title">端口转发</span>
        <el-button size="small" type="primary" :icon="Plus" :disabled="!servers.selectedId"
          @click="editingForward = blankForward(); forwardDialog = true">添加转发</el-button>
      </div>
      <template v-if="servers.selectedId">
        <el-table :data="currentForwards" style="width: 100%">
          <el-table-column prop="name" label="名称" min-width="110" />
          <el-table-column label="类型" width="90">
            <template #default="{ row }">{{ forwardKindText(row.kind) }}</template>
          </el-table-column>
          <el-table-column label="监听" width="140">
            <template #default="{ row }">
              <span class="mono">{{ row.bind_addr }}:{{ row.bind_port }}</span>
            </template>
          </el-table-column>
          <el-table-column label="目标" min-width="140">
            <template #default="{ row }">
              <span v-if="row.kind !== 'dynamic'" class="mono">{{ row.target_host }}:{{ row.target_port }}</span>
              <span v-else>—</span>
            </template>
          </el-table-column>
          <el-table-column label="状态" width="140">
            <template #default="{ row }">
              <el-tooltip :content="forwards.forwardStatus[row.id]?.error ?? ''"
                :disabled="!forwards.forwardStatus[row.id]?.error">
                <span class="status-line">
                  <span class="status-dot" :class="statusDotClass(forwards.forwardStatus[row.id]?.status)" />
                  <span class="status-text">{{ forwardStatusText(forwards.forwardStatus[row.id]?.status) }}</span>
                </span>
              </el-tooltip>
            </template>
          </el-table-column>
          <el-table-column label="操作" width="180">
            <template #default="{ row }">
              <el-switch
                :model-value="['running', 'starting'].includes(forwards.forwardStatus[row.id]?.status ?? '')"
                @change="forwards.toggle(row.id)" />
              <el-button size="small" text :icon="Edit"
                @click="editingForward = { ...row }; forwardDialog = true" aria-label="编辑转发" />
              <el-popconfirm title="删除该转发?" @confirm="forwards.remove(row.id)">
                <template #reference>
                  <el-button size="small" text type="danger" :icon="Delete" aria-label="删除转发" />
                </template>
              </el-popconfirm>
            </template>
          </el-table-column>
        </el-table>
      </template>
      <el-empty v-else description="先在左侧选择一台服务器" :image-size="100">
        <template #image>
          <el-icon :size="48" class="guide-icon"><Guide /></el-icon>
        </template>
      </el-empty>
    </section>

    <ServerEditorDialog v-model="serverDialog" :server="editingServer" @submit="saveServer" />
    <ForwardEditorDialog v-if="editingForward" v-model="forwardDialog" :forward="editingForward" @submit="saveForward" />
  </div>
</template>

<style scoped>
.main-view {
  display: flex;
  gap: 16px;
  height: 100%;
  padding: 16px;
  box-sizing: border-box;
}

/* 面板卡片:浅色下白底浮于灰页底,深色下实心面板色 */
.panel {
  background: var(--app-panel-bg);
  border: 1px solid var(--app-border);
  border-radius: 8px;
  box-shadow: var(--app-panel-shadow);
  padding: 16px;
  overflow: auto;
}

.panel-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}
.panel-title { font-weight: 600; font-size: 15px; }

.server-list { width: 264px; flex-shrink: 0; }
.forward-panel { flex: 1; min-width: 0; }

.server-item {
  position: relative;
  padding: 10px 12px;
  border-radius: 8px;
  cursor: pointer;
  margin-bottom: 6px;
  transition: background-color 160ms ease;
}
.server-item:hover { background: var(--app-hover-bg); }
.server-item.active { background: var(--app-active-bg); }
/* 选中指示:左侧主色竖条 */
.server-item.active::before {
  content: '';
  position: absolute;
  left: 0;
  top: 20%;
  bottom: 20%;
  width: 3px;
  border-radius: 2px;
  background: var(--el-color-primary);
}

.server-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}
.server-name { font-weight: 600; font-size: 14px; }
.server-sub {
  font-size: 12px;
  color: var(--app-text-secondary);
  font-family: var(--app-font-mono);
  margin-top: 2px;
}

/* 状态点:颜色分级 + 进行中脉冲 */
.status-line { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; }
.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--app-text-secondary);
}
.status-dot.ok { background: var(--app-success); }
.status-dot.bad { background: var(--app-danger); }
.status-dot.pending {
  background: var(--app-warning);
  animation: pulse 1.2s ease-in-out infinite;
}
.status-text { font-size: 12px; color: var(--app-text-secondary); }

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}

/* 操作按钮平时隐藏,hover/选中时浮现,减少视觉噪音 */
.server-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-top: 6px;
  opacity: 0;
  transition: opacity 160ms ease;
}
.server-item:hover .server-actions,
.server-item.active .server-actions { opacity: 1; }
.server-actions .el-button + .el-button { margin-left: 0; }

.mono { font-family: var(--app-font-mono); font-size: 12px; }
.guide-icon { color: var(--app-text-secondary); }
</style>
