// 路由配置
//
// 使用 HTML5 History 模式（Tauri 内嵌 WebView 支持 history 模式）

import { createRouter, createWebHistory } from "vue-router";
import type { RouteRecordRaw } from "vue-router";

// 路由表
const routes: RouteRecordRaw[] = [
  {
    path: "/",
    component: () => import("../components/layout/AppLayout.vue"),
    children: [
      {
        path: "",
        name: "Home",
        component: () => import("../pages/ChatPage.vue"),
      },
      {
        path: "projects",
        name: "Projects",
        component: () => import("../pages/ProjectList.vue"),
        meta: { title: "项目" },
      },
      {
        path: "settings",
        component: () => import("../pages/settings/SettingsLayout.vue"),
        children: [
          {
            path: "",
            redirect: { name: "SettingsGeneral" },
          },
          {
            path: "general",
            name: "SettingsGeneral",
            component: () => import("../pages/settings/GeneralSettings.vue"),
            meta: { title: "通用设置" },
          },
          {
            path: "agents",
            name: "SettingsAgents",
            component: () => import("../pages/settings/AgentSettings.vue"),
            meta: { title: "智能体" },
          },
          {
            path: "mcp",
            name: "SettingsMcp",
            component: () => import("../pages/settings/McpSettings.vue"),
            meta: { title: "工具集" },
          },
          {
            path: "kb",
            name: "SettingsKb",
            component: () => import("../pages/settings/KbSettings.vue"),
            meta: { title: "全局知识库" },
          },
          {
            path: "logs",
            name: "SettingsLogs",
            component: () => import("../pages/settings/LogSettings.vue"),
            meta: { title: "运行日志" },
          },
        ],
      },
    ],
  },
  // 通配兜底
  {
    path: "/:pathMatch(.*)*",
    redirect: "/",
  },
];

// 创建 router 实例
const router = createRouter({
  history: createWebHistory(),
  routes,
});

// 全局后置钩子：更新页面标题
router.afterEach((to) => {
  const title = (to.meta?.title as string | undefined) ?? "IcePaw";
  document.title = `${title} | IcePaw`;
});

export default router;
