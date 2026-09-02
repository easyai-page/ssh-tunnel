import { defineStore } from 'pinia'
import { api, onHostKeyPrompt, onTunnelEvent } from '../api'
import type { HostKeyPrompt, Server, ServerStatus, StatusEntry, UpsertServerInput } from '../types'

export const useServersStore = defineStore('servers', {
  state: () => ({
    servers: [] as Server[],
    selectedId: null as string | null,
    serverStatus: {} as Record<string, StatusEntry<ServerStatus>>,
    hostKeyPrompt: null as HostKeyPrompt | null,
  }),
  actions: {
    async load() {
      const [servers, snapshot] = await Promise.all([api.listServers(), api.getSnapshot()])
      this.servers = servers
      // 快照字段做兜底:不信任边界输入,缺失时置空表,避免状态表变成 undefined 后读写抛错
      this.serverStatus = snapshot.servers ?? {}
      if (!this.selectedId && servers.length > 0) this.selectedId = servers[0].id
      if (this.selectedId && !servers.some((s) => s.id === this.selectedId)) {
        this.selectedId = servers[0]?.id ?? null
      }
    },
    select(id: string) {
      this.selectedId = id
    },
    async save(input: UpsertServerInput) {
      await api.upsertServer(input)
      await this.load()
    },
    async remove(id: string) {
      await api.deleteServer(id)
      await this.load()
    },
    async connect(id: string) {
      await api.connectServer(id)
    },
    async disconnect(id: string) {
      await api.disconnectServer(id)
    },
    async respondHostKey(trust: boolean) {
      if (this.hostKeyPrompt) {
        await api.respondHostKey(this.hostKeyPrompt.prompt_id, trust)
        this.hostKeyPrompt = null
      }
    },
  },
})

// 事件绑定幂等:重复调用只绑一次
let bound = false
export function bindServersEvents() {
  if (bound) return
  bound = true
  onTunnelEvent((ev) => {
    if (ev.type === 'server_status') {
      const store = useServersStore()
      store.serverStatus[ev.server_id] = { status: ev.status, error: ev.error }
    }
  })
  onHostKeyPrompt((p) => {
    useServersStore().hostKeyPrompt = p
  })
}
