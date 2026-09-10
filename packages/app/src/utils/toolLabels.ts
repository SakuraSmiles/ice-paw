/**
 * 工具展示名词表 —— 工具英文名 → 中文展示名（单一真相源）。
 * 消费方：聊天气泡工具行（ChatMessages 通用行 + toolSummary 结构行）+
 * 设置-工具集（McpSettings 内置工具行主名）。
 * 词表外裸透英文原值（技术兜底，不猜）——新增工具忘了配不会出错，
 * 与 McpSettings.builtinDescZh 同款降级口径。
 * 2026-09-07 二轮扩全：内置 39 件 + delegate_to_agent（chat 错误兜底行用），
 * 词表与 McpSettings.BUILTIN_TOOL_META 的 key 集合保持同步（增工具两边一起补）。
 */
export function toolDisplayName(name: string): string {
  return TOOL_LABELS[name] ?? name;
}

/** 展示名词表本体（导出供结构测试锁项数非空唯一，仿 stylePresets 范式） */
export const TOOL_LABELS: Record<string, string> = {
  // 文件与命令
  write_file: "写入文件",
  edit_file: "编辑文件",
  delete_file: "删除文件",
  move_file: "移动",
  copy_file: "复制",
  create_directory: "创建目录",
  read_file: "读取文件",
  read_multiple_files: "批量读取",
  list_directory: "列出目录",
  directory_tree: "目录树",
  get_file_info: "文件信息",
  search_files: "搜索内容",
  run_command: "执行命令",
  git: "Git 查询",
  // 网络获取
  web_fetch: "抓取网页",
  // 知识库
  search_kb: "检索知识库",
  read_kb_document: "读取知识库文档",
  save_to_kb: "存入知识库",
  // 附件与引用
  read_attachment_page: "读取附件页",
  view_attachment_image: "查看附件图",
  read_reference: "读取引用",
  // Word 文档
  inspect_docx: "查看文档",
  edit_docx: "编辑文档",
  write_docx: "生成文档",
  validate_docx: "校验文档",
  // 配置与计划
  read_agent_config: "读取配置",
  propose_config_change: "提出配置提案",
  update_plan: "更新计划",
  // 屏幕操作
  capture_screen: "截取屏幕",
  list_windows: "列出窗口",
  capture_window: "截取窗口",
  mouse_move: "移动鼠标",
  mouse_click: "点击鼠标",
  mouse_drag: "拖拽鼠标",
  mouse_scroll: "滚动滚轮",
  type_text: "输入文字",
  press_key: "按下按键",
  wait: "等待",
  request_screen_session: "请求屏幕共享",
  // 委派
  delegate_to_agent: "委派任务",
  // 跨会话通讯（MA-3）
  send_message_to_session: "跨会话投递",
  list_conversations: "列出会话",
};
