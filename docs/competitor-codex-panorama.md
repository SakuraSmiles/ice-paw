# 竞品研读 02 — OpenAI Codex 全景（架构 / 技术实现 / 设计理念 / 功能盘点）

> 借鉴拍第二份（2026-08-16）。四问框架：它是什么、解决什么问题 / 靠什么架构 / 我们要不要 / 引入成本。
> **只积累不实施**——候选项进台账借鉴拍，动工须用户拍板。
>
> 置信纪律同 [01b](competitor-claude-code-panorama.md)：【官方文档】openai/codex 仓库一手 + developers.openai.com + openai.com/alignment 博客 > 【源码分析】社区源码级分析（deepwiki/agentgrep 等）+ changelog PR 推断 > 【社区观察】issue/第三方。两代理素材以官方为主，合成时社区单源降档、未证实保留标记。

## 一、它是什么、解决什么问题

**定位**："One agent for everywhere you code"——同一 agent 内核跨五形态：CLI（Rust，Apache-2.0 开源）/ IDE 扩展 / 桌面 App / 云端（chatgpt.com/codex，per-task 容器）/ 移动触点（iOS Codex tab 远程引导审批）。本地与云共享 5h 窗口 + 周上限的额度池。【官方文档】

**公开表述的设计公理**（官方文档/博客原文）：
1. **安全默认，本地云端同一套**——"By default, the agent runs with network access disabled and edits files restricted to the current workspace, **whether locally or in the cloud**."【官方文档】
2. **两轴正交**——Sandbox（技术执行边界：能写哪里/能否联网）× Approval（何时暂停问人）是两个独立控制；"Changing who reviews a request doesn't expand the sandbox."（换审查者不扩边界）【官方文档】
3. **审批摩擦本身是安全威胁**——"Approval friction harms security"：摩擦逼用户开 Full Access / 写过宽前缀规则 / 盲目批准（内部流量实测：相当多用户允许一切 `python` 开头的命令）→ 用独立审查 agent 替代人在边界同步盖章【alignment 博客】
4. **角色分离换可审计**——主 agent 有完成任务的压力、会把审批边界当障碍；批准决策放进**独立模型调用**使策略 "easier to evaluate, monitor, and improve"【alignment 博客】
5. **上下文卫生是产品质量主张**——公开引用 context pollution / context rot，子代理脏输出移出主线程、只回摘要【官方文档】
6. **诚实的能力边界声明**——auto-review "should not be treated as a guarantee of security"，不防 scheming；"We should aim for a future where agents can be trusted like employees. We do not live in that future today."【alignment 博客】

## 二、靠什么架构（子系统盘点）

### 1. 工程底座
- `codex-rs/` = **130+ crate Cargo workspace**（edition 2024），领域分组：核心循环（core/exec/tui/cli）/ 共享内核服务层（**app-server 六件套**）/ 沙箱（sandboxing/linux-sandbox/bwrap/execpolicy/network-proxy）/ 会话存储（rollout/state/thread-store/agent-graph-store）/ 模型接入 / MCP（rmcp =3.0.0）/ 扩展（ext/ 14 crate）/ ~25 个 utils【官方文档（Cargo.toml 一手）】
- **app-server JSON-RPC 2.0 = 一份内核、多种前端的共享基石**：CLI/TUI 直接链接 core，IDE 扩展与桌面 App 走 `codex app-server`（stdio JSONL 默认，WS/Unix socket 实验）；可生成版本化 TS/schema（`generate-ts`）；过载返回 -32001 + 有界队列退避【官方文档】
- 核心抽象三元组：**Thread（对话）⊃ Turn（一次请求+agent 工作）⊃ Item（输入输出单元 tagged union）**；事件驱动，"submissions flow in, events stream out"【官方文档 + 源码分析】

### 2. Agent loop 与事件模型
- 轮次生命周期：`turn/started` → `item/started|delta|completed` → `turn/completed`（终态 completed|interrupted|failed + 结构化错误分类表：ContextWindowExceeded / UsageLimitExceeded / ResponseStreamDisconnected 等 11 类）【官方文档】
- Item 类型 17+：userMessage / agentMessage（commentary|final_answer 两相）/ plan / reasoning（summary 流式摘要 + content 原始推理，分段 summaryIndex）/ commandExecution / fileChange / mcpToolCall / webSearch / contextCompaction…【官方文档】
- **turn 级操控**：`turn/steer`（带 expectedTurnId 向进行中 turn 追加输入不开新轮）/ `turn/interrupt` / `thread/rollback`（内存丢最后 N turn + rollout 持久化 rollback 标记，**不改写已写行**）/ `thread/inject_items`（绕过用户轮直接注入 items 并持久化）【官方文档】
- 容量治理：`rollout_budget`（实验，turn 级 limit_tokens + **reminder_interval 默认 10% 额度时注入提醒**给 agent 自管理）/ `model_auto_compact_token_limit`（自动压缩阈值）/ `tool_output_token_limit`（单工具输出预算）；**无官方暴露的每 turn 工具调用次数上限**（观察缺失非承诺）【官方文档】
- reasoning items 在工具循环内持久保留、跨用户轮边界丢弃【源码分析】

### 3. 沙箱与审批（两轴 × 五层）
- **平台机制**：macOS Seatbelt / Linux landlock+seccomp（社区指实际默认 bwrap + seccomp，文档滞后）/ Windows AppContainer 受限 token（实验）+ 代理环境变量禁网【官方文档 + 源码分析】
- **声明式权限 profile**（beta）：`:read-only`/`:workspace`/`:danger-full-access` 内置 + 自定义 `extends` 继承；filesystem 按 path/glob 给 read|write|deny（**嵌套内层覆盖外层**：全局可写但 `**/*.env` deny）；network 按 domain allow/deny【官方文档】
- **本地网络代理**：沙箱内联网走 127.0.0.1:3128 代理（HTTP/SOCKS），host+protocol+port 分组审批【官方文档】
- **execpolicy**：Starlark 语言的命令规则引擎；审批决定可携带 amendment 现场改放行规则【官方文档】
- **requirements.toml 管理员强制层**：只紧不松（allowed_approval_policies / deny_read 用户不可放宽 / MCP allowlist / 云端下发）【官方文档】
- **auto-review**（2026-04）：主 agent 到沙箱边界申请升级时，**另一个独立 Codex agent（GPT-5.4 Thinking 低推理）**审该请求——数据：人工打断减少 **~200x**、升级动作自动批准率 ~99%（全动作 99.93%）、拒绝后过半情况自寻更安全路径、反复被拒自动停轨迹防 gaming；`approvals_reviewer = auto_review`【alignment 博客】
- UI 三档：Read Only / Agent（默认）/ Full Access；目录信任门控（受版本控制 → Agent，否则 Read Only）；granular 审批按动作类型分策略【官方文档】

### 4. 会话与状态
- 会话 = **append-only JSONL rollout 文件**（`~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`），五种行：session_meta / **response_item（直接镜像 Responses API 的 ResponseItem）** / turn_context（每 turn 模型+策略快照）/ compacted（压缩检查点，后续轮基于最新检查点重建）/ event_msg【源码分析 + 社区观察】
- 索引层：session_index.jsonl + **SQLite 状态库**（agent jobs + resumable runtime state）——两侧不一致是已知问题源（#21196 JSONL 丢但 SQLite 有 / #29083）【官方文档 + 社区观察】
- 长线程：单文件实测 732MB/17 万行中 compacted 记录占 ~337MB（#24948）；损坏/超长从最新 compacted 检查点构建 rescue rollout【社区观察】
- fork：`thread/fork` 复制历史成新 threadId（forkedFromId）；rollback = 标记式非改写；archive/delete 级联 spawned 后代【官方文档】
- 跨会话 memory（默认关）：后台从历史 rollout 提炼/整合，提炼与整合可用不同模型【官方文档】

### 5. 多 agent
- 五工具：spawn_agent / send_input / resume_agent / wait_agent / close_agent；`max_threads` 默认 6、**`max_depth` 默认 1**（子 agent 不能再开子 agent）【官方文档】
- 子 agent 会话与主会话**同构存储**（sourceKinds: subAgent/subAgentReview/…）；主线程回收摘要不收原始噪声（context pollution 论证）【官方文档】
- 内置三角色 default/worker/**explorer（只读勘察）**；自定义 agent TOML 可覆写 model/sandbox/mcp_servers——官方示例直接展示「read-only reviewer + worker」按角色配沙箱【官方文档】
- 子代理**继承父会话沙箱策略与运行中的实时覆盖**（/permissions 改动、--yolo），即使自定义文件写了别的默认值【官方文档】

### 6. 扩展体系与 MCP
- Skills（SKILL.md + SKILL.json dependencies 声明 env_var/mcp 依赖可自动装）/ Hooks（10 事件，command 型）/ Plugins + marketplace（local/git/remote 三源）/ Apps（SaaS 连接器，destructive 注解工具无条件强制审批）/ 动态工具（客户端执行的 tool，持久化进 rollout）【官方文档】
- **MCP 双向**：作为客户端（stdio + streamable HTTP + OAuth，per-tool approval_mode，热重载）；**作为 server**（`codex mcp-server` 暴露 codex/codex-reply 两工具，把自己嵌进别的 agent 流水线）【官方文档】
- 官方 Claude Code 迁移通道：九类资产（AGENTS.md/skills/config/plugins/MCP/subagents/hooks/commands/sessions）一键导入【官方文档】

### 7. 多模型
- `[model_providers.x]` 自定义 + 内置 openai/ollama/lmstudio/bedrock；`--oss` 本地模式直连 Ollama/LM Studio【官方文档】
- **`wire_api` 唯一支持值 = "responses"**——Chat Completions 支持已从代码库移除（#7600：本地服务必须实现 Responses API）【官方文档 + 社区观察】
- 认证四模式（apiKey / chatgpt OAuth / 宿主代管 token / Bedrock）；gpt-oss:20b 经 Ollama 跑 Codex 质量差（Reddit 实测）【社区观察】

### 8. 配置分层与项目指令
- config.toml 四层：用户 → **受信项目 `.codex/config.toml`（凭证/provider/遥测键强制忽略——防仓库投毒）** → profile 文件 → requirements（管理员）；`-c key=value` 点路径一次性覆盖【官方文档】
- AGENTS.md：全局 → repo 根 → 子目录嵌套（**离文件最近的生效**）；进第一轮上下文；官方把「缩小 AGENTS.md」列为省额度手段；GitHub review 对每个变更文件应用离它最近的那份【官方文档】

## 三、我们要不要（对照 IcePaw 现状）

### 已对齐（两条独立竞品同构决策 = 交叉验证）
| Codex | IcePaw 现状 |
|---|---|
| append-only rollout JSONL + turn_context 快照 + compacted 检查点 | session_events 同构（append-only + turn_context 落库 + 摘要锚点）；且我们**单一 SQLite 真相源**，天然没有他们的 JSONL/SQLite 双侧不一致债（#21196/#29083） |
| `max_depth` 默认 1 | delegate v2 深度=1 结构护栏——同一决策 |
| rollback = 标记式不改写已写行 | P9 rewind 从事件派生路线被再次佐证；日志无损不变式两边一致 |
| 子代理独立会话同构存储、主线程只收摘要 | 委派子会话有自己的 session_events + tool_result 回传 |
| turn/plan/updated（pending/inProgress/completed） | plan_updated 事件 + update_plan 工具——同构 |
| 三档权限 UI + 目录信任门控 | P8 权限模式分档已在台账（加法拍首位候选） |
| --oss 对接 Ollama / 自定义 provider | 我们的产品本体就是本地多模型（GLM/OpenAI 兼容/Anthropic/Ollama），多 provider 适配层是我们的既定优势 |
| OTEL 遥测默认关、prompt 默认脱敏 | 本地 tracing 日志，无云端遥测——本地优先隐私立场一致 |

### 要借鉴（进台账 / 喂已有条目）
1. **两轴正交权限模型**（P8 设计输入升级）——「能力边界（能做什么）」与「审批时机（何时问人）」分开表述与配置。IcePaw 的 workspace 校验=能力轴、AuthorizationLevel=审批轴，P8 落地时按两轴组织而非单旋钮四档；"换审查者不扩边界"写成 P8 的验收不变式。
2. **审批摩擦 = 安全威胁的论证**（P8 价值论证）——摩擦的三种植竭式逃逸（全开/过宽规则/盲目批准）在 IcePaw 同样成立：用户把 Always 授权当默认就是我们的 yolo。治审批疲劳不只是体验问题。
3. **turn 级预算 reminder 注入**（S8 补充输入）——`reminder_interval` 默认 10% 额度时**向 agent 本身**注入「剩余额度」提醒让它自管理（收尾/分段），与我们的 HUD pill（给人看）互补：一个治透明度、一个治自调度。S8 的终止语义重排可吸收。
4. **auto-review 角色分离**（远期观察，非现在）——「主 agent 有完成任务压力、会把审批边界当障碍」的论证漂亮；IcePaw 版未来可= 辅助任务小模型（P10-②槽位）+ proposal_guard 的 LLM 化。本地单机授权弹窗量级远小于云端，先不做。
5. **explorer 只读勘察角色**（MA-2 输入）——多 agent 角色按沙箱分级（reviewer 只读 / worker 可写）是他们的推荐实践；我们 delegate v2 已有 AuthorizationLevel 通道，MA-2 任务台账时代可给「勘察型专家」预设只读档。
6. **AGENTS.md 嵌套分层 + 离文件最近生效**（agent.yaml 远期参考）——指令文档按目录就近覆盖、限量（project_doc_max_bytes）防吃上下文；我们单 agent 单 yaml，多 agent 项目化后若加项目级指令层，此模式可参考。

### 结构性更强（承认差距，不追）
- **OS 级沙箱**（Seatbelt/landlock/AppContainer + 网络代理 + Starlark 规则）——五年安全工程积累，平台相关大工程；我们 Windows 单机场景授权层 + 工作区边界已覆盖主风险面。
- **app-server 多前端共享内核**——跨端协议层的解法；我们 Tauri 单壳 webview 前端，无需跨端 RPC。
- **云端并行 + per-task 容器**——与本地优先定位相反的方向。
- 130+ crate 工程规模 / 订阅+credits 商业体系——规模与商业模式错配。

### 不借鉴（反面守则）
- **wire_api 只留 Responses API**——他们砍 Chat Completions 是单厂商收敛；我们多 provider 兼容是立身之本，反向佐证我们的适配层路线。
- **双真相源（JSONL + SQLite 索引）**——他们的对账债（#21196/#29083、732MB 单文件）反证我们「单一 SQLite + 事件日志同库」的 dsh 借鉴决策；**多真相源必有对账债**补进守则。
- **compacted 检查点膨胀**（占单文件 45%）——摘要态与原始态同文件混存的代价；我们的摘要锚（covered_until_rowid）+ 行表分离回避了此问题，Phase 2B 治理时注意勿引入。
- /yolo 保留但标注不推荐——我们的等价物（全局 Always）目前无警示文案；P8 落地时补「不推荐」标注即可，不需要学它的开关形态。

## 四、引入成本（若做对应借鉴项）

| 项 | 实现面 | 量级 |
|---|---|---|
| P8 两轴权限（模式前置短路层） | tool_executor 现有 AuthorizationLevel + workspace 校验之上加会话档位；两轴表述进文档与 UI | 小-中（P8 原估不变，设计语言升级） |
| S8 reminder 注入 | 预算剩余 <10% 时向 LLM 上下文注入一段固定提醒（MemoryStage/系统提示层） | 极小（文案级） |
| auto-review | 辅助任务小模型槽位 + 审批 prompt + 策略文件 | 中-大（远期，先靠 P8 弹窗分组减量） |
| explorer 只读角色 | delegate 可调度集元数据 + AuthorizationLevel 预设 | 小（MA-2 顺手） |

## 未证实存疑清单（引用前须核官方）

- `/reviews`、`/quests` 字面命令**未证实**——实际对应物是单数 `/review` 命令族 + "Side Quests"（社区报告的 0.122.0 功能，主任务运行中快速插问；仅社区推文 + `thread.side` 指标佐证）
- gpt-5.6 / gpt-5.6-terra / gpt-5.6-luna 命名——learn.chatgpt.com 与 developers.openai.com 两文档站存在代际差，未进一步证实
- pricing 页 GPT-5.4 云任务列显示 "Not available"——疑似渲染陈旧
- `v8-poc` crate（内嵌 V8）用途未证实；code_mode 与其的关联未证实
- Linux 默认沙箱实现 bwrap vs landlock——官方文档与社区源码分析不一致（文档滞后于实现）

## 来源

**官方一手**
- https://github.com/openai/codex （README + codex-rs/Cargo.toml workspace 清单）
- https://developers.openai.com/codex/ （security / cloud / pricing / cli / ide / app / integrations / changelog / config-reference / app-server / mcp-server）
- https://learn.chatgpt.com/docs/permission-modes.md 、 agent-configuration/subagents.md 、 config-file/config-advanced.md
- https://openai.com/index/codex-for-almost-everything/（2026-08-13 App 大更新）
- https://openai.com/index/running-codex-safely/（2026-08-04 安全部署原则）
- https://alignment.openai.com/auto-review/（2026-04-30 auto-review 设计与评测）
- https://openai.com/index/unlocking-the-codex-harness/

**issue / discussion（社区观察）**
- #24948（rollout 732MB 膨胀）、#25215（rescue rollouts）、#24425（坏 JSON 行吞会话）、#21196 / #29083（JSONL/SQLite 不一致）、#7600（OSS wire_api）、discussion #3827（文件名）、#1174（Rust 重写）

**社区源码分析**
- deepwiki.com/openai/codex 、 zread.ai（agent system / TUI）、 agentgrep.org（RolloutItem schema）、 agent-safehouse.dev（bwrap/Landlock 演进）、 news.lavx.hu（agent loop）、 swequiz / bytebytego 架构文、 simonwillison.net 沙箱调查、 pierce.dev agent sandbox 深潜
