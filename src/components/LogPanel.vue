<script setup lang="ts">
import { onMounted } from 'vue'
import { useLogsStore } from '../stores/logs'

const store = useLogsStore()
onMounted(() => store.load())
</script>

<template>
  <div class="log-panel">
    <div class="log-toolbar">
      <el-button size="small" @click="store.load()">刷新</el-button>
      <el-button size="small" @click="store.clear()">清空显示</el-button>
    </div>
    <div class="log-list">
      <div v-for="(e, i) in store.entries" :key="i" :class="['log-line', e.level.toLowerCase()]">
        <span class="ts">{{ e.timestamp }}</span> <span class="lv">{{ e.level }}</span> {{ e.message }}
      </div>
      <el-empty v-if="store.entries.length === 0" description="暂无日志" :image-size="60" />
    </div>
  </div>
</template>

<style scoped>
.log-panel { display: flex; flex-direction: column; height: 100%; }
.log-list { flex: 1; overflow: auto; font-family: monospace; font-size: 12px; }
.log-line .ts { color: #999; margin-right: 6px; }
.log-line.error .lv { color: #f56c6c; }
.log-line.warn .lv { color: #e6a23c; }
</style>
