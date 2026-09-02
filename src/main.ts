import { createApp } from 'vue'
import { createPinia } from 'pinia'
import ElementPlus from 'element-plus'
import zhCn from 'element-plus/es/locale/lang/zh-cn'
import 'element-plus/dist/index.css'
import App from './App.vue'

// 无路由：App.vue 用 tab 切换三个视图，Pinia + Element Plus(中文文案)在此统一挂载
createApp(App).use(createPinia()).use(ElementPlus, { locale: zhCn }).mount('#app')
