<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import type { Forward } from '../types'

const props = defineProps<{ modelValue: boolean; forward: Forward }>()
const emit = defineEmits<{ 'update:modelValue': [boolean]; submit: [Forward] }>()

// 本地副本,取消不落盘
const form = reactive<Forward>({ ...props.forward })
watch(() => props.forward, (f) => Object.assign(form, f))

const needTarget = computed(() => form.kind !== 'dynamic')
const bindLabel = computed(() => (form.kind === 'remote' ? '远程监听' : '本地监听'))

function submit() {
  if (!form.name || !form.bind_port) return
  if (needTarget.value && (!form.target_host || !form.target_port)) return
  if (form.kind === 'dynamic') {
    // SOCKS 无固定目标,清空残留避免脏数据进配置
    form.target_host = null
    form.target_port = null
  }
  emit('submit', { ...form })
  emit('update:modelValue', false)
}
</script>

<template>
  <el-dialog :model-value="modelValue" :title="form.id ? '编辑转发' : '添加转发'" width="480px"
    @update:model-value="emit('update:modelValue', $event)">
    <el-form label-width="90px" @submit.prevent="submit">
      <el-form-item label="名称" required>
        <el-input v-model="form.name" placeholder="例如:测试库 MySQL" />
      </el-form-item>
      <el-form-item label="类型">
        <el-radio-group v-model="form.kind">
          <el-radio-button value="local">本地 -L</el-radio-button>
          <el-radio-button value="remote">远程 -R</el-radio-button>
          <el-radio-button value="dynamic">SOCKS -D</el-radio-button>
        </el-radio-group>
      </el-form-item>
      <el-form-item :label="bindLabel" required>
        <el-input v-model="form.bind_addr" style="width: 60%" placeholder="127.0.0.1" />
        <el-input-number v-model="form.bind_port" :min="1" :max="65535" style="width: 38%; margin-left: 2%" />
      </el-form-item>
      <el-form-item v-if="needTarget" label="目标地址" required>
        <el-input v-model="form.target_host" style="width: 60%" placeholder="目标主机" />
        <el-input-number v-model="form.target_port" :min="1" :max="65535" style="width: 38%; margin-left: 2%" />
      </el-form-item>
      <el-form-item>
        <el-checkbox v-model="form.auto_start">应用启动时自动开启</el-checkbox>
      </el-form-item>
    </el-form>
    <template #footer>
      <el-button @click="emit('update:modelValue', false)">取消</el-button>
      <el-button type="primary" @click="submit">保存</el-button>
    </template>
  </el-dialog>
</template>
