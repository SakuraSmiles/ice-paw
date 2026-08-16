# 竞品研读 03 — OpenClaw 全景（架构 / 技术实现 / 设计理念 / 功能盘点）

> 借鉴拍第三份（2026-08-16）。四问框架：它是什么、解决什么问题 / 靠什么架构 / 我们要不要 / 引入成本。
> **只积累不实施**——候选项进台账借鉴拍，动工须用户拍板。
>
> 置信纪律同 [01b](competitor-claude-code-panorama.md) / [02](competitor-codex-panorama.md)：【官方文档】docs.openclaw.ai + 仓库一手 + 作者博客 > 【作者一手】Steinberger 本人表述 > 【源码分析】GitHub API 元数据 > 【社区观察】HN/Reddit/安全厂商。两代理素材合成时已收紧：恶意插件数量等单源结论剔除、收购传闻证伪、官墙策展背书降档。

## 一、它是什么、解决什么问题

**定位**：消息渠道优先的 **always-on 个人助理**（非 coding agent）——自托管 Gateway 常驻进程，把 WhatsApp/Telegram/Slack/Discord/Signal/iMessage 等 10+ 渠道接到 AI agent。383K stars（5 个月，GitHub 史上最快），一人 vibe-coded 起家（作者语录 "Every single line is generated - or rewritten - by codex"），2026-02 作者加入 OpenAI、项目归 OpenClaw 基金会保持开源独立。【官方文档 + 作者一手】

沿革：Warelay（WhatsApp 网关）→ Clawd/Clawdbot →（Anthropic 商标邮件）→ Molty → OpenClaw（2026-01-30 三小时全迁）。lore 页自曝两起事故（`find ~` 目录清单倒进群聊 / 购物车被下 $74,500 机器狗）——**自曝事故史本身是产品设计：用幽默建立风险透明度**。

**公开表述的设计公理**：
1. **单操作者信任域**——"designed for a single operator"，一个 Gateway 一个信任域；"Most failures are not exotic exploits - they are 'someone messed the bot and the bot did what they asked.'"
2. **防御优先级：Identity first → Scope next → Model last**——先管谁能在说话，再管它能碰什么，最后才是模型层注入防御
3. **无隐藏状态**——"The model only remembers what gets saved to disk; there is no hidden state"（记忆全 markdown 落盘）
4. **workspace 即人格**——配置 = 一个 json5 + 一堆 markdown（AGENTS.md 管规则 / SOUL.md 管声音："Short beats long. Sharp beats vague."）
5. **涌现即能力**——"give them access to your system and they'll just figure things out"（语音转写链路 ogg→ffmpeg→whisper 是模型自己发现的，作者从未编程）
6. **单一 Gateway 单写者**——多端（Control UI/CLI/TUI/手机 node）全是客户端，Gateway 是会话与工具的唯一所有者

## 二、靠什么架构（子系统盘点）

### 1. 工程底座
- TypeScript + Node 22+，pnpm monorepo；Gateway 守护进程 + typed WS API（127.0.0.1:18789）；**协议单一来源 codegen 到 JSON Schema + Swift + Kotlin**（四端类型一致 + coverage 护栏）【官方文档】
- 工程纪律重：import-cycle 检查、插件 SDK 边界检查、SQLite 事务边界检查、max-lines 棘轮、deadcode、数十个 Docker e2e（onboard 旅程/升级 survivor/多节点更新/插件市场）【官方文档】
- 原生伴侣 app（macOS 菜单栏 Swift / iOS / Android Kotlin）+ node 节点设备（手机/电脑配对贡献相机/屏幕/exec 能力）【官方文档】

### 2. Agent loop 与常驻工程
- **每会话串行 lane + 全局队列**；入队四模式：steer（插队改写当前任务）/ followup / collect / interrupt【官方文档】
- **持久写者锁**：`activeWriterRunId` 持久领取 + `expectedWriterRunId` fencing 栅栏在同步 commit 事务里校验，防并发写者；SQLite writer 队列 + 状态目录锁（单实例）【官方文档】
- always-on 三件套：**Heartbeat**（默认 30min 完整 agent turn 巡检，无事回 `HEARTBEAT_OK` 压缩汇报；skip 有官方词表）/ **NO_REPLY 静默 token**（模型可选择不外发）/ 卡死诊断 + bounded metadata-only 审计账本【官方文档】
- 超时：agent 运行默认 48h / 模型空闲 Cloud 120s·自托管 300s【官方文档】
- **外部 harness 复用而非自造**：codex app-server / copilot / claude-cli 可作为 agent runtime（ACP 协议接入）【官方文档】

### 3. 上下文与记忆
- 系统提示每 run 重建；工作区引导文件分节注入（AGENTS/SOUL/IDENTITY/USER/BOOTSTRAP.md；单文件 20K、总量 60K 限额）；技能只常驻元数据、正文按需读；`/context list|detail|map` 可观测【官方文档】
- **compaction ≠ pruning**：pruning 只在内存裁 tool_result，transcript 磁盘**原样保留**——与 IcePaw 日志无损不变式同构【官方文档】
- 压缩工程（S8 级参照物）：自动压缩匹配 provider overflow 错误触发 + 手动 `/compact [instructions]`（keepRecentTokens 20K）；**压缩模型可独立配置**（如 ollama 小模型）；**压缩前静默记忆轮 memoryFlush 先落记忆**（可另配小模型）；safeguard 质量护栏（校验摘要结构，坏则纠正重试）；可插拔压缩 provider【官方文档】
- 长期记忆：markdown 体系（USER.md 用户画像 / MEMORY.md 精炼 / memory/日期.md 日记自动载入今天+昨天 / DREAMS.md 梦日记待人审）+ memory_search 向量关键词混合检索 + **dreaming**（默认开 cron 后台固化：阈值晋升 + taint 污染门控 + 产物人审）；可一键导入 Claude Code / Codex / Hermes 记忆【官方文档】

### 4. 会话与状态
- 按 来源路由：DM 共享 main 会话、群组隔离、cron 每次新鲜；reset 模式 none/daily/idle【官方文档】
- 每 agent 一个 SQLite（会话行/transcript 行/token 用量）+ 归档 transcript 工件；pruneAfter 30 天 / maxEntries 500；incognito 仅内存【官方文档】
- 多设备 = 多客户端接同一 Gateway（无副本同步）；备份迁移走命令行【官方文档】
- **无 append-only 事件日志 / 对账系统**（文档未见对应物）——可解释性弱于 IcePaw

### 5. 工具系统与 MCP
- 内置工具面按 profile（messaging/coding/minimal）；exec / 文件 / browser（playwright）/ apply_patch / message（跨渠发消息）/ automations / memory 组 / canvas【官方文档】
- **MCP 双向**：作为 server（`openclaw mcp serve` 把 Gateway 会话暴露给外部 agent：conversations_list / messages_read / events_poll / permissions_respond 等工具，带 Docker 冒烟测试）；作为 client 注册表（stdio/sse/streamable-http、OAuth loopback 自动回调 + SQLite lease 防并发刷 token、mTLS、per-server toolFilter glob、resources/prompts 自动生成工具、失败短暂熔断、空闲 10min 回收）【官方文档】
- **MCP Apps 扩展**：ui:// 资源渲染（2MiB 上限、双 iframe 沙箱源、opaque origin、CSP、10min 内存 view lease）【官方文档】

### 6. 权限与安全（全站最重投入）
- 入口三层：**DM pairing 配对码**（8 位排混淆字符、1h 过期、≤3 pending）/ 双层 allowlist（DM + 群组发言人级）/ 节点设备配对（bootstrapToken 10min）【官方文档】
- **exec 五档**：deny / allowlist / ask / auto / full（默认宿主 full、沙箱内 deny；askFallback 默认 deny）；strictInlineEval 拦 `python -c` 类内联求值；per-agent glob + argPattern 白名单【官方文档】
- 三控制点原话："Sandbox decides where tools run; Tool policy decides which tools available; Elevated is an exec-only escape hatch."——沙箱 opt-in（Docker network:none + readOnlyRoot + capDrop ALL + bind mount 黑名单 .aws/.ssh 等）+ tools.elevated 逃生舱（"All gates must pass"）【官方文档】
- 注入防御：不可信内容 `<EXTERNAL_UNTRUSTED_CONTENT>` 包裹 + chat-template 特殊 token 剥离 + **出站 sanitizer 剥离泄漏的 `<tool_call>`/system-reminder**【官方文档】
- **日志/transcript 脱敏永开不可关（硬编码不变式）**；`openclaw security audit` 一键配置体检；凭证治理（workspace .env 阻止 provider 凭证变量）【官方文档】
- 审批 UX：审批卡**原生长在渠道里**（按钮卡片）+ `/approve` 兜底 + 审批转发别的房间；**拒绝结果回贴会话（agent 能看见被拒不重试）**【官方文档】

### 7. 流式与富输出（被渠道宿主逼出来的节流语义）
- **无真 token 流式**（渠道消息不可 token 级刷新）："no true token-delta streaming today"。两层替代：Block streaming（完成块独立消息）+ Preview streaming（临时预览消息边生成边编辑，TG/Slack 默认 progress、Discord 默认 off）【官方文档】
- 防刷屏：coalesce 合并（minChars 1500 + idleMs 1000 空闲 flush）+ humanDelay 块间 800-2500ms 随机拟人停顿；**代码围栏内永不切分、必须切时先闭合再重开围栏**【官方文档】
- **慢工具 5s 定时器才出行**（"web_fetch arms a five-second timer when it starts"）；进度草稿 maxLines=8、每行 120 字符截断【官方文档】
- 富输出：结构化 `mediaUrl`/`mediaUrls` 字段优先，`MEDIA:` 行是 legacy 文本约定且自认脆弱（正迁往结构化）；远程附件必须公网 https（内网/回环一律拒）【官方文档】

### 8. 多模型与多 agent
- Provider **全部是插件**（registerProvider 拥有目录/鉴权/传输/故障转移）；官方面覆盖 anthropic/openai/google/**zai glm-5.2**/github-copilot/openrouter/ollama/lmstudio/vllm/deepseek/minimax/kimi…；多 key 轮换（`PROVIDER_API_KEYS` 多值）；failback 模型链；自定义 provider 任意 baseUrl + api 双协议【官方文档】
- 多 agent = 多隔离边界：每 agent 独立 workspace + SQLite + auth profiles；bindings 渠道路由（最具体优先）；**a2a 通讯默认关**；每 agent 差异化策略（官方示例：个人 agent 全权限 / 家人 agent 沙箱只读 / 公共 agent 无 fs/shell）【官方文档】
- 子 agent：sessions_spawn/send 委托（allowAgents 白名单 + `sandbox: "require"` 强制沙箱护栏）；子 agent 会话只注入 AGENTS.md；sessions_history 有界脱敏【官方文档】

### 9. Automations（定时任务）
- Gateway 进程内调度器 + SQLite 持久化；三入口：CLI / 聊天 owner-only `/loop [interval] <prompt>`（agent 可 1min-1h 自调速）/ agent 侧 automations 工具【官方文档】
- 5 触发（at/every/cron/on-exit/stream-command）× 4 payload（system-event 不调模型 / message / command / script）；整点自动错峰 ≤5min【官方文档】
- 护栏：**agent 创建的任务工具集钳制在创建回合可用范围内不可放宽**、连续 10 败自动禁用、退避 30s→60s→5m→15m→60m【官方文档】

## 三、我们要不要（对照 IcePaw 现状）

### 已对齐（三产品交叉验证 + 镜像互补）
| OpenClaw | IcePaw 现状 |
|---|---|
| 配置 = 一个 json5 + 一堆 markdown；状态走 `/usage` `/status` 聊天命令 | 配置放置阶梯同构（L3 yaml ≈ 其 workspace 文件、L2 HUD ≈ 其聊天命令、L4 表单 ≈ 其 onboard 向导）——该哲学撑起了 38 万星产品 |
| Gateway 单写者（所有客户端经它） | 「会话单一写者」反面守则同源；我们目前 ChatState 内存级，他们做到**持久 fencing**（见借鉴 #2） |
| pruning 只裁内存、transcript 磁盘原样 | 日志无损不变式同构（第三家同构：CC/codex/openclaw 全部保原始态） |
| compaction 可独立配置便宜模型（ollama）做摘要 | 我们 summary_provider 已有独立通道（自适应额度）；方向一致，他们多了质量护栏与记忆 flush |
| automations 工具钳制不可放宽 | 与 proposal「agent 全程无写权限」同一信任边哲学 |
| usage 透明（context ring / /usage / 菜单栏） | 预算 pill（V1）同一判断：用量是状态不是配置 |
| 拒绝结果回贴会话（agent 可见不重试） | 我们 tool_auth 拒绝走 tool_result 回传——等价闭环 |

### 要借鉴（进台账 / 喂已有条目）
1. **压缩工程三件套喂 S8**——①压缩前 memoryFlush 静默记忆轮（先落重要信息再折叠）②safeguard 摘要质量护栏（校验结构、坏则纠正重试，比我们的「熔断 10min」多了**修复**而非仅放弃）③keepRecentTokens 式热尾参数化。S8 设计已含热尾，前两项是新增输入。
2. **持久写者锁 fencing 喂 MA-3**——`activeWriterRunId` + `expectedWriterRunId` 在 SQLite 事务内校验，是「会话单一写者」守则的持久化实现样板；MA-3 持久通道/多客户端时代的直接抄写对象。
3. **慢工具 5s 进度行阈值 + 进度草稿上限**（maxLines=8 / 120 字符行截断）——聊天 UI 节流的成熟参数包，IcePaw 工具运行中状态展示（前端打磨）可直接参考。
4. **记忆可迁移性是获客杠杆**——OpenClaw 一键导入 Claude Code 记忆直接降切换成本；反向启示：IcePaw 的会话/轨迹 JSONL 导出（已有 export_session_trajectory）可加「Claude Code 兼容格式」观察项。
5. **usageTemplate 式页脚定制进 L3**——把用量话术开放成配置 DSL 走得更远，验证「状态展示也可进 yaml」；我们 L2 pill 已覆盖主场景，仅作阶梯边界的参考案例。

### 结构性更强（承认差距，不追）
- **渠道矩阵工程**（20+ 渠道插件 + 各渠道富卡片/语音/配对细节）——消息渠道不是我们的宿主，错位竞争。
- **always-on 工程全套**（heartbeat / NO_REPLY / 节点设备能力 / 48h 超时体系）——产品形态不同（我们按回合对话）。
- **协议 codegen 四端类型一致 + ClawHub 生态**——规模错配。
- 一人 vibe-code 五个月 38 万星的发行速度——不可复制的时势（Anthropic 商标事件流量 + 作者影响力），非工程方法。

### 不借鉴（反面守则）
- **无事件日志/对账**——会话可解释性弱于我们；三产品对比下我们的 session-event-log（单库 append-only + derive 对账）是**差异化优势**：比 CC 的快照干净、比 codex 少一个真相源、比 openclaw 可回放。反向确认此投入正确。
- **工程质量让位于发行速度**——社区批评 "very very vibe-coded" + 安全焦虑主旋律（HN "running it scares the crap out of me"）+ 第三方安全厂商连环报道——权限面大 + 纪律松的组合是安全叙事的反面教材；我们的 proposal guardrail + 事件审计是刻意更严的路线。
- **`MEDIA:` 行文本协议的脆弱 legacy**——**协议第一天就结构化，勿走文本约定再迁移**（他们正在付这笔债）；对我们 MCP 工具输出/事件 payload 设计是提醒。
- **agent 改配置走 owner 权限直接改**——无提案通道；我们 propose_config_change 的审批卡片更严，保持。
- 作者本人 `--yolo` 姿态（"nothing bad ever happened... for 6 months"）——幸存者偏差的示范；产品把严格做成默认推荐是对的。

## 四、引入成本（若做对应借鉴项）

| 项 | 实现面 | 量级 |
|---|---|---|
| S8 压缩三件套（memoryFlush / safeguard / 热尾参数化） | MemoryStage 前置记忆钩子 + 摘要结构校验纯函数 + 重试一次 | 中（并入 S8 原有范围） |
| MA-3 写者 fencing | 会话表加 writer_run_id 列 + 事务内校验 | 中（MA-3 时代） |
| 慢工具进度行 5s 阈值 | 前端工具运行态组件 + 定时器 | 小（打磨拍顺手） |
| 导出 Claude Code 兼容格式 | export 命令加目标格式适配 | 小（观察池） |

## 未证实存疑清单（引用前须核官方）

- 「ClawHub 近 3000 工具中 341 个恶意」——Facebook 群组单源，**剔除**（仅记录存在此说法）
- 「数千暴露实例」——Bitsight 等安全厂商博客（利益相关：卖攻击面管理），量级无官方确认
- 「OpenAI 收购 OpenClaw」——Twitter 传闻，**证伪**（一手事实 = 作者个人加入 OpenAI + OpenAI 赞助 + 项目归基金会独立）
- 许可证——README 称 MIT 但 GitHub API license=NOASSERTION，须看 LICENSE 原文
- 「52+ modules 单进程近无限权限」——NanoClaw 作者第三方技术指控，未做源码统计
- 名人背书（Satya/Sam/Elon）——均出自官方策展墙转述，未在本人一手渠道核验
- CNCERT 弱默认警告——经 thehackernews 转述，未取得一手公告

## 来源

**官方文档 / 仓库一手**
- https://github.com/openclaw/openclaw （README）+ https://api.github.com/repos/openclaw/openclaw （元数据 2026-08-16）
- https://docs.openclaw.ai/ ：start/lore · start/openclaw · install · concepts/architecture · agent-loop · agent-runtimes · context · compaction · session · memory · multi-agent · model-providers · soul · usage-tracking · streaming · reference/rich-output-protocol · gateway/security · gateway/sandboxing · tools/exec-approvals · automation/cron-jobs · plugins/architecture · cli/mcp · help/faq · help/faq-first-run · clawhub

**作者一手**
- https://steipete.me/posts/2026/clawdbot （起源/工作哲学/语录）
- https://steipete.me/posts/2026/openclaw （加入 OpenAI + 基金会宣言）

**社区观察（多源交叉后降档采用）**
- HN：46850205（NanoClaw 及安全讨论）· 46848552 等；Reddit：r/LocalLLM 1qp0jhl · r/AgentsOfAI 1qp49iq · r/selfhosted 1qupw04
- Bitsight / thehackernews / Immersive Labs 安全报道（利益相关降档）
- https://openclaw.ai （官方策展推荐墙——引言真实但经策展）
