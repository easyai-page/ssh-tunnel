// Tauri 后端契约的唯一封装层：组件与 store 只允许经这里调 invoke/listen，
// 命令名与事件名集中在此，后端改名时只需改这一处
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ElMessage } from 'element-plus'
import { mockApi, mockListen } from './api.mock'
import type {
  Forward, HostKeyPrompt, LogEntry, Server, Settings, StatusSnapshot, TunnelEvent, UpsertServerInput,
} from './types'

// 浏览器环境(无 Tauri 运行时,如 vite 独立调试/README 截图)回落到演示 mock;
// 测试环境例外:invoke/listen 已被 vi.mock 接管,必须走真实调用路径
const isTauri =
  (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) ||
  import.meta.env.MODE === 'test'

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

const tauriApi = {
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

export const api = isTauri ? tauriApi : mockApi

export const onTunnelEvent = (cb: (ev: TunnelEvent) => void) =>
  isTauri ? listen<TunnelEvent>('tunnel-event', (e) => cb(e.payload)) : mockListen<TunnelEvent>('tunnel-event', (e) => cb(e.payload))
export const onLog = (cb: (entry: LogEntry) => void) =>
  isTauri ? listen<LogEntry>('log', (e) => cb(e.payload)) : mockListen<LogEntry>('log', (e) => cb(e.payload))
export const onHostKeyPrompt = (cb: (p: HostKeyPrompt) => void) =>
  isTauri ? listen<HostKeyPrompt>('host-key-prompt', (e) => cb(e.payload)) : mockListen<HostKeyPrompt>('host-key-prompt', (e) => cb(e.payload))
export const onNavigate = (cb: (nav: { view: string; server_id?: string }) => void) =>
  isTauri
    ? listen<{ view: string; server_id?: string }>('navigate', (e) => cb(e.payload))
    : mockListen<{ view: string; server_id?: string }>('navigate', (e) => cb(e.payload))
