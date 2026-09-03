<script setup lang="ts">
import { onMounted } from 'vue'
import { Delete, Refresh } from '@element-plus/icons-vue'
import { useLogsStore } from '../stores/logs'

const store = useLogsStore()
onMounted(() => store.load())
</script>

<template>
  <div class="log-panel">
    <div class="log-toolbar">
      <span class="log-title">运行日志</span>
      <div class="log-actions">
        <el-button size="small" :icon="Refresh" @click="store.load()">刷新</el-button>
        <el-button size="small" :icon="Delete" @click="store.clear()">清空显示</el-button>
      </div>
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
.log-panel {
  height: 100%;
  padding: 16px;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
}

.log-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 12px;
}
.log-title { font-weight: 600; font-size: 15px; }

.log-list {
  flex: 1;
  overflow: auto;
  background: var(--app-panel-bg);
  border: 1px solid var(--app-border);
  border-radius: 8px;
  box-shadow: var(--app-panel-shadow);
  padding: 12px 16px;
  font-family: var(--app-font-mono);
  font-size: 12px;
  line-height: 1.7;
}

.log-line { padding: 1px 4px; border-radius: 4px; }
/* hover 行底色,便于逐行追踪 */
.log-line:hover { background: var(--app-hover-bg); }
.log-line .ts { color: var(--app-text-secondary); margin-right: 6px; }
.log-line .lv { font-weight: 600; }
.log-line.error .lv { color: var(--app-danger); }
.log-line.warn .lv { color: var(--app-warning); }
.log-line.info .lv { color: var(--el-color-primary); }
</style>
