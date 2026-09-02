import { defineStore } from 'pinia'
import { api, onLog } from '../api'
import type { LogEntry } from '../types'

export const useLogsStore = defineStore('logs', {
  state: () => ({ entries: [] as LogEntry[] }),
  actions: {
    async load() {
      this.entries = await api.getLogs()
    },
    clear() {
      this.entries = []
    },
  },
})

// 事件绑定幂等:重复调用只绑一次
let bound = false
export function bindLogsEvents() {
  if (bound) return
  bound = true
  onLog((entry) => {
    useLogsStore().entries.push(entry)
  })
}
