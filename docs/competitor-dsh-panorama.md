# 竞品研读 05 — DeepSeek Harness 全景（架构 / 技术实现 / 设计理念 / 功能盘点）

> 借鉴拍第五份（2026-08-20）。四问框架：它是什么、解决什么问题 / 靠什么架构 / 我们要不要 / 引入成本。
> **只积累不实施**——候选项进台账借鉴拍，动工须用户拍板。
>
> 置信纪律：本份素材等级最高——【源码一手】本地完整 checkout（deepseek-harness 仓库，v0.1.0-rc.8）逐文件研读：README / docs/architecture.md / packages/README.md / 三个 bundle patch 全文 / 4 个 agent preset 全文 / agent-spine-demo。无社区转述环节。注意其为 developer preview，破坏性变更频繁，条目以当日快照为准。

## 一、它是什么、解决什么问题

**定位**：DeepSeek AI 开源的 **agent 宿主框架（harness）**——不是聊天产品，是"让 agent 干活的运行环境"：模型适配、工具执行、沙箱、审批、持久化、多 agent 编排、Web GUI 全栈，且**一切皆插件**（含 agent loop 本身）。命令行 `dsh`，npm 分发（`npx @deepseek-ai/dsh web`），本地 3080 端口起 Web GUI。【源码一手】

**设计公理**（README + architecture.md 原文提炼）：
1. **一切皆插件，没有特权核心**——"There is no privileged core to patch"：模型适配器、工具注册表、会话日志、agent loop 全是 Cordis 插件，从配置替换，不 fork 代码。
2. **模型可见即可从日志重建**——运行时不变量：凡进模型请求的内容必须能从 session log 重构，违规即 assert。fork/resume/回放/遥测全从日志派生。
3. **注册即可逆**——插件注册的服务/事件/副作用在卸载时自动 unwind，热插拔安全。
4. **接缝三分**——每个能力拆 Service Definition（接口）/ Provider（实现）/ Consumer（通常=模型工具）三角色；"一个 provider 换掉整个产品"（fs+subprocess 共享执行世界，指向远程沙箱则 Bash/PTY/LSP 整体迁移）。
5. **日志中心 × 压缩投影视图**——compaction 只作用于喂模型的 projection，不改日志。
6. **组合分层**——profile（命名组合）堆叠 bundle（分发格式），上层 patch 可替换下层任意行；用户层永远赢。

## 二、靠什么架构（子系统盘点）

### 1. 工程底座
- pnpm monorepo，~50 个能力组（`@deepseek-ai/dsh-*`），分 Product 稳定 API / POC / Support 三档预期；依赖图 CI 门禁保鲜（`gen-module-graph` freshness-gated）。【源码一手】
- 基于 **Cordis**（Koishi 系 DI/插件框架，论文《A Programming Paradigm for Spatiotemporal Composability》）：插件向共享 context 贡献服务、typed 事件、可逆 effect。

### 2. 组合机制（profile × bundle × patch）
- 运行中的 dsh = 启动时按层组装的插件树：`dsh-base`（所有模式第一层，~77 行）→ 模式 bundle（`dsh-web-app` 加浏览器面 ~35 行 / `dsh-headless` 一次性运行器）→ profile 自带 `cordis.patch.yml` → home 级 → `--patch` 临时层；patch 按 id 定位行、整体替换 config，last-write-wins。【源码一手】
- `dsh --profile web --dump-config` 可随时 dump 实际树——配置即文档。
- **agent preset**（web 模式第三层）：模型工具不在 base 生效，移到 per-session 的 preset；出厂 4 个——`standard`（全量）/ `code`（+Code Mode：模型写 TS 程序一次执行，5 次往返变 1 次）/ `cordis`（+自指工具集：agent 读写自己运行的插件运行时，"让 agent 造 agent"）/ `minimal`（仅 2 工具：持久 shell + str_replace_editor，固定 prompt，无压缩）。【源码一手】

### 3. Agent loop 与事件模型
- **turn/step 两级**：step = 一次模型请求 + 其工具调用；turn = 零或多 step，"opens before its first input is claimed and closes once nothing is owed"；**被拒绝或空的首个 claim 也关闭一个零 step 的持久 turn**——日志记录尝试本身，审计完整。【源码一手】
- 三域事件：session 事件（durable，落日志）/ agent 事件（live：inbox、pre-step、request、validation、continuation）/ 能力事件（fs/*、tools/* 挂策略不碰 loop）。`agent/pre-step`、`llm/stream`、`tools/*` 为 waterfall（必须 `next()` 委派，可逐层改写拦截）；`agent/turn-stopping` 串行无 next。【源码一手】
- 输入经单一 inbox；部分消息立即唤醒，注入上下文排队等下一条唤醒消息。

### 4. 会话与状态
- append-only `SessionEvent` 日志为唯一真相源；`deriveMessages()` 投影模型历史；**"model-visible means logged" 是运行时 assert**（新模型可见输入必须新增 SessionEvent 并从日志渲染——与我们 derive-on-read 主读路径同构，但他们以不变量强制而非迁移对账达成）。【源码一手】
- 持久化 seam：JSONL / SQLite 后端可换；`session-query`（全文检索 opt-in，默认 `openAt: never` 不开 SQLite）；`session-checkpoint-policy` 每次模型请求前落检查点；`session-title` log-backed 标题服务；fork = `sessions.fork(source, boundary?)`。【源码一手】

### 5. 沙箱与审批（两轴，与我们/Codex 三方同构）
- **Sandbox 轴**：`dsh-sandbox` 进程 confinement seam，bwrap（Linux）/ Landlock / Seatbelt（macOS）三后端；文件效果边界 `fs-sandbox`（read-only / workspace-write / danger-full-access）。【源码一手】
- **Approval 轴**：`approval` 插件（ask / never）；`permission-presets` 把两轴绑成三档预设（read-only+ask / workspace-write+ask / danger-full-access+never）。【源码一手】
- 与 Codex 对照：同是两轴正交 + 三档 UI，DSH 少了 execpolicy 规则引擎与 auto-review，多了"审批插件本身可换"。

### 6. 多 agent 与编排（对我们 MA 设计最有参照价值）
- **subagent seam**：provider 注册表（spawn=全新子代理 / fork=继承历史的一次性分身 / 接外部产品如 codex、claude-code 作子代理——preset 里有现成禁用行）+ 模型面工具（subagent / subagent_fork）；continuable 子代理有 `report` 回传通道 + `send_message` 全局后续消息工具 + `list_agents` 跨会话查询。【源码一手】
- **委派=会话**：子代理有自己的 session log；continuable 背景子代理按委派工具选择。**与 MA-1"委派会话化"同构**。
- **workflow 引擎**：worker-thread 执行 JS 编排脚本（`agent()/pipeline()/parallel()` fan-out，schema 校验子代理结果）+ `ralph` 工具（fresh-agent 迭代循环：每轮全新无记忆子代理，共享 workspace 为长期记忆，上限 64 轮）。【源码一手】
- **Agent Teams**（实验性）：`ctx.agentTeams` 私有协调 seam——持久名册 + 任务板 + 邮箱，叠在 continuable 子代理上；**与我们 MA-3 通道设计方向一致，但走"协调 seam"而非"统一 session 类型"**。【源码一手】
- goal 体系：同 session 跨轮目标持久化 + 轮驱动（round limit / blocked 三轮判定 / resume 重臂）——长任务不靠单 turn。【源码一手】

### 7. 扩展体系与集成
- `extensions` 包：**agent 运行时自我修改**——live 插件/服务检查 + 模型写插件挂载/卸载（`cordis` preset 的底座）。【源码一手】
- `hooks` 包：Claude Code / Codex 线协议桥（复用两家的 hooks 生态）；`mcp`（客户端）；`lsp`（stdio provider + lsp 工具）；`acp`（Automation-only Agent Client Protocol server）；`sdk`（进程外 JSON-RPC 运行时）。【源码一手】
- skill 体系：provider 注册表 + 本地文件系统 provider + 会话前缀目录 + `skill` 工具；与 AGENTS.md（`agent-instructions`）分层共存。【源码一手】

### 8. 上下文预算
- 分层治理：`tool-result-pruner`（单条工具结果 8192 字符阈值，头 4096+尾 1024 保形截断）→ `compaction-basic`（全局对话压缩）→ `spill`（超预算工具结果外溢存储，50KB 内联线）；`token-meter` 进程级计量（host plane，Web stats 条读它）；`repeat-tool-reminder` 连续重复调用提醒（阈值 3/5/8）。【源码一手】

## 三、我们要不要（对照 IcePaw 现状）

### 已对齐（独立同构 = 交叉验证）
| DSH | IcePaw 现状 |
|---|---|
| append-only session log + deriveMessages 投影 + "model-visible means logged" 运行时 assert | session_events + derive-on-read 主读路径（Phase 2B 已退役 legacy）；他们靠新写不变量、我们靠迁移对账——两种到达方式，同一终点 |
| 零 step turn 也持久关闭（记录尝试） | turn_ended 先于 cleanup 落库——审计完整性同执念 |
| 压缩只作用投影视图不动日志 | TokenWindowStage/滚动摘要 vs session_events 无损——不变式两边一字不差同构 |
| 委派=会话、子代理有独立日志 | MA-1 愿景不变式 3"委派边界一律 session，无 event 逃生口" |
| sandbox 轴 × approval 轴两轴正交 | workspace 校验（能力轴）× AuthorizationLevel（审批轴）——与 Codex 三方互证 P8 |
| skill 目录 + agent-instructions 分层 | agent.yaml + KB——按受众分层同思路 |
| provider seam + settings 热重载（`llm-deepseek`/`llm-pi-ai` 休眠挂载） | per-Agent 自持 provider/model/key——我们的更贴桌面多角色场景 |

### 要借鉴（进台账 / 喂已有条目）
1. **能力接缝三分法**（MA-1 delegate.rs 升格的设计输入）——Service Definition / Provider / Consumer 三角色分离。我们的 delegate v2 若把"委派后端"（本地子会话 / 未来接 Codex、Claude Code 作专家）留成 seam 而非硬编码，MA-2/3 不用重构。DSH preset 里 `subagent_codex`/`subagent_claude_code` 禁用行就是"后端可插"的现成范本。
2. **工具结果预算分层**（预算诚实化后续）——pruner（单条结果保形截断，头尾保留）先于 compactor（全局摘要）跑，便宜手段先用。我们现在一刀切摘要；加一层 per-result 截断可显著减少摘要触发。
3. **turn 级预算 reminder 注入**（与 Codex `reminder_interval` 同款，双源互证 S8）——DSH 的 goal 轮驱动 + repeat-reminder（3/5/8 阈值提醒模型自纠）证明"给模型看的预算信号"和"给人看的 HUD pill"是互补的两面。
4. **minimal preset 思想**（L1-L4 阶梯的 agent 面）——按会话组合能力集（standard/code/cordis/minimal 四档），"能力是选择不是全局"。我们的 per-Agent enabled_tools 已是雏形；MA-2 任务台账时可升格为"任务类型→预设能力档"。
5. **headless 入口**（session_runner 的免费红利）——DSH 证明 base 层与入口解耦后 headless 只是薄壳。我们 session_runner 已抽内核，CLI 一次性执行入口可进远期台账（自动化/CI 场景）。
6. **配置即文档的 dump-config**——一条命令 dump 实际生效插件树；我们的 agent.yaml 未来若有继承/分层，同款 `--dump` 是排障刚需。

### 结构性更强（承认差距，不追）
- **OS 级进程沙箱三后端**（bwrap/Landlock/Seatbelt）——与 Codex 同级的五年工程，单机桌面场景不追，判断同 Codex 篇。
- **workflow/ralph 编排引擎**——worker-thread JS 编排脚本 + fresh-agent 迭代循环，重型编排设施；我们 MA 图协作走轻路线。
- **extensions 自指修改**——agent 写插件改自己运行时，激进权限面（对比我们 proposal guardrail 是反向的保守设计）；观察其安全事件再议。
- **Cordis 万物插件化**——框架复杂度对桌面单体是负资产；我们的"Rust 分层单体 + MCP/yaml 扩展"对终端用户是更对的形态。

### 不借鉴（反面守则）
- **一切皆插件的学习成本**——50 包 × profile/bundle/patch 三层组合，可组合性换可理解性；桌面用户不可调试组合爆炸。佐证我们"配置放置阶梯"是对的：给用户 yaml 而非插件 API。
- **多真相源风险**——settings/credentials/profile/patch 四层配置各自可覆盖，排障需 dump 全树；我们单 SQLite + agent.yaml 的配置面保持单真相源。
- **遥测默认挂载**（虽 DISABLED）——本地优先立场不挂载不上报，勿学其"装而不用"。

## 四、引入成本（若做对应借鉴项）

| 项 | 实现面 | 量级 |
|---|---|---|
| MA-1 委派 seam 化 | delegate.rs 升格时后端 trait 化（本地会话实现一个） | 小（设计期顺路，MA-1 原估不变） |
| 工具结果 pruner | tool_executor 返参后加保形截断层（阈值/头尾参数进 L1 好默认） | 小 |
| 预算 reminder 注入 | 预算余量 <10% 时 MemoryStage 注入固定文案 | 极小（与 Codex 篇 S8 条目合并实施） |
| 能力档预设 | enabled_tools 之上加命名档位（勘察=只读 / 标准 / 全量） | 小-中（MA-2 顺手） |
| headless CLI | session_runner 加 CLI bin 入口 + 无窗日志输出 | 中（远期） |
| agent.yaml --dump | 配置解析器加 dump 出口 | 极小 |

## 未证实存疑清单（引用前须核）

- Agent Teams（`ctx.agentTeams`）标注 experimental/private opt-in——成熟度未知，MA-3 参照时须再看其演进。
- `dsh-llm-pi-ai` 多 provider 适配的休眠/激活细节仅读配置注释，未跑通验证。
- extensions 自指工具集的实际权限边界（是否有 guardrail）未深入源码——与我们 proposal 系统对比结论可能随版本变化。
- 版本快照 v0.1.0-rc.8（developer preview，明示将有破坏性变更）；本篇条目有效期建议 3 个月。

## 来源

**源码一手（本地 checkout，最高置信）**
- 本机完整仓库：README.md / docs/architecture.md / packages/README.md
- packages/bundle/base/cordis.patch.yml（451 行全文）/ packages/bundle/web-app/cordis.patch.yml（445 行全文）
- apps/cli/config/agent-presets/{standard,minimal,code,cordis}/agent.cordis.yml（全文）
- packages/examples/agent-spine-demo（最小脊柱清单与 README）
- 包清单：core/llm/subprocess/shell/terminal/sandbox/fs/lsp/skill/compaction/subagent/jobs/workflow/web/session/session-query/settings/credentials/extensions/hooks/acp/sdk 等组 README 标题与定位

**官方渠道**
- https://github.com/deepseek-ai/deepseek-harness （README / CONTRIBUTING / docs/）
- npm `@deepseek-ai/dsh`（分发面）
- Cordis：https://github.com/cordiverse/cordis 及其论文《A Programming Paradigm for Spatiotemporal Composability》
