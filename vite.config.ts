/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { configDefaults } from 'vitest/config'

export default defineConfig({
  plugins: [vue()],
  server: { port: 1420, strictPort: true },
  clearScreen: false,
  test: {
    environment: 'jsdom',
    globals: true,
    // 排除仓库内的 git worktree 副本（.claude/worktrees/）：从主仓库根目录扫描时会
    // 重复命中 worktree 里的同名测试，双份 node_modules 导致 Vue 实例分裂的假失败
    exclude: [...configDefaults.exclude, '.claude/**'],
  },
})
