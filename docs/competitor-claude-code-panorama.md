# 竞品研读 01b — Claude Code 全景（架构 / 技术实现 / 设计理念 / 功能盘点）

> 承接 [01 切片](competitor-claude-code.md)（上下文压缩与续跑深潜），本篇是整产品全景四问。**只积累不实施**——候选项进台账/借鉴拍，动工须用户拍板。
>
> 置信纪律同 01：【平台文档】官方 docs / 工程博客 > 【源码分析】社区逆向（质量高但非承诺）> 【社区观察】issue/博客推断。三代理原始素材中社区占比偏高，合成时已收紧：Reddit 单源结论降档或剔除、agents 自标「未证实」的一律保留标记。

## 一、它是什么、解决什么问题

**定位**：终端优先的 agentic coding 载体，正在 SDK 化为通用 agent 引擎（Claude Code SDK → 2026 更名 Claude Agent SDK——从「编码助手」扩张为「构建自己的 agent 应用」的底座）。四形态共享同一 agent loop：CLI / IDE 扩展（VSCode + JetBrains）/ 桌面 / web；headless（`claude -p` + stream-json）面向 CI。【平台文档 + 社区观察】

**公开表述的设计公理**（Anthropic 工程博客/研究）：
1. **工具对等**——「给 Claude 与程序员相同的工具」（找文件/读改写/跑命令/搜索），而非扩大权限【官方博客】
2. **Context 是主动管理的资源**——压缩管道（见 01 切片）+ 工具输出预算 + 渐进加载，全是这一条的具体化【官方博客】
3. **可控性优先于自动化程度**——permission modes + hooks 双层守卫 + plan mode 只读探索【平台文档】

## 二、靠什么架构（子系统盘点）

### 1. Agent loop
- 单循环 + 工具分发；流式下**工具在响应流式到达时即开始执行**（不等完整 turn），thinking → tool_use → thinking → tool_use 可交错【源码分析】
- **并发/串行分组**：一轮内独立工具并行执行、依赖调用串行——两层并发优化【源码分析】
- 终止治理：token 预算检查 + 压缩阈值 + Stop hook 可 `{"ok":false}` 拦停（社区报过该机制自旋 bug #55754）【平台文档 + 社区观察】；**无单会话轮数上限**（同 01 切片结论）

### 2. 内置工具集
| 工具 | 职责 / notable 设计 |
|---|---|
| Read / Write / Edit / Delete / Move | 文件五件套；**Edit = 精确字符串替换**（非 diff/正则），失败恢复阶梯：唯一性匹配失败 → 扩宽上下文重试 → `replace_all`；大文件小编辑自动降级 Write【平台文档】 |
| Glob / Grep | 模式匹配 + ripgrep 封装；结果按修改时间排序【平台文档】 |
| Bash | 默认 2min 超时（可至 10min）；**30K 字符截断 + 全文落临时文件可再读**；被官方称为「最昂贵的表面」【平台文档】 |
| Task（Agent） | spawn 子代理【平台文档】 |
| WebSearch / WebFetch | 搜索 + URL→markdown【平台文档】 |
| NotebookRead/Edit、TodoWrite、AskUserQuestion、EnterPlanMode、exit_plan_mode | 辅助件【平台文档】 |

### 3. 子代理与多代理
- 子代理 = 独立 context window + 独立 system prompt + 可选工具子集；`.claude/agents/*.md` 零代码定义（frontmatter: name/description/tools/model），主 agent 按 description 与意图**自动路由委派**【平台文档 + 社区】
- **worktree 隔离**：`isolation: "worktree"` 建临时 git worktree（`.claude/worktrees/<name>/`），子代理命令困在自己副本内——并行开发不冲突；社区抱怨非 git 目录下体验差、隔离近乎强制【平台文档 + 源码分析 + 社区观察】
- Agent Teams：命名队友（预定义专用 agent）+ 共享任务列表（`CLAUDE_CODE_TASK_LIST_ID` env 多实例共享 + worktree 状态标志防冲突）【平台文档 + 社区】
- 社区实践共识：**深度 = 1**（单层委派）防链式失控；探索型子代理配便宜模型（Haiku）省成本【社区观察】

### 4. 扩展体系五件分工
| 机制 | 定位 | 一句话选型 |
|---|---|---|
| Skills | 能力封装（SKILL.md：metadata ~100 tok 常驻 + body <5K 按需） | 要「可分享的行为模式/命令」→ Skills |
| Hooks | 生命周期事件触发（command/HTTP/prompt/agent 四类动作；PreToolUse 可 allow/deny/modify） | 要「事件驱动的守卫/注入」→ Hooks |
| MCP | 标准协议外接工具 | 要「外部服务/工具」→ MCP |
| Plugins | 打包分发（skills+agents+hooks+MCP 一揽子） | 要「整包分发」→ Plugins |
| Subagents | 独立 agent 流程 | 要「独立上下文干活」→ Subagent |

【平台文档，选型决策流程系社区指南】。Skills 渐进加载**尚未完全 lazy**（issue #16160 推动中：理想 = metadata 进上下文、判定相关才读 body）【社区观察】。

### 5. 会话与状态管理
- transcript = 本地 JSONL（`~/.claude/projects/<项目>/` 下），含完整对话+工具调用记录【源码分析 + 社区】
- `--resume <session-id>` / `--continue` 恢复完整上下文；同会话双终端 resume 会消息交错（无 fork 保护）【平台文档 + 社区】
- **checkpoint/rewind**（Esc Esc 或 `/rewind`）：对话+代码状态一起回退；局限——不重写 git 历史、已删文件大多不恢复【平台文档】

### 6. 模型路由与多模型
- Opus（深推理/规划）/ Sonnet（编码主力）/ Haiku（快查/探索子代理）分工是**社区实践**非自动路由（自动路由是 issue #44976 功能请求未实现）【社区观察】
- Fast mode：Opus 高速配置（≤2.5×，每 token 更贵），受 managed allowlist 约束【平台文档】
- 配置链：`--model` > `/model` > env（`ANTHROPIC_MODEL`）> settings.json > 默认【平台文档 + 社区】
- Bedrock / Vertex / 自定义 endpoint 可接，但模型宇宙仍是 Claude 单厂商【平台文档】

### 7. 权限与配置
- **Permission modes**：`default`（逐次确认）/ `acceptEdits`（编辑自动批，其余确认）/ `plan`（只读探索 + 产出计划文档，浏览器呈现，批准后执行）/ `bypassPermissions`（`--dangerously-skip-permissions`）；Shift+Tab 循环切换【平台文档】
- **allow/deny 规则**（settings.json `permissions` 字段）：`Read:*`、`Edit:src/**/*.ts`、`Bash:rm *` —— 工具级 + 路径通配【平台文档】
- PreToolUse hook 在权限检查**之前**执行，可 allow/deny 短路——hooks 与权限形成双层守卫【平台文档】
- **settings 五层**：enterprise managed > project `.claude/settings.json` > user `~/.claude/settings.json` > local `.claude/settings.local.json` > env【平台文档】
- **记忆三层**：enterprise/project/user CLAUDE.md + auto memory（`~/.claude/projects/*/memory/`）+ `#` 快捷记忆 + `@import` 模块化引入；CLAUDE.md 管「内容性知识」，settings 管「行为控制」【平台文档】

### 8. 其余关键功能速览
- **任务追踪**：TodoWrite → Tasks 演进（持久任务列表、跨 session、依赖关系、多 agent 共享）【平台文档 + 社区；共享列表机制细节系社区逆向】
- **后台/定时**：`run_in_background` 后台会话 + 本地 cron 定时（每会话 ≤50 任务）+ 云端 Routines【平台文档】
- **成本可见性**：`/cost` `/usage` + statusline 实时花费（rich JSON payload 喂自定义脚本）+ 企业 spend caps【平台文档】
- **输入语法**：`@file` 引用、`#` 记忆、`!` bash、图片粘贴/拖拽【平台文档】
- **企业治理**：managed settings / analytics / audit / SSO / MCP 审批流【平台文档】

## 三、我们要不要（比对 IcePaw 现状）

### 已对齐（架构级等价或更强，无需动）
| 它的机制 | 我们的对应 | 判 |
|---|---|---|
| transcript JSONL append-only | session_events + reconcile + derive-on-read | **我们更强**：对账/回放/双读路径是它没有的纪律 |
| checkpoint/rewind | 事件日志天然可回放（未做 UI 入口） | 地基已有，见「要借鉴 #5」 |
| 子代理深度=1 + 独立上下文 | MA-1 delegate v2（真子会话 + 深度=1 护栏） | 等价 |
| 零代码 subagent（md 定义） | agent = 出生证表单 + agent.yaml 行为层 | 等价（形态不同） |
| 工具输出截断 + 落盘 | infra/strings 安全截断 + per-round 持久化 | 等价（01 切片已结论） |
| 内置工具 native 化 | file_tools.rs | 等价 |
| plan mode | 计划系统（update_plan + PlanCard，第 14 类事件） | 等价 |
| 多模型 | **我们更强**：GLM/DeepSeek/Ollama/MiniMax 多厂商 vs 它 Claude 单宇宙 | 架构性优势 |
| 成本可见性 | BudgetPill HUD + chat:budget + 续期 toast | 等价 |
| hooks 生命周期 | 4 接入点 + 内置动作（InjectPrompt/CallTool/Log） | 等价，且我们的「内置动作 vs 任意 shell」是**有意的安全取舍** |
| 授权审批 | 工具授权对话 + 分层记忆（Once/ThisDir/ThisTool）+ 配置提案 guardrail | 等价，提案通道是我们独有 |

### 要借鉴（候选项，只积累不实施）
1. **权限模式分档**（default / acceptEdits / plan / bypass 四档旋钮 + allow/deny 规则文件）——治「审批疲劳」：现在每敏感调用都弹窗。落点符合配置阶梯：模式=会话级 UI 旋钮（L2），规则=agent.yaml（L3）。**候选项里性价比最高**
2. **并发/串行工具分组**——一轮内独立工具并行执行。我们目前串行；长任务延迟收益明显。落点 loop_engine 工具分发层
3. **Edit 失败恢复阶梯**（唯一匹配失败 → 扩宽上下文 → replace_all 提示）——我们 Edit 工具已精确替换，但恢复策略靠模型自觉；可把阶梯写进工具 description 引导
4. **Bash 30K 截断 + 全文落盘可再读**——即 01 切片 L0 结论的交叉印证（确定性手段排 LLM 前）
5. **rewind/分叉 UI 入口**——事件日志给了我们免费的「回到第 N 轮重开」能力（derive from events + 新会话锚定），它要做 checkpoint 快照，我们只需投影。**有机会做成比它更干净的产品点**（它的局限：不重写 git 历史、已删文件不恢复——我们从事件派生没有这些坑）
6. **Skills 渐进加载思想**（metadata 常驻 + body 按需）——工具集描述膨胀、KB 检索提示都是同构问题；长期可作 agent.yaml 的「能力包」形态参考
7. **便宜模型跑探索性子任务**——我们多 agent 架构天然支持（每个 agent 自带模型），但**默认值设计**可借鉴：委派/摘要/代读类辅助任务倾向给小模型

### 我们有机会更强（结构性差异）
- **熔断后不搁浅**：它压缩熔断后无回退（01 切片）；S8 确定性折叠填这个洞
- **本地优先**：Ollama 全离线 + stronghold 密钥不出机器 vs 它的云端计费焦虑（用户社区对 thinking token 计费的不满即此）
- **事件日志泛化**：它 Context Collapse / checkpoint 各自局部实现的「非破坏投影」，我们有全局地基（append-only 唯相 + 压缩只作用实时窗口——产品愿景锁定不变式）

### 不借鉴
- 企业治理（SSO/audit/spend caps/managed settings）——非我们的市场
- 云 Routines——与本地优先冲突
- settings 五层——我们的配置放置阶梯（L1 默认/L2 上屏/L3 yaml/L4 出生证）比五层 JSON 更符合极简哲学
- 任意 shell 命令 hooks——攻击面大；内置动作 + 提案通道是我们的有意取舍
- 它的重型 slash 命令体系（50+）——GUI 形态下用不上这个量级；命令面板级即可

## 四、引入成本（候选项量级）

| 候选 | 实现面 | 量级 |
|---|---|---|
| 权限模式分档 + 规则文件 | 工具授权层加模式枚举 + yaml 规则解析（复用现有 Once/ThisDir/ThisTool 记忆结构） | 中 |
| 并发工具分组 | loop_engine 分发层（需审授权时序：并行的授权弹窗会乱序） | 中 |
| Edit 恢复阶梯 | 工具 description 文案 + 错误信息引导 | 极小 |
| rewind/分叉入口 | 前端入口 + `load_history_from_events` 锚 seq 派生新会话（derive 已有） | 小-中 |
| 辅助任务小模型默认 | 摘要/代读 provider 选择处默认值 | 小 |
| Skills 式能力包 | 远期，agent.yaml 扩展形态 | 大（先观察） |

共同前提不变式不变：**一切投影/折叠/rewind 只读事件日志，永不改写 session_events**。

## 来源（去重精选）

官方：agent loop（code.claude.com/docs/en/agent-sdk/agent-loop）、tools-reference、sub-agents、skills、hooks、plugins-reference、sessions、checkpointing、model-config、fast-mode、env-vars、permission-modes、settings、memory、costs、scheduled-tasks、common-workflows、admin-setup、mcp；platform.claude.com 的 parallel-tool-use / bash-tool / compaction（01 切片）；anthropic.com/engineering（agentic coding best practices）、/research/claude-code-expertise。

社区（源码分析）：georgesung LLM traffic tracing、claude-code-from-source（GitHub book）、wong2 gist（系统提示与工具）、HarrisonSec / Barazany / Decode Claude（01 切片已列）。

社区（观察）：issue #44976（自动路由请求）、#59416（子代理无 WebSearch）、#55754（Stop hook 自旋）、#34886（worktree 耦合）、#16160（skills lazy loading）、#47926（跨设备恢复）；ccusage / cccost 生态。

**未证实存疑清单**（引用前先核官方）：hooks「30 事件」（官方核心清单远小，30 系社区博客口径）；「RECUSE 停止哨兵」（仅学术论文口径）；本地 `.claude/workflows/` 定义格式；MAX_THINKING_TOKENS 之外的环境变量全表；Tasks 共享列表的冲突预防细节；「thinking 故意隐藏系成本焦虑」（Reddit 单源，已剔除不采信）。
