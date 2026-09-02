// Tauri 后端契约的唯一封装层：组件与 store 只允许经这里调 invoke/listen，
// 命令名与事件名集中在此，后端改名时只需改这一处
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ElMessage } from 'element-plus'
import type {
  Forward, HostKeyPrompt, LogEntry, Server, Settings, StatusSnapshot, TunnelEvent, UpsertServerInput,
} from './types'

// 所有 command 调用经此封装：invoke 失败时弹出用户可读的错误提示
// (Rust 侧错误串本身就是面向用户的文案)，再原样抛出,
// 由调用方决定后续动作(如保存失败时保持对话框打开)
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args)
  } catch (e) {
    ElMessage.error(String(e))
    throw e
  }
}

export const api = {
  listServers: () => call<Server[]>('list_servers'),
  upsertServer: (input: UpsertServerInput) => call<Server>('upsert_server', { input }),
  deleteServer: (id: string) => call<void>('delete_server', { id }),
  listForwards: () => call<Forward[]>('list_forwards'),
  upsertForward: (forward: Forward) => call<Forward>('upsert_forward', { forward }),
  deleteForward: (id: string) => call<void>('delete_forward', { id }),
  startForward: (id: string) => call<void>('start_forward', { id }),
  stopForward: (id: string) => call<void>('stop_forward', { id }),
  connectServer: (id: string) => call<void>('connect_server', { id }),
  disconnectServer: (id: string) => call<void>('disconnect_server', { id }),
  getSnapshot: () => call<StatusSnapshot>('get_snapshot'),
  getSettings: () => call<Settings>('get_settings'),
  saveSettings: (settings: Settings) => call<void>('save_settings', { settings }),
  getLogs: () => call<LogEntry[]>('get_logs'),
  respondHostKey: (promptId: string, trust: boolean) =>
    call<void>('respond_host_key', { promptId, trust }),
}

export const onTunnelEvent = (cb: (ev: TunnelEvent) => void) =>
  listen<TunnelEvent>('tunnel-event', (e) => cb(e.payload))
export const onLog = (cb: (entry: LogEntry) => void) =>
  listen<LogEntry>('log', (e) => cb(e.payload))
export const onHostKeyPrompt = (cb: (p: HostKeyPrompt) => void) =>
  listen<HostKeyPrompt>('host-key-prompt', (e) => cb(e.payload))
export const onNavigate = (cb: (nav: { view: string; server_id?: string }) => void) =>
  listen<{ view: string; server_id?: string }>('navigate', (e) => cb(e.payload))
