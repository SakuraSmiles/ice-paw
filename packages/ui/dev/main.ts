// IcePaw UI 预览站入口
import { createApp } from 'vue'
import App from './App.vue'
import '../src/styles/index.css' // 直接引入源码样式，方便开发

const app = createApp(App)
app.mount('#app')
