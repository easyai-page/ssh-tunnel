import { ref, watch } from 'vue'

export type ThemeMode = 'light' | 'dark' | 'system'

const STORAGE_KEY = 'app-theme'

// 模块级单例：整个应用共享一份主题状态，避免各组件各自读 localStorage 不一致
const mode = ref<ThemeMode>((localStorage.getItem(STORAGE_KEY) as ThemeMode) || 'light')
const isDark = ref(false)

const media = window.matchMedia('(prefers-color-scheme: dark)')

function apply() {
  isDark.value = mode.value === 'dark' || (mode.value === 'system' && media.matches)
  document.documentElement.classList.toggle('dark', isDark.value)
}

watch(mode, (m) => {
  localStorage.setItem(STORAGE_KEY, m)
  apply()
})

// system 模式下跟随系统切换实时变化
media.addEventListener('change', () => {
  if (mode.value === 'system') apply()
})

apply()

export function useTheme() {
  return {
    mode,
    isDark,
    setMode(m: ThemeMode) {
      mode.value = m
    },
  }
}
