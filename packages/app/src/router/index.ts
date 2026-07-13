// 路由配置
// 使用 HTML5 History 模式（Tauri 内嵌 WebView 支持 history 模式）
// 仅保留 /chat 与 /agents 两个主路由，所有页面级组件均按需懒加载
import { createRouter, createWebHistory } from "vue-router";
import type { RouteRecordRaw } from "vue-router";

// 路由懒加载：按需加载页面组件，提升首屏性能
const ChatPage = () => import("../pages/ChatPage.vue");
const AgentManagerPage = () => import("../pages/AgentManagerPage.vue");
const TemplateManagerPage = () => import("../pages/TemplateManagerPage.vue");

// 路由表
const routes: RouteRecordRaw[] = [
  {
    // 首页重定向到聊天页
    path: "/",
    redirect: { name: "Chat" },
  },
  {
    // 聊天主页面
    path: "/chat",
    name: "Chat",
    component: ChatPage,
    meta: { title: "聊天" },
  },
  {
    // Agent 管理页
    path: "/agents",
    name: "AgentManager",
    component: AgentManagerPage,
    meta: { title: "Agent 管理" },
  },
  {
    // 模板管理页
    path: "/templates",
    name: "TemplateManager",
    component: TemplateManagerPage,
    meta: { title: "模板管理" },
  },
  // 通配兜底：未匹配到的路径重定向到聊天页
  {
    path: "/:pathMatch(.*)*",
    redirect: { name: "Chat" },
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
