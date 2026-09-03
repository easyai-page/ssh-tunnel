<script setup lang="ts">
import { onMounted, reactive } from 'vue'
import { ElMessage } from 'element-plus'
import { Check } from '@element-plus/icons-vue'
import { api } from '../api'
import { useTheme, type ThemeMode } from '../composables/useTheme'
import type { Settings } from '../types'

const form = reactive<Settings>({ auto_reconnect: true, minimize_to_tray: true, launch_at_login: false })
const { mode, setMode } = useTheme()

onMounted(async () => {
  Object.assign(form, await api.getSettings())
})

async function save() {
  try {
    await api.saveSettings({ ...form })
    ElMessage.success('已保存')
  } catch {
    // 失败提示已由 api 层弹出,这里不再重复
  }
}
</script>

<template>
  <div class="settings-view">
    <div class="settings-card">
      <div class="group">
        <div class="group-title">外观</div>
        <!-- 主题立即生效,不参与下方保存流程 -->
        <el-form label-width="100px">
          <el-form-item label="主题">
            <el-radio-group :model-value="mode" @change="setMode($event as ThemeMode)">
              <el-radio-button value="light">浅色</el-radio-button>
              <el-radio-button value="dark">深色</el-radio-button>
              <el-radio-button value="system">跟随系统</el-radio-button>
            </el-radio-group>
          </el-form-item>
        </el-form>
      </div>

      <div class="group">
        <div class="group-title">行为</div>
        <el-form label-width="180px">
          <el-form-item label="断线自动重连">
            <el-switch v-model="form.auto_reconnect" />
          </el-form-item>
          <el-form-item label="关闭窗口时最小化到托盘">
            <el-switch v-model="form.minimize_to_tray" />
          </el-form-item>
          <el-form-item label="开机自启动">
            <el-switch v-model="form.launch_at_login" />
          </el-form-item>
          <el-form-item>
            <el-button type="primary" :icon="Check" @click="save">保存</el-button>
          </el-form-item>
        </el-form>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-view {
  height: 100%;
  overflow: auto;
  padding: 16px;
  box-sizing: border-box;
}

.settings-card {
  max-width: 560px;
  background: var(--app-panel-bg);
  border: 1px solid var(--app-border);
  border-radius: 8px;
  box-shadow: var(--app-panel-shadow);
  padding: 20px 24px;
}

.group + .group {
  margin-top: 8px;
  padding-top: 16px;
  border-top: 1px solid var(--app-border);
}
.group-title { font-weight: 600; font-size: 15px; margin-bottom: 12px; }
</style>
