// 应用入口：创建 Vue 应用实例、依次挂载 router 与 pinia，然后挂载到 DOM
// 注意挂载顺序：先 router 再 pinia 是一个稳妥的写法（router 通常不依赖 pinia，
// 但反过来 pinia 也通常不依赖 router；只要都在 mount 之前完成 use() 即可）。
import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import pinia from "./stores";

const app = createApp(App);
app.use(router);
app.use(pinia);
app.mount("#app");
