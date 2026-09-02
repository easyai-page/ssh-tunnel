<script setup lang="ts">
import { onMounted, ref } from 'vue'
import MainView from './views/MainView.vue'
import SettingsView from './views/SettingsView.vue'
import LogPanel from './components/LogPanel.vue'
import HostKeyDialog from './components/HostKeyDialog.vue'
import { onNavigate } from './api'
import { bindServersEvents, useServersStore } from './stores/servers'
import { bindForwardsEvents, useForwardsStore } from './stores/forwards'
import { bindLogsEvents } from './stores/logs'

const tab = ref('main')
const mainView = ref<InstanceType<typeof MainView>>()

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
  <el-container class="app-shell">
    <el-header class="app-header" height="48px">
      <span class="title">SSH Tunnel</span>
      <el-radio-group v-model="tab" size="small">
        <el-radio-button value="main">转发</el-radio-button>
        <el-radio-button value="logs">日志</el-radio-button>
        <el-radio-button value="settings">设置</el-radio-button>
      </el-radio-group>
    </el-header>
    <el-main class="app-main">
      <MainView v-show="tab === 'main'" ref="mainView" />
      <LogPanel v-show="tab === 'logs'" />
      <SettingsView v-show="tab === 'settings'" />
    </el-main>
    <HostKeyDialog />
  </el-container>
</template>

<style>
html, body, #app { height: 100%; margin: 0; }
.app-shell { height: 100%; }
.app-header { display: flex; align-items: center; gap: 16px; border-bottom: 1px solid #e4e7ed; }
.app-header .title { font-weight: 700; }
.app-main { padding: 0; }
</style>
