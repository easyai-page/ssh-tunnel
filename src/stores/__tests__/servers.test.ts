import { vi } from 'vitest'
import { invokeMock, listenMock } from './mock-tauri'

// vi.mock 必须留在测试文件顶层:vitest 只可靠提升本文件内的调用,
// 工厂惰性执行时 mock-tauri 的共享状态(invokeMock/listenMock)已随导入就绪
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))

import { setActivePinia, createPinia } from 'pinia'
import { useServersStore } from '../servers'
import type { Server } from '../../types'

function srv(over: Partial<Server> = {}): Server {
  return { id: 's1', name: 'db', host: '10.0.0.2', port: 22, username: 'u', auth: { type: 'password' }, ...over }
}

describe('servers store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    invokeMock.mockReset()
  })

  it('load 后自动选中第一台', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_servers') return Promise.resolve([srv()])
      if (cmd === 'get_snapshot') return Promise.resolve({ servers: {}, forwards: {} })
      return Promise.resolve(null)
    })
    const store = useServersStore()
    await store.load()
    expect(store.selectedId).toBe('s1')
  })

  it('remove 后清空选择并刷新列表', async () => {
    invokeMock.mockResolvedValue([srv()])
    const store = useServersStore()
    await store.load()
    invokeMock.mockResolvedValue([])
    await store.remove('s1')
    expect(invokeMock).toHaveBeenCalledWith('delete_server', { id: 's1' })
    expect(store.servers).toEqual([])
    expect(store.selectedId).toBeNull()
  })
})
