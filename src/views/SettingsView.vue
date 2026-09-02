<script setup lang="ts">
import { onMounted, reactive } from 'vue'
import { ElMessage } from 'element-plus'
import { api } from '../api'
import type { Settings } from '../types'

const form = reactive<Settings>({ auto_reconnect: true, minimize_to_tray: true, launch_at_login: false })

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
    <el-form label-width="180px" style="max-width: 480px">
      <el-form-item label="断线自动重连">
        <el-switch v-model="form.auto_reconnect" />
      </el-form-item>
      <el-form-item label="关闭窗口时最小化到托盘">
        <el-switch v-model="form.minimize_to_tray" />
      </el-form-item>
      <el-form-item label="开机自启动">
        <el-switch v-model="form.launch_at_login" />
      </el-form-item>
      <el-form-item><el-button type="primary" @click="save">保存</el-button></el-form-item>
    </el-form>
  </div>
</template>
