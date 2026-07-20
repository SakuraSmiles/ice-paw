// 内置聊天模板（Phase 1 硬编码）
//
// 职责：
//   - 提供新会话空状态的 4 个引导卡片数据
//   - 用户点击卡片后自动填入 textarea draft（Phase 1 不做变量交互）
//
// 设计要点：
//   - icon 字段为 lucide-vue-next 的图标组件名（字符串），
//     TemplateCards 组件按名映射到实际组件
//   - content 中的 {变量} 为占位符，Phase 1 用户手动替换

/**
 * 内置聊天模板（Phase 1 硬编码，非用户自定义 Template）。
 *
 * 与 types/index.ts 的 Template（数据库实体）不同：
 * - 此处仅用于空状态卡片引导，不走 CRUD 流程
 * - content 是完整的 user message 文本（非 system_prompt / user_prompt_prefix 拆分）
 */
export interface BuiltinTemplate {
  id: string;
  name: string;
  /** lucide-vue-next 图标组件名 */
  icon: string;
  description: string;
  /** 模板内容，{变量} 为占位符（Phase 1 用户手动替换） */
  content: string;
}

/**
 * 4 个内置聊天模板卡片。
 * 用于新会话空状态引导，点击后填入 textarea。
 */
export const BUILTIN_TEMPLATES: BuiltinTemplate[] = [
  {
    id: "code-review",
    name: "代码评审",
    icon: "Code2",
    description: "审查代码质量、风格、潜在问题",
    content:
      "请评审以下代码，从命名规范、错误处理、边界条件、安全性角度给出建议：\n\n```\n{粘贴代码}\n```",
  },
  {
    id: "brainstorm",
    name: "头脑风暴",
    icon: "Lightbulb",
    description: "快速生成创意和方案",
    content: "关于「{主题}」，请帮我头脑风暴，给出 5-10 个创意方向。",
  },
  {
    id: "explain-code",
    name: "解释代码",
    icon: "FileCode",
    description: "理解陌生代码的功能和逻辑",
    content:
      "请解释以下代码的功能和逻辑：\n\n```\n{粘贴代码}\n```",
  },
  {
    id: "translate",
    name: "翻译",
    icon: "Languages",
    description: "中英互译，保持技术术语准确",
    content: "请将以下内容翻译为{目标语言}：\n\n{文本}",
  },
];
