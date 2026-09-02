// Tauri 后端契约的唯一封装层：组件与 store 只允许经这里调 invoke/listen，
// 命令名与事件名集中在此，后端改名时只需改这一处
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  Forward, HostKeyPrompt, LogEntry, Server, Settings, StatusSnapshot, TunnelEvent, UpsertServerInput,
} from './types'

export const api = {
  listServers: () => invoke<Server[]>('list_servers'),
  upsertServer: (input: UpsertServerInput) => invoke<Server>('upsert_server', { input }),
  deleteServer: (id: string) => invoke<void>('delete_server', { id }),
  listForwards: () => invoke<Forward[]>('list_forwards'),
  upsertForward: (forward: Forward) => invoke<Forward>('upsert_forward', { forward }),
  deleteForward: (id: string) => invoke<void>('delete_forward', { id }),
  startForward: (id: string) => invoke<void>('start_forward', { id }),
  stopForward: (id: string) => invoke<void>('stop_forward', { id }),
  connectServer: (id: string) => invoke<void>('connect_server', { id }),
  disconnectServer: (id: string) => invoke<void>('disconnect_server', { id }),
  getSnapshot: () => invoke<StatusSnapshot>('get_snapshot'),
  getSettings: () => invoke<Settings>('get_settings'),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  getLogs: () => invoke<LogEntry[]>('get_logs'),
  respondHostKey: (promptId: string, trust: boolean) =>
    invoke<void>('respond_host_key', { promptId, trust }),
}

export const onTunnelEvent = (cb: (ev: TunnelEvent) => void) =>
  listen<TunnelEvent>('tunnel-event', (e) => cb(e.payload))
export const onLog = (cb: (entry: LogEntry) => void) =>
  listen<LogEntry>('log', (e) => cb(e.payload))
export const onHostKeyPrompt = (cb: (p: HostKeyPrompt) => void) =>
  listen<HostKeyPrompt>('host-key-prompt', (e) => cb(e.payload))
export const onNavigate = (cb: (nav: { view: string; server_id?: string }) => void) =>
  listen<{ view: string; server_id?: string }>('navigate', (e) => cb(e.payload))
