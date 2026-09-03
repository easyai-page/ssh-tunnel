// 纯浏览器环境(无 Tauri 运行时)的演示实现:vite 独立开发前端、README 截图用。
// api.ts 在检测不到 Tauri 时回落到这里;数据是写死的演示场景,操作只改内存
import type {
  Forward, HostKeyPrompt, LogEntry, Server, Settings, StatusSnapshot, UpsertServerInput,
} from './types'

const servers: Server[] = [
  {
    id: 's1', name: '生产跳板机', host: 'jump.example.com', port: 22, username: 'ops',
    auth: { type: 'key_file', path: '~/.ssh/id_ed25519' },
  },
  {
    id: 's2', name: '内网测试机', host: '192.168.188.210', port: 22, username: 'test',
    auth: { type: 'password' },
  },
]

const forwards: Forward[] = [
  {
    id: 'f1', server_id: 's1', name: '生产 MySQL', kind: 'local',
    bind_addr: '127.0.0.1', bind_port: 3306, target_host: '127.0.0.1', target_port: 3306,
    auto_start: true,
  },
  {
    id: 'f2', server_id: 's1', name: 'Redis 缓存', kind: 'local',
    bind_addr: '127.0.0.1', bind_port: 6379, target_host: '10.0.0.15', target_port: 6379,
    auto_start: true,
  },
  {
    id: 'f3', server_id: 's1', name: 'SOCKS 代理', kind: 'dynamic',
    bind_addr: '127.0.0.1', bind_port: 1080, target_host: null, target_port: null,
    auto_start: false,
  },
  {
    id: 'f4', server_id: 's1', name: 'Webhook 回调', kind: 'remote',
    bind_addr: '0.0.0.0', bind_port: 9000, target_host: '127.0.0.1', target_port: 3000,
    auto_start: false,
  },
]

const snapshot: StatusSnapshot = {
  servers: {
    s1: { status: 'connected', error: null },
    s2: { status: 'disconnected', error: null },
  },
  forwards: {
    f1: { status: 'running', error: null },
    f2: { status: 'running', error: null },
    f4: { status: 'running', error: null },
  },
}

let settings: Settings = { auto_reconnect: true, minimize_to_tray: true, launch_at_login: false }

const logs: LogEntry[] = [
  { timestamp: '10:24:01', level: 'INFO', message: '连接 jump.example.com:22 成功' },
  { timestamp: '10:24:01', level: 'INFO', message: '本地转发已启动: 127.0.0.1:3306 -> 127.0.0.1:3306' },
  { timestamp: '10:24:02', level: 'INFO', message: '本地转发已启动: 127.0.0.1:6379 -> 10.0.0.15:6379' },
  { timestamp: '10:24:02', level: 'INFO', message: '远程转发已启动: 0.0.0.0:9000 -> 127.0.0.1:3000' },
]

export const mockApi = {
  listServers: async () => structuredClone(servers),
  upsertServer: async (input: UpsertServerInput) => {
    const s = { ...input.server, id: input.server.id || `s${Date.now()}` }
    const i = servers.findIndex((x) => x.id === s.id)
    if (i >= 0) servers[i] = s; else servers.push(s)
    return structuredClone(s)
  },
  deleteServer: async (id: string) => {
    const i = servers.findIndex((x) => x.id === id)
    if (i >= 0) servers.splice(i, 1)
  },
  listForwards: async () => structuredClone(forwards),
  upsertForward: async (forward: Forward) => {
    const f = { ...forward, id: forward.id || `f${Date.now()}` }
    const i = forwards.findIndex((x) => x.id === f.id)
    if (i >= 0) forwards[i] = f; else forwards.push(f)
    return structuredClone(f)
  },
  deleteForward: async (id: string) => {
    const i = forwards.findIndex((x) => x.id === id)
    if (i >= 0) forwards.splice(i, 1)
  },
  startForward: async () => {},
  stopForward: async () => {},
  connectServer: async () => {},
  disconnectServer: async () => {},
  getSnapshot: async () => structuredClone(snapshot),
  getSettings: async () => ({ ...settings }),
  saveSettings: async (s: Settings) => { settings = { ...s } },
  getLogs: async () => [...logs],
  respondHostKey: async (_promptId: string, _trust: boolean) => {},
}

// 浏览器里没有 Tauri 事件总线,listen 返回 noop 取消函数即可
export const mockListen = <T>(_event: string, _cb: (e: { payload: T }) => void) =>
  Promise.resolve(() => {})

// 标记类型用,避免未使用告警
export type { HostKeyPrompt }
