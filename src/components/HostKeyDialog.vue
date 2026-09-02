<script setup lang="ts">
import { computed } from 'vue'
import { useServersStore } from '../stores/servers'

const store = useServersStore()
const prompt = computed(() => store.hostKeyPrompt)
</script>

<template>
  <el-dialog :model-value="!!prompt" :title="prompt?.is_mismatch ? '警告:主机密钥已变更' : '信任新的主机密钥?'"
    width="480px" :close-on-click-modal="false" :show-close="false">
    <template v-if="prompt">
      <p><b>{{ prompt.host }}:{{ prompt.port }}</b></p>
      <p>指纹:<code>{{ prompt.fingerprint }}</code></p>
      <el-alert v-if="prompt.is_mismatch" type="error" :closable="false"
        title="与已记录的密钥不符,连接可能被劫持。确认服务器重装/换 key 后再信任。" />
      <el-alert v-else type="warning" :closable="false" title="首次连接该主机,信任后将记录此密钥。" />
    </template>
    <template #footer>
      <el-button @click="store.respondHostKey(false)">拒绝</el-button>
      <el-button :type="prompt?.is_mismatch ? 'danger' : 'primary'" @click="store.respondHostKey(true)">信任并继续</el-button>
    </template>
  </el-dialog>
</template>
