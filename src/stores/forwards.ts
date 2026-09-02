import { defineStore } from 'pinia'
import { api, onTunnelEvent } from '../api'
import type { Forward, ForwardStatus, StatusEntry } from '../types'

export const useForwardsStore = defineStore('forwards', {
  state: () => ({
    forwards: [] as Forward[],
    forwardStatus: {} as Record<string, StatusEntry<ForwardStatus>>,
  }),
  actions: {
    async load() {
      const [forwards, snapshot] = await Promise.all([api.listForwards(), api.getSnapshot()])
      this.forwards = forwards
      // 快照字段做兜底:不信任边界输入,缺失时置空表,避免状态表变成 undefined 后读写抛错
      this.forwardStatus = snapshot.forwards ?? {}
    },
    forwardsOf(serverId: string) {
      return this.forwards.filter((f) => f.server_id === serverId)
    },
    async save(forward: Forward) {
      await api.upsertForward(forward)
      await this.load()
    },
    async remove(id: string) {
      await api.deleteForward(id)
      await this.load()
    },
    async toggle(id: string) {
      const status = this.forwardStatus[id]?.status
      if (status === 'running' || status === 'starting') {
        await api.stopForward(id)
      } else {
        await api.startForward(id)
      }
    },
  },
})

// 事件绑定幂等:重复调用只绑一次
let bound = false
export function bindForwardsEvents() {
  if (bound) return
  bound = true
  onTunnelEvent((ev) => {
    if (ev.type === 'forward_status') {
      const store = useForwardsStore()
      store.forwardStatus[ev.forward_id] = { status: ev.status, error: ev.error }
    }
  })
}
