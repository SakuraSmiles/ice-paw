// 应用入口：创建 Vue 应用实例，依次挂载 pinia 与 router，然后挂载到 DOM
// 注意挂载顺序：pinia 先于 router。
// 这样 router 中的路由守卫（例如未来会用到的导航守卫）若调用 store，不会因 pinia 未挂载而报错
import { createApp } from "vue";
import App from "./App.vue";
import pinia from "./stores";
import router from "./router";

const app = createApp(App);
app.use(pinia);
app.use(router);
app.mount("#app");
