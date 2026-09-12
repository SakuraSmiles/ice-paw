// toolGroups.ts — 工具组展示词表（2026-09-12 工具集权限批次）
//
// 组键 = 后端 `harness/mcp/tool_scopes.rs::TOOL_GROUPS` 单一真相源（固定名单
// 快照：组 = 建组时固化名单，新增工具不自动进组）；本文件只放**展示名**——
// 组成员数从 `list_builtin_tools` 响应的 group 字段聚合，勿在前端手抄名单。
// 「other」是设置页展示兜底（未分组工具），**不是合法 scope 组**。

/** 组键 → 展示名（McpSettings 分组标题 / AgentForm 工具区块选项共用） */
export const TOOL_GROUP_LABELS: Record<string, string> = {
  files: "文件与命令",
  web: "网页",
  kb: "知识库",
  attach: "附件与引用",
  docx: "Word 文档",
  config: "配置与计划",
  screen: "屏幕读写",
  other: "其他",
};

/** 组展示顺序（镜像后端 TOOL_GROUPS 组序；other 兜底恒在末位） */
export const TOOL_GROUP_ORDER: string[] = [
  "files",
  "web",
  "kb",
  "attach",
  "docx",
  "config",
  "screen",
  "other",
];
