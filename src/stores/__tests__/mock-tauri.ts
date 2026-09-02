import { vi } from 'vitest'
import type { TunnelEvent } from '../../types'

type TunnelHandler = (ev: TunnelEvent) => void

// listenMock 捕获的 tunnel-event 处理器,emitTunnel 经它把事件注入 store
let tunnelHandler: TunnelHandler | null = null

export const invokeMock = vi.fn()

// 与生产 listen 同签名(返回 unlisten 的 Promise),测试文件内联 vi.mock 时直接复用
export function listenMock(name: string, handler: (e: { payload: unknown }) => void) {
  if (name === 'tunnel-event') tunnelHandler = (ev) => handler({ payload: ev })
  return Promise.resolve(() => {})
}

export function emitTunnel(ev: TunnelEvent) {
  tunnelHandler?.(ev)
}
