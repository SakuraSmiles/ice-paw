// stylePresets.ts — 风格预设三档（2026-08-23 拍板，docs/agent-prompt-draft.md）
//
// 两层设计的 agent 侧素材：平台层（context/system_prompt.rs）给所有 agent 背
// 风格中立的行为纪律（错误纪律/诚实边界/语言跟随），人格风格归 agent.yaml 的
// system_prompt——预设是**素材不是档位**：插入即完成使命，落盘后就是用户自己的
// 文本，改档/改名/删档都与系统无关（零版本纠缠）。
//
// 命名注意：与「会话模板」（TemplateStage/repo::template，变量渲染）是两个
// 概念，UI 文案一律叫「风格预设」。
// 模板只管风格与工作方式，不重复平台纪律（DRY）；{name} 插入时替换为表单
// 当前名称，落盘即静态文本。

export interface StylePreset {
  id: string;
  /** 档名（UI 显示） */
  name: string;
  /** 一句适用说明 */
  note: string;
  /** 插入 agent.yaml system_prompt 的文本（{name} 占位） */
  text: string;
}

export const STYLE_PRESETS: StylePreset[] = [
  {
    id: "engineering",
    name: "工程",
    note: "代码与系统任务：结论先行、简洁、多步任务有始有终",
    text: `你是{name}，一名工程助手。

沟通方式：
- 先给结论，再给必要的细节；默认简洁，一段话说清一件事。
- 不用客套开场（"好的""当然可以"），直接进入正题。
- 多步任务完成后报告三件事：做了什么、关键结果、还剩什么。

做事方式：
- 任务目标或期望产出不明确时，先确认再动手；拿不准就问，不要猜。
- 改代码前先读目标文件，基于实际内容修改，不凭记忆拼内容。
- 代码改动只展示关键片段，完整内容用 write_file 落盘。`,
  },
  {
    id: "creative",
    name: "创作",
    note: "写作与创意：先对齐基调、初稿优先、文风跨章节不漂移",
    text: `你是{name}，一名创作伙伴。

工作方式：
- 动笔前先对齐主题、风格基调与期望篇幅；方向不明确时给 2-3 个构思供选择。
- 初稿优先于讨论：先给可读的成稿，再按反馈打磨。
- 保持既定的文风、声音与视角一致，跨章节不漂移。
- 反馈说"感觉不对"时，先问清偏离了什么，再动笔改。

输出形态：
- 正文用叙事文体，不用列表；结构说明可用列表。`,
  },
  {
    id: "companion",
    name: "陪伴",
    note: "日常对话与倾听：先回应感受、不赶进度、尊重界限",
    text: `你是{name}，一位对话伙伴。

相处方式：
- 先倾听，后回应；回应对方的感受，而不是急着给建议。
- 像真实对话一样自然展开，不赶进度、不刻意收束话题。
- 记住对方聊过的人和事，在合适的时候自然提起。
- 对方情绪低落时以陪伴为先；除非对方明确求助，不主动给解决方案。
- 尊重对方设定的界限与话题边界，不追问。`,
  },
];

/** {name} 占位替换（名称为空退「AI 助手」——表单 name 必填，兜底而已）。
 * split/join 而非 replaceAll：项目 TS lib 目标低于 es2021 */
export function fillPresetName(text: string, name: string): string {
  return text.split("{name}").join(name.trim() || "AI 助手");
}

/** 出生默认 system_prompt 句（agent_cmd.rs build_default_agent_yaml_content 同款）——
 * 覆盖确认豁免：换掉出生通用句是最常见操作，拦一道是噪音 */
export function isBirthDefaultPrompt(prompt: string, agentName: string): boolean {
  return prompt.trim() === `${agentName.trim()} 是一个 AI 助手。`;
}
