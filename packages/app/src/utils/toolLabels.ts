/**
 * 工具展示名词表 —— 工具英文名 → 中文展示名（单一真相源）。
 *
 * 消费方：工具行展示名（ChatMessages 通用行/结构行）+ 展开内容组件。
 * 词表外裸透英文原值（技术兜底，不猜）——新增工具忘了配不会出错，
 * 与 McpSettings.builtinDescZh 同款降级口径。
 *
 * 当前覆盖 P1 范围（文件六件 + docx 四件 + read_file，2026-09-07 拍板）；
 * 其余工具（search_kb/run_command/屏幕族…）降级显英文原名 = 现状。
 */

/** 工具名 → 中文展示名（词表外裸透原值） */
export function toolDisplayName(name: string): string {
  return TOOL_LABELS[name] ?? name;
}

/** 展示名词表本体（导出供结构测试锁 11 项非空唯一，仿 stylePresets 范式） */
export const TOOL_LABELS: Record<string, string> = {
  write_file: "写入文件",
  edit_file: "编辑文件",
  delete_file: "删除文件",
  move_file: "移动",
  copy_file: "复制",
  create_directory: "创建目录",
  read_file: "读取文件",
  inspect_docx: "查看文档",
  edit_docx: "编辑文档",
  write_docx: "生成文档",
  validate_docx: "校验文档",
};
