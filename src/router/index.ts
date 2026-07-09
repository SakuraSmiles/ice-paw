// 路由配置
// 使用 HTML5 History 模式（Tauri 内嵌 WebView 支持 history 模式）
import { createRouter, createWebHistory } from "vue-router";
import type { RouteRecordRaw } from "vue-router";

// 路由懒加载：按需加载页面组件，提升首屏性能
const HomePage = () => import("../pages/HomePage.vue");
const CounterPage = () => import("../pages/CounterPage.vue");
const TestRouterPage = () => import("../pages/TestRouterPage.vue");
const TestPiniaPage = () => import("../pages/TestPiniaPage.vue");
const TestSqlPage = () => import("../pages/TestSqlPage.vue");
const TestKeychainPage = () => import("../pages/TestKeychainPage.vue");

// 路由表
const routes: RouteRecordRaw[] = [
  {
    path: "/",
    name: "Home",
    component: HomePage,
    meta: { title: "首页" },
  },
  {
    path: "/counter",
    name: "Counter",
    component: CounterPage,
    meta: { title: "计数器" },
  },
  {
    // 基础路径：/test-router（无参数）
    path: "/test-router",
    name: "TestRouter",
    component: TestRouterPage,
    meta: { title: "路由测试" },
  },
  {
    // 动态路由参数：/test-router/:id
    path: "/test-router/:id",
    name: "TestRouterWithId",
    component: TestRouterPage,
    meta: { title: "路由测试 - 动态参数" },
  },
  {
    // Pinia 状态管理测试页
    path: "/test-pinia",
    name: "TestPinia",
    component: TestPiniaPage,
    meta: { title: "Pinia 测试" },
  },
  {
    // SQLite 数据库测试页（依赖 tauri-plugin-sql，仅在原生窗口中可用）
    path: "/test-sql",
    name: "TestSql",
    component: TestSqlPage,
    meta: { title: "SQLite 测试" },
  },
  {
    // Keychain 加密存储测试页（依赖 tauri-plugin-store，仅在原生窗口中可用）
    path: "/test-keychain",
    name: "TestKeychain",
    component: TestKeychainPage,
    meta: { title: "Keychain 测试" },
  },
  // 通配兜底：未匹配到的路径重定向到首页
  {
    path: "/:pathMatch(.*)*",
    redirect: { name: "Home" },
  },
];

// 创建 router 实例
const router = createRouter({
  history: createWebHistory(),
  routes,
  // 路由切换时滚动到顶部
  scrollBehavior(_to, _from, savedPosition) {
    if (savedPosition) {
      return savedPosition;
    }
    return { top: 0 };
  },
});

// 全局后置钩子：更新页面标题
router.afterEach((to) => {
  const title = (to.meta?.title as string | undefined) ?? "IcePaw";
  document.title = `${title} | IcePaw`;
});

export default router;