import { vi } from 'vitest'
import { invokeMock, emitTunnel, listenMock } from './mock-tauri'

// vi.mock 必须留在测试文件顶层:vitest 只可靠提升本文件内的调用,
// 工厂惰性执行时 mock-tauri 的共享状态(invokeMock/listenMock)已随导入就绪
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))

import { setActivePinia, createPinia } from 'pinia'
import { useForwardsStore, bindForwardsEvents } from '../forwards'
import type { Forward } from '../../types'

function fwd(over: Partial<Forward> = {}): Forward {
  return {
    id: 'f1', server_id: 's1', name: 'mysql', kind: 'local',
    bind_addr: '127.0.0.1', bind_port: 3306,
    target_host: 'db', target_port: 3306, auto_start: false,
    ...over,
  }
}

describe('forwards store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    invokeMock.mockReset()
  })

  it('load 拉取转发与快照', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_forwards') return Promise.resolve([fwd()])
      if (cmd === 'get_snapshot') return Promise.resolve({ servers: {}, forwards: { f1: { status: 'running', error: null } } })
      return Promise.resolve(null)
    })
    const store = useForwardsStore()
    await store.load()
    expect(store.forwards).toHaveLength(1)
    expect(store.forwardStatus['f1'].status).toBe('running')
  })

  it('forwardsOf 按服务器过滤', async () => {
    invokeMock.mockResolvedValue([fwd(), fwd({ id: 'f2', server_id: 's2' })])
    const store = useForwardsStore()
    await store.load()
    expect(store.forwardsOf('s1').map((f) => f.id)).toEqual(['f1'])
  })

  it('toggle 停启判断:running → stop,其余 → start', async () => {
    invokeMock.mockResolvedValue([fwd()])
    const store = useForwardsStore()
    await store.load()
    await store.toggle('f1')
    expect(invokeMock).toHaveBeenLastCalledWith('start_forward', { id: 'f1' })

    store.forwardStatus['f1'] = { status: 'running', error: null }
    await store.toggle('f1')
    expect(invokeMock).toHaveBeenLastCalledWith('stop_forward', { id: 'f1' })
  })

  it('tunnel-event 更新状态表', async () => {
    invokeMock.mockResolvedValue([])
    const store = useForwardsStore()
    await store.load()
    bindForwardsEvents()
    emitTunnel({ type: 'forward_status', forward_id: 'f1', server_id: 's1', status: 'error', error: '本地端口 3306 被占用' })
    expect(store.forwardStatus['f1']).toEqual({ status: 'error', error: '本地端口 3306 被占用' })
  })
})
