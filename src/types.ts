// 与 Rust serde 输出一一对应的共享数据模型（snake_case 字段保持一致，避免序列化错位）
export type AuthMethod =
  | { type: 'password' }
  | { type: 'key_file'; path: string }
  | { type: 'key_data' }

export interface Server {
  id: string
  name: string
  host: string
  port: number
  username: string
  auth: AuthMethod
}

export type ForwardKind = 'local' | 'remote' | 'dynamic'

export interface Forward {
  id: string
  server_id: string
  name: string
  kind: ForwardKind
  bind_addr: string
  bind_port: number
  target_host: string | null
  target_port: number | null
  auto_start: boolean
}

export interface Settings {
  auto_reconnect: boolean
  minimize_to_tray: boolean
  launch_at_login: boolean
}

export type ServerStatus = 'disconnected' | 'connecting' | 'connected' | 'reconnecting' | 'error'
export type ForwardStatus = 'stopped' | 'starting' | 'running' | 'error'

export interface StatusEntry<T> {
  status: T
  error: string | null
}

export interface StatusSnapshot {
  servers: Record<string, StatusEntry<ServerStatus>>
  forwards: Record<string, StatusEntry<ForwardStatus>>
}

export type TunnelEvent =
  | { type: 'server_status'; server_id: string; status: ServerStatus; error: string | null }
  | { type: 'forward_status'; forward_id: string; server_id: string; status: ForwardStatus; error: string | null }

export interface LogEntry {
  timestamp: string
  level: string
  message: string
}

export interface HostKeyPrompt {
  prompt_id: string
  host: string
  port: number
  fingerprint: string
  is_mismatch: boolean
}

// 敏感值（密码/密钥内容/passphrase）只经此输入一次性传给后端写入钥匙串，绝不落进 store 状态
export interface UpsertServerInput {
  server: Server
  password?: string | null
  key_data?: string | null
  key_passphrase?: string | null
}
