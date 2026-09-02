<script setup lang="ts">
import { reactive, ref, watch } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import type { Server, UpsertServerInput } from '../types'

const props = defineProps<{ modelValue: boolean; server: Server | null }>()
const emit = defineEmits<{ 'update:modelValue': [boolean]; submit: [UpsertServerInput] }>()

const form = reactive({ name: '', host: '', port: 22, username: '' })
const authType = ref<'password' | 'key_file' | 'key_data'>('password')
const password = ref('')
const keyPath = ref('')
const keyData = ref('')
const keyPassphrase = ref('')

watch(
  () => props.server,
  (s) => {
    if (!s) {
      Object.assign(form, { name: '', host: '', port: 22, username: '' })
      authType.value = 'password'
      password.value = ''
      keyPath.value = ''
      keyData.value = ''
      keyPassphrase.value = ''
      return
    }
    Object.assign(form, { name: s.name, host: s.host, port: s.port, username: s.username })
    authType.value = s.auth.type
    if (s.auth.type === 'key_file') keyPath.value = s.auth.path
    // 敏感值不回填:留空表示保持不变
    password.value = ''
    keyData.value = ''
    keyPassphrase.value = ''
  },
  { immediate: true },
)

// 打开系统文件选择框挑选私钥;取消时返回 null,不动现有值
async function pickKeyFile() {
  const selected = await open({ multiple: false, directory: false })
  if (typeof selected === 'string') keyPath.value = selected
}

function submit() {
  if (!form.name || !form.host || !form.username) return
  // 新建且选密码认证时密码必填:留空保存后连接必然失败(「未保存密码」),拦在提交前
  if (!props.server && authType.value === 'password' && !password.value) return
  const server: Server = {
    id: props.server?.id ?? '',
    name: form.name,
    host: form.host,
    port: form.port,
    username: form.username,
    auth:
      authType.value === 'password'
        ? { type: 'password' }
        : authType.value === 'key_file'
          ? { type: 'key_file', path: keyPath.value }
          : { type: 'key_data' },
  }
  emit('submit', {
    server,
    // 空字符串视为未修改,避免把钥匙串里的值清空
    password: password.value || null,
    key_data: keyData.value || null,
    key_passphrase: keyPassphrase.value || null,
  })
  // 不在此处关对话框:保存是否成功只有父组件知道,由父组件在成功后关闭
  // 敏感值用后即清,不留组件状态;保存失败重试需重新输入,是有意的安全取舍
  password.value = ''
  keyData.value = ''
  keyPassphrase.value = ''
}
</script>

<template>
  <el-dialog :model-value="modelValue" :title="server ? '编辑服务器' : '添加服务器'" width="520px"
    @update:model-value="emit('update:modelValue', $event)">
    <el-form label-width="80px">
      <el-form-item label="名称" required><el-input v-model="form.name" /></el-form-item>
      <el-form-item label="主机" required>
        <el-input v-model="form.host" style="width: 65%" placeholder="域名或 IP" />
        <el-input-number v-model="form.port" :min="1" :max="65535" style="width: 33%; margin-left: 2%" />
      </el-form-item>
      <el-form-item label="用户名" required><el-input v-model="form.username" /></el-form-item>
      <el-form-item label="认证">
        <el-tabs v-model="authType" style="width: 100%">
          <el-tab-pane label="密码" name="password">
            <el-input v-model="password" type="password" show-password
              :placeholder="server ? '留空保持不变' : '登录密码'" />
          </el-tab-pane>
          <el-tab-pane label="密钥文件" name="key_file">
            <el-input v-model="keyPath" placeholder="如 ~/.ssh/id_ed25519" style="margin-bottom: 8px">
              <template #append><el-button @click="pickKeyFile">浏览</el-button></template>
            </el-input>
            <el-input v-model="keyPassphrase" type="password" show-password placeholder="密钥密码(如有,留空保持不变)" />
          </el-tab-pane>
          <el-tab-pane label="粘贴密钥" name="key_data">
            <el-input v-model="keyData" type="textarea" :rows="6"
              :placeholder="server ? '留空保持不变' : '粘贴 -----BEGIN OPENSSH PRIVATE KEY----- 完整内容'" style="margin-bottom: 8px" />
            <el-input v-model="keyPassphrase" type="password" show-password placeholder="密钥密码(如有,留空保持不变)" />
          </el-tab-pane>
        </el-tabs>
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="emit('update:modelValue', false)">取消</el-button>
      <el-button type="primary" @click="submit">保存</el-button>
    </template>
  </el-dialog>
</template>
