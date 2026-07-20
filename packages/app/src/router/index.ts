// 路由配置（Phase 2 重构版）
//
// Phase 2 变更：
//   - 从扁平路由改为嵌套路由：/projects/:projectId/chat
//   - 兼容旧 /chat 路由（redirect 到默认项目）
//   - 新增 ProjectSettings 路由
//
// 使用 HTML5 History 模式（Tauri 内嵌 WebView 支持 history 模式）

import { createRouter, createWebHistory } from "vue-router";
import type { RouteRecordRaw } from "vue-router";

// 路由懒加载：按需加载页面组件，提升首屏性能
const ChatPage = () => import("../pages/ChatPage.vue");
const AgentManagerPage = () => import("../pages/AgentManagerPage.vue");
const TemplateManagerPage = () => import("../pages/TemplateManagerPage.vue");
const SettingsPage = () => import("../pages/SettingsPage.vue");
const SettingsGeneral = () => import("../pages/settings/SettingsGeneral.vue");
const SettingsAppearance = () => import("../pages/settings/SettingsAppearance.vue");
const SettingsKeyboard = () => import("../pages/settings/SettingsKeyboard.vue");
const SettingsStorage = () => import("../pages/settings/SettingsStorage.vue");
const SettingsAbout = () => import("../pages/settings/SettingsAbout.vue");
const ProjectManagerPage = () => import("../pages/ProjectManagerPage.vue");

// 路由表
const routes: RouteRecordRaw[] = [
  {
    // 首页重定向到默认项目的聊天页
    path: "/",
    redirect: { name: "ProjectChat", params: { projectId: "default" } },
  },
  {
    // Phase 2: 嵌套路由 — 项目维度
    path: "/projects/:projectId",
    children: [
      {
        path: "chat",
        name: "ProjectChat",
        component: ChatPage,
        meta: { title: "聊天" },
      },
      {
        path: "chat/:conversationId",
        name: "ProjectChatConversation",
        component: ChatPage,
        meta: { title: "聊天" },
      },
      {
        path: "settings",
        name: "ProjectSettings",
        component: ProjectManagerPage,
        meta: { title: "项目管理" },
      },
    ],
  },
  // 兼容旧 /chat 路由（redirect 到默认项目）
  {
    path: "/chat",
    redirect: { name: "ProjectChat", params: { projectId: "default" } },
  },
  // 兼容旧 /chat/:conversationId 路由（保留会话 ID）
  {
    path: "/chat/:conversationId",
    redirect: (to) => ({
      name: "ProjectChatConversation",
      params: {
        projectId: "default",
        conversationId: to.params.conversationId,
      },
    }),
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
  {
    path: "/settings",
    component: SettingsPage,
    meta: { title: "设置" },
    children: [
      { path: "", redirect: { name: "SettingsGeneral" } },
      { path: "general", name: "SettingsGeneral", component: SettingsGeneral, meta: { title: "通用" } },
      { path: "appearance", name: "SettingsAppearance", component: SettingsAppearance, meta: { title: "外观" } },
      { path: "keyboard", name: "SettingsKeyboard", component: SettingsKeyboard, meta: { title: "快捷键" } },
      { path: "storage", name: "SettingsStorage", component: SettingsStorage, meta: { title: "存储" } },
      { path: "about", name: "SettingsAbout", component: SettingsAbout, meta: { title: "关于" } },
    ],
  },
  // 通配兜底：未匹配到的路径重定向到默认项目聊天页
  {
    path: "/:pathMatch(.*)*",
    redirect: { name: "ProjectChat", params: { projectId: "default" } },
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
