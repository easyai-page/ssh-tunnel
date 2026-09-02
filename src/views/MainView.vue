<script setup lang="ts">
import { computed, ref } from 'vue'
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
    bind_addr: '127.0.0.1', bind_port: 0, target_host: null, target_port: null, auto_start: false,
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
    <aside class="server-list">
      <div class="list-header">
        <span>服务器</span>
        <el-button size="small" type="primary" @click="editingServer = null; serverDialog = true">添加</el-button>
      </div>
      <div v-for="s in servers.servers" :key="s.id"
        :class="['server-item', { active: s.id === servers.selectedId }]" @click="servers.select(s.id)">
        <div class="server-name">{{ s.name }}</div>
        <div class="server-sub">{{ s.username }}@{{ s.host }}:{{ s.port }}</div>
        <el-tooltip :content="servers.serverStatus[s.id]?.error ?? ''"
          :disabled="!servers.serverStatus[s.id]?.error">
          <div class="server-status" :class="servers.serverStatus[s.id]?.status">
            {{ statusText(servers.serverStatus[s.id]?.status) }}
          </div>
        </el-tooltip>
        <div class="server-actions">
          <el-button v-if="canConnect(s.id)" size="small" text type="primary"
            @click.stop="servers.connect(s.id)">连接</el-button>
          <el-button v-if="canDisconnect(s.id)" size="small" text
            @click.stop="servers.disconnect(s.id)">断开</el-button>
          <el-button size="small" text @click.stop="editingServer = s; serverDialog = true">编辑</el-button>
          <el-popconfirm title="删除该服务器及其全部转发?" @confirm="servers.remove(s.id)">
            <template #reference><el-button size="small" text type="danger" @click.stop>删除</el-button></template>
          </el-popconfirm>
        </div>
      </div>
      <el-empty v-if="servers.servers.length === 0" description="还没有服务器,点击「添加」" :image-size="80" />
    </aside>

    <section class="forward-panel">
      <div class="list-header">
        <span>端口转发</span>
        <el-button size="small" type="primary" :disabled="!servers.selectedId"
          @click="editingForward = blankForward(); forwardDialog = true">添加转发</el-button>
      </div>
      <el-table :data="currentForwards" style="width: 100%">
        <el-table-column prop="name" label="名称" min-width="110" />
        <el-table-column label="类型" width="90">
          <template #default="{ row }">{{ forwardKindText(row.kind) }}</template>
        </el-table-column>
        <el-table-column label="监听" width="130">
          <template #default="{ row }">{{ row.bind_addr }}:{{ row.bind_port }}</template>
        </el-table-column>
        <el-table-column label="目标" min-width="140">
          <template #default="{ row }">
            <span v-if="row.kind !== 'dynamic'">{{ row.target_host }}:{{ row.target_port }}</span>
            <span v-else>—</span>
          </template>
        </el-table-column>
        <el-table-column label="状态" width="160">
          <template #default="{ row }">
            <el-tooltip :content="forwards.forwardStatus[row.id]?.error ?? ''"
              :disabled="!forwards.forwardStatus[row.id]?.error">
              <span :class="['fwd-status', forwards.forwardStatus[row.id]?.status]">
                {{ forwardStatusText(forwards.forwardStatus[row.id]?.status) }}
              </span>
            </el-tooltip>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="190">
          <template #default="{ row }">
            <el-switch
              :model-value="['running', 'starting'].includes(forwards.forwardStatus[row.id]?.status ?? '')"
              @change="forwards.toggle(row.id)" />
            <el-button size="small" text @click="editingForward = { ...row }; forwardDialog = true">编辑</el-button>
            <el-popconfirm title="删除该转发?" @confirm="forwards.remove(row.id)">
              <template #reference><el-button size="small" text type="danger">删除</el-button></template>
            </el-popconfirm>
          </template>
        </el-table-column>
      </el-table>
    </section>

    <ServerEditorDialog v-model="serverDialog" :server="editingServer" @submit="saveServer" />
    <ForwardEditorDialog v-if="editingForward" v-model="forwardDialog" :forward="editingForward" @submit="saveForward" />
  </div>
</template>

<style scoped>
.main-view { display: flex; height: 100%; }
.server-list { width: 240px; border-right: 1px solid #e4e7ed; overflow: auto; padding: 8px; }
.list-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px; font-weight: 600; }
.server-item { padding: 8px; border-radius: 6px; cursor: pointer; margin-bottom: 4px; }
.server-item.active { background: #ecf5ff; }
.server-name { font-weight: 600; }
.server-sub { font-size: 12px; color: #909399; }
.server-status { font-size: 12px; color: #909399; }
.server-status.connected { color: #67c23a; }
.server-status.error, .server-status.reconnecting { color: #f56c6c; }
.forward-panel { flex: 1; padding: 8px 16px; overflow: auto; }
.fwd-status.running { color: #67c23a; }
.fwd-status.error { color: #f56c6c; }
</style>
