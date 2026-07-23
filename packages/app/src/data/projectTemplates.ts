// 项目推荐模板（P0-7 共用数据源）
//
// 职责：
//   - 提供项目管理页空状态的 4 个引导卡数据（软件开发 / 研究调研 / 内容创作 / 学习助手）
//   - 由 EmptyProjectCard.vue 渲染；由 ProjectManagerPage.vue 监听点击 → 打开创建弹窗并预填
//
// 设计要点：
//   - icon 字段为 lucide-vue-next 的图标组件（与 agentTemplates.ts 风格一致）
//   - accentClass 与 EmptyProjectCard.vue 内部的 CSS 渐变绑定（template-card--*）
//   - 字段全部 string / Vue Component，无业务依赖，可在任意组件复用

import type { Component } from "vue";
import { Code, Search, PenTool, GraduationCap } from "lucide-vue-next";

/**
 * 项目推荐模板定义
 *
 * 字段：
 *   - key          模板唯一标识（用于 emit 传值 / 父组件查找）
 *   - icon         Lucide 图标组件
 *   - title        推荐的项目名（父组件打开创建弹窗时预填 name）
 *   - description  推荐的项目描述（父组件预填 description）
 *   - accentClass  卡片渐变色类名（CSS 绑定）
 */
export interface ProjectTemplate {
  key: string;
  icon: Component;
  title: string;
  description: string;
  accentClass: string;
}

/**
 * 4 个项目推荐模板。
 * 顺序即渲染顺序。
 */
export const PROJECT_TEMPLATES: ProjectTemplate[] = [
  {
    key: "code",
    icon: Code,
    title: "软件开发",
    description: "主 Agent + 审查 Agent",
    accentClass: "template-card--ice",
  },
  {
    key: "research",
    icon: Search,
    title: "研究调研",
    description: "搜索 + 总结 + 写作",
    accentClass: "template-card--aurora",
  },
  {
    key: "writing",
    icon: PenTool,
    title: "内容创作",
    description: "大纲 + 文案 + 润色",
    accentClass: "template-card--violet",
  },
  {
    key: "learning",
    icon: GraduationCap,
    title: "学习助手",
    description: "讲解 + 测验 + 反馈",
    accentClass: "template-card--ember",
  },
];