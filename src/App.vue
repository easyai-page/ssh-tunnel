<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Connection, Document, Moon, Setting, Sunny, Switch } from '@element-plus/icons-vue'
import MainView from './views/MainView.vue'
import SettingsView from './views/SettingsView.vue'
import LogPanel from './components/LogPanel.vue'
import HostKeyDialog from './components/HostKeyDialog.vue'
import { onNavigate } from './api'
import { bindServersEvents, useServersStore } from './stores/servers'
import { bindForwardsEvents, useForwardsStore } from './stores/forwards'
import { bindLogsEvents } from './stores/logs'
import { useTheme } from './composables/useTheme'

const tab = ref('main')
const mainView = ref<InstanceType<typeof MainView>>()
const { isDark, setMode } = useTheme()

const NAV_ITEMS = [
  { key: 'main', label: '转发', icon: Switch },
  { key: 'logs', label: '日志', icon: Document },
  { key: 'settings', label: '设置', icon: Setting },
]

onMounted(async () => {
  const servers = useServersStore()
  const forwards = useForwardsStore()
  // 事件绑定与初始加载集中在壳层:store 保持纯粹,不自带副作用
  bindServersEvents()
  bindForwardsEvents()
  bindLogsEvents()
  await Promise.all([servers.load(), forwards.load()])
  onNavigate((nav) => {
    // 托盘点击「添加转发」时切回主视图并打开对话框
    tab.value = 'main'
    if (nav.view === 'add-forward') mainView.value?.openAddForward(nav.server_id)
  })
})
</script>

<template>
  <div class="app-shell">
    <aside class="app-nav">
      <div class="brand">
        <el-icon :size="22" class="brand-icon"><Connection /></el-icon>
        <div class="brand-text">
          <span class="brand-name">SSH Tunnel</span>
          <span class="brand-sub">端口转发管理</span>
        </div>
      </div>

      <nav class="nav-menu">
        <div v-for="item in NAV_ITEMS" :key="item.key"
          :class="['nav-item', { active: tab === item.key }]"
          :aria-current="tab === item.key ? 'page' : undefined"
          role="button" tabindex="0"
          @click="tab = item.key"
          @keydown.enter="tab = item.key">
          <el-icon :size="17"><component :is="item.icon" /></el-icon>
          <span>{{ item.label }}</span>
        </div>
      </nav>

      <div class="nav-footer">
        <div class="nav-item" role="button" tabindex="0"
          @click="setMode(isDark ? 'light' : 'dark')"
          @keydown.enter="setMode(isDark ? 'light' : 'dark')">
          <el-icon :size="17"><component :is="isDark ? Sunny : Moon" /></el-icon>
          <span>{{ isDark ? '浅色模式' : '深色模式' }}</span>
        </div>
      </div>
    </aside>

    <main class="app-content">
      <MainView v-show="tab === 'main'" ref="mainView" />
      <LogPanel v-show="tab === 'logs'" />
      <SettingsView v-show="tab === 'settings'" />
    </main>
    <HostKeyDialog />
  </div>
</template>

<style scoped>
.app-shell { display: flex; height: 100%; }

.app-nav {
  width: 200px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  background: var(--app-panel-bg);
  border-right: 1px solid var(--app-border);
  padding: 16px 12px;
  box-sizing: border-box;
}

.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 8px 16px;
  border-bottom: 1px solid var(--app-border);
  margin-bottom: 12px;
}
.brand-icon { color: var(--el-color-primary); flex-shrink: 0; }
.brand-text { display: flex; flex-direction: column; }
.brand-name { font-weight: 700; font-size: 14px; }
.brand-sub { font-size: 12px; color: var(--app-text-secondary); }

.nav-menu { flex: 1; display: flex; flex-direction: column; gap: 4px; }

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border-radius: 8px;
  font-size: 14px;
  color: var(--app-text);
  transition: background-color 180ms ease, color 180ms ease;
}
.nav-item:hover { background: var(--app-hover-bg); }
.nav-item.active {
  background: var(--app-active-bg);
  color: var(--el-color-primary);
  font-weight: 600;
}
.nav-item:focus-visible {
  outline: 2px solid var(--el-color-primary);
  outline-offset: 2px;
}

.nav-footer { border-top: 1px solid var(--app-border); padding-top: 8px; }

.app-content { flex: 1; min-width: 0; background: var(--app-bg); }
</style>
