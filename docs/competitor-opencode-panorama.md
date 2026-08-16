# 竞品研读 04 — OpenCode 全景（架构 / 技术实现 / 设计理念 / 功能盘点）

> 借鉴拍第四份（2026-08-16）。四问框架：它是什么、解决什么问题 / 靠什么架构 / 我们要不要 / 引入成本。
> **只积累不实施**——候选项进台账借鉴拍，动工须用户拍板。
>
> 置信纪律同 [01b](competitor-claude-code-panorama.md) / [02](competitor-codex-panorama.md) / [03](competitor-openclaw-panorama.md)：【官方文档】opencode.ai/docs 18 页全读 + 仓库 README + 团队公开表述（HN/Baseten·Sulat 访谈逐字稿）> 【源码分析】**本份空缺**（GitHub raw 在研读环境不可达，架构线未做源码级核验，涉内部实现处已降档标注）> 【社区观察】HN 两波（2025-06 发布 + 2026-07 批评）多源交叉。两代理素材合成时已收紧：Zen 订阅价讹传弃用、收购/背书类单源剔除。

## 一、它是什么、解决什么问题

**定位**：开源 AI coding agent（TS/Bun），**client/server 分体是自称的核心差异**——`opencode` 单命令 = 本地 server + TUI thin client，`opencode serve` 起无头 HTTP server，TUI 只是客户端之一。7 形态共享同一内核：TUI（主力，自研 OpenTUI/Zig）/ Web / Desktop(beta) / IDE 扩展 / ACP（Zed）/ GitHub Action / JS SDK。**197,940 stars**（GitHub API 2026-08-16 实测；SST 团队出品，org 已更名 Anomaly/anomalyco）。【官方文档】

沿革与命名争议：2025-04 Kujtim Hoxha 基于 Charm 全家桶建 TermAI→OpenCode；Dax Raad/Adam 深度加入并买下域名；Kujtim 接受 Charm 收编把仓库（连 star）迁走，Dax/Adam fork 保留名字，Charm 版改名 crush 让名。多源交叉记录为公开争议。【社区观察】

**公开表述的设计公理**（团队原话可溯源，编号对应文末原话集）：
1. **Provider 中立是存在理由**——"We basically took Claude Code and did an open source version of it… we're not locked to a specific provider."（#9/#20）；75+ provider 经 models.dev 元数据单源接入
2. **开源是手段不是信仰**——"Just because something's open source doesn't mean it's going to be any better… It only happens when there's a long tail of things to cover."（#7）；甚至反向论证 Claude Code 无需开源（"there's no problem that Claude Code has that a large open source effort would solve"）
3. **UX 优先于模型性能**——"we're very focused on UX and less so on LLM performance. we use all the same system prompts/config as claude code"（#3）；"we reverse engineered Claude Code and re-implemented almost the exact same logic… people are perceiving all these differences"（鸽子迷信论：模型随机性让聪明工程师做占星术）——**感知差异来自 UX 而非模型**
4. **client/server 为多前端而设**——"the goal is to build alternative frontends, mobile, web, desktop, etc"（#1）；"OpenCode is a core that should be embeddable everywhere"
5. **反馈回路自动化优于模型智力**——LSP 诊断即时回灌（"edit tools return errors and the LLM immediately fixes them"）解释了同 prompt 下体感优于 Claude Code（#4）
6. **商业模式 = double miracle 金字塔**——开源基底永不付费化（红线原话 #18），收费 = Zen 网关（breakeven 哲学 #11）+ Go 订阅 + 企业 control plane；"The size of the top is a function of how big the base is."（#19）
7. **反 benchmark 立场**——"if a company says we're number one on this benchmark, they are bullshit"（#13）；自有评测 = "anecdotes over data" + 私有真实任务集

## 二、靠什么架构（子系统盘点）

### 1. 工程底座
- Bun/TypeScript monorepo（bun workspaces）；server 默认 `127.0.0.1:4096`，`OPENCODE_SERVER_PASSWORD` basic auth，mDNS 发现【官方文档】
- **OpenAPI 3.1 spec 挂在 `/doc` = SDK 单一真相源**：官方 JS SDK 类型全部由 spec 生成【官方文档】
- 实时事件：**SSE 单总线**（`/event`），事件命名空间统一前缀（server./session./message./tool./file./lsp./permission./tui.）——进程内总线 + 各自持久化，**非 append-only 唯一真相源**【官方文档】

### 2. Agent loop 与终止治理（对 IcePaw 最有参考价值的子系统）
- **三件套**：① agent `steps` 上限——触顶后**注入「仅文本总结」system prompt 强制收尾**（不是硬终止）② `doom_loop` 权限——同一工具+相同输入连呼 3 次即触发，**默认 ask** ③ `--auto` 全自动批 ask（deny 仍生效）【官方文档】
- 结构化输出：`format: json_schema` → 内部 `StructuredOutput` 工具 + 验证失败自动重试（retryCount 默认 2）【官方文档】
- 消息级回滚 `session.revert`/`unrevert`（配快照，见 §4）【官方文档】
- 子 agent 工具 `task`；运行中介入：Esc 中断 + 排队输入【官方文档】

### 3. 上下文管理
- compaction = **隐藏系统 agent**（`/compact` 手动 / 别名 `/summarize`）；三旋钮：`auto`（满自动压）/ `prune`（剥离旧工具输出）/ `reserved`（预留 token 缓冲）【官方文档】
- **压缩策略可编程覆写**：插件 hook `experimental.session.compacting` 可注入附加上下文或**整体替换压缩提示词**【官方文档】
- 图片治理：`attachment.image` auto_resize（上限 2000×2000）+ max_base64_bytes 5MB；超限工具返图省略、用户图报错【官方文档】
- `small_model`：标题生成等轻任务走便宜档（Haiku/Nano/Flash 级）【官方文档 + Zen 页】
- **prompt cache 击穿是社区最大痛点**（AGENTS.md 每 SSE 轮重读 + 系统 prompt 内日期跨午夜全量 miss）；V2 方案 = 系统提示组件化定义避免 cache busting（"providers are supporting this as a native concept"）【社区观察 + 团队表述】

### 4. 会话与状态
- 三层模型：session / message / **part**（消息部件 = 流式更新最小单位，SSE `message.part.updated`）【官方文档】
- **快照系统 = 内部 git 仓库**跟踪文件变更支撑 `/undo` `/redo`；要求 git repo、默认开、大仓库可关【官方文档】（实现细节未源码核验）
- 持久化：凭据 `~/.local/share/opencode/auth.json` + MCP OAuth `mcp-auth.json`；**会话数据文件格式文档未披露**（社区传按 project hash 分目录 JSON——未证实）【官方文档 + 存疑】
- share 匿名只读链接（opencode.ai 托管，manual/auto/disabled 三档）【官方文档】
- 会话导入导出：`export --sanitize` 脱敏 / `import` JSON 或 share URL【官方文档】

### 5. 工具系统
- 内置：bash / edit / write / read / grep / glob / **lsp** / apply_patch / **skill** / todowrite / webfetch / **websearch（Exa 托管 MCP 免 key）** / **question**（多问题+选项+自定义作答问卷）【官方文档】
- grep/glob 内置 ripgrep（尊重 .gitignore）；apply_patch 标记行格式【官方文档】
- **LSP**：30+ 内置服务器默认关；诊断作为 agent 反馈进循环；官方最佳实践反而推荐优先 CLI lint（LSP 有内存与失同步成本）【官方文档】
- MCP：local（command）/remote（url+headers）；远程自动 OAuth（401 触发 + Dynamic Client Registration RFC 7591）；工具拉取超时 5s；glob 批量启停 + per-agent 白名单【官方文档】
- **自定义工具**：`.opencode/tools/*.ts`（Zod args），文件名即工具名，**同名覆盖内置工具**【官方文档】

### 6. 权限模型
- `permission` 三态 allow/ask/deny × 每工具键；**细粒度对象语法**：bash 匹配解析后命令、edit 匹配文件路径、webfetch 匹配 URL、task 匹配子代理类型；glob 通配 + **last-match-wins**【官方文档】
- 默认值：多数 allow；`doom_loop` / `external_directory` 默认 ask；read 默认 allow 但 `*.env`/`*.env.*` **deny**【官方文档】
- ask UI 三选：一次 / **总是（会话级，且工具主动建议安全 pattern 如 `git status*`）** / 拒绝【官方文档】
- **无沙箱/容器**；plan agent = 权限收窄样板（deny edit、bash ask）【官方文档】
- 文本解析 allowlist 可绕争议（`echo git | bash`）——社区辩护共识："命令过滤是 steering 不是 security，真隔离该用 sandbox"；与 Claude Code 同属行业共性问题【社区观察】
- 信任演进时间线：2025-06 发布时几无写权限确认（被点名）→ 中期默认偏松（CC 用户被惊到）→ 现默认有确认弹窗【社区观察多用户交叉】

### 7. 多模型接入
- **Vercel AI SDK + models.dev 元数据**（75+ provider，窗口/定价单一来源）；模型解析四级：`--model` > config > 上次使用 > 内部优先级【官方文档】
- 自定义 provider：config 指定 **npm SDK 包名** + baseURL/headers/models；Ollama/LM Studio/llama.cpp 全走此模板【官方文档】
- OAuth 家族：GitHub Copilot（device flow）/ ChatGPT / GitLab Duo / Snowflake / SuperGrok / DigitalOcean【官方文档】
- **variants**：推理档位（Anthropic high/max、OpenAI minimal~xhigh）+ 键切换【官方文档】
- **Zen 网关**（完全可选）：按量零加价 breakeven、~65 模型、美国托管零保留、teams 免费 beta + 企业管控面【官方文档】
- **Go 订阅**：$5 首月/$10 月，开放模型可靠访问（Beta）【官方页面】

### 8. 扩展机制
- **插件**：JS/TS 模块（本地/npm），**20+ hook 事件**（`tool.execute.before/after` 可改参/抛错拦截、`session.idle/compacted`、`permission.asked`…）【官方文档】
- 命令：markdown frontmatter + `$ARGUMENTS` + `` !`cmd` `` 子任务 + `@file` 注入【官方文档】
- **skills：SKILL.md 渐进披露**——只有 name+description 进工具清单，调 `skill` 工具才注入正文；兼容 `.claude/skills`【官方文档】
- 规则：AGENTS.md（全局/项目），CLAUDE.md fallback；`instructions` 数组支持 glob 与远程 URL；`/init` 扫仓库生成【官方文档】
- 配置 **8 层优先级合并非替换**（remote .well-known → global → env → project → .opencode → 内联 → managed → MDM）【官方文档】

### 9. 多 agent
- **primary 双主角 build/plan**（Tab 随时切换）vs subagent（general/explore/scout）；`@agent` 提及调度；命令 `subtask:true` 强制子代理运行不污染主上下文【官方文档】
- **`subagent_depth` 默认 1**（0=全禁）【官方文档】
- **隐藏系统 agent（compaction/title/summary）复用同一 agent 机制**——"系统任务也是 agent"的统一抽象【官方文档】

## 三、我们要不要（对照 IcePaw 现状）

### 已对齐（四产品交叉验证）
| OpenCode | IcePaw 现状 |
|---|---|
| `subagent_depth` 默认 1 | delegate v2 深度=1——**第三家同构**（CC agent teams / codex max_depth / opencode），结构护栏是行业收敛决策 |
| 系统任务也是 agent（compaction/title 走隐藏 agent） | 内置 MCP 工具（proposal/memory）与外部 server 同一注册表——同一统一抽象 |
| ask UI 一次/总是（会话级） | Once / ThisTool / ThisDir 授权档同族 |
| compaction 独立通道 + small_model 便宜档 | summary_provider 独立通道；P10-② 已登记辅助任务便宜档 |
| `*.env` 默认 deny | reject_sensitive 红线拦硬写凭证同方向 |
| 多 provider + 自定义 baseUrl + 本地模型 | 产品本体；且我们 Anthropic+OpenAI 双协议原生，比其"经 npm SDK 包"更贴协议层 |
| UX 优先哲学（感知差异来自 UX 非 LLM） | 我们的 HUD/pill/透明度路线获强佐证——他们用同模型同 prompt 重实现后用户仍感知"更好" |
| 状态透明（stats 按天/工具/模型/项目） | 预算 pill 同族，维度可参考其 stats 分组 |

### 要借鉴（进台账 / 喂已有条目）
1. **steps 上限「仅文本收尾」喂 S8 终止语义**——触顶后注入 system prompt 让模型**输出总结再停**，而非 budget_exceeded 硬终止。给 agent 一次「收尾发言权」：用户看到的是一段交代而非一行错误。与 codex reminder 注入（事前自管理）组成前后两半。
2. **doom_loop 同参重复检测补 stuck_detect 信号维度**——同工具+相同输入连呼 3 次=最强卡死信号，比轮数/时间启发式精准；他们做到权限层（触发即 ask），我们 stuck_detect 已有熔断位，加此信号即可。
3. **ask 时「工具主动建议安全 pattern」喂 P8**——审批弹窗里工具附一个收窄 pattern（`git status*`）让用户一键放行——治审批摩擦的最小交互（codex「审批摩擦=安全威胁」论证的 UX 解法）。
4. **权限细粒度对象语法喂 P8**——bash 匹配**解析后命令**（非原文）、edit 匹配**路径**、webfetch 匹配 **URL**：每个工具声明自己的鉴权维度，比单一 AuthorizationLevel 更贴工具语义；last-match-wins 的规则序语义可参考。
5. **skill 渐进披露**——name+description 常驻、正文按需注入：我们工具软裁剪已做相关性排序，「文档型内容只进目录不进正文」可延伸到 KB/help 体系（观察池，撞上再做）。
6. **压缩提示词可被插件整体替换**（`session.compacting` hook）——我们 hooks 系统的四接入点可远期加 BeforeCompact；先积累。

### 结构性更强（承认差距，不追）
- **LSP 集成与诊断回灌**（30+ 服务器）——coding agent 反馈回路核心；但非 IDE 编码工具错位，且官方自认成本（内存/失同步，推荐 CLI lint 优先）说明这项贵。
- **client/server 多前端矩阵**（7 形态 + OpenAPI codegen）——规模错配；我们 Tauri 单壳，无跨端 RPC 需求。
- 自研 OpenTUI（Zig）终端框架 + Zen/Go 商业体系 + models.dev 生态位——不可复制的产品时势。

### 不借鉴（反面守则）
- **无 append-only 事件日志**（SSE 进程内总线 + 各自持久化）——**第四家反证**：CC（快照）、codex（JSONL+SQLite 双源对账债）、openclaw（无日志）、opencode（总线非真相源）四家没有一家同时做到「append-only + 单一真相 + 可派生」；session-event-log 是我们的差异化。
- **快照式回退**——undo/redo 靠内部 git 仓库（要求 git repo、大仓库要关）：CC checkpoint 污染 git 历史之后**第二次**快照式回退的债证；P9 事件派生 rewind 第三次被佐证。
- **文本解析 bash allowlist**——"steering 不是 security"的行业教训：授权判据要走结构化信号（我们的 AuthorizationLevel + workspace 校验路线），勿走文本解析。
- **系统 prompt 臃肿 + 强观点泄漏**——"ABSOLUTELY NO COMMENTS" 泄漏进子 agent 派发、用户只能改源码（社区批评）；系统提示的分层与子代理继承控制要刻意设计。
- **信任默认值演进教训**——发布时几无写确认→被骂→补默认弹窗：**权限默认值是产品信任的第一天决策**；IcePaw 的 ask 默认 + proposal 审批从第一天就走严格路线，保持。

## 四、引入成本（若做对应借鉴项）

| 项 | 实现面 | 量级 |
|---|---|---|
| S8 仅文本收尾 | loop_engine budget_exceeded 路径改为注入收尾 prompt + 放行最后一轮（无工具） | 小-中（并入 S8 终止语义重排） |
| stuck_detect 同参信号 | 工具调用序记录 (tool, args_hash)，3 连同即触发现有熔断 | 小 |
| P8 建议 pattern + 细粒度对象语法 | tool_executor 授权数据结构加 pattern 字段 + 各工具声明鉴权维度 | 中（P8 原估内升级设计） |
| skill 渐进披露 | KB/help 注入管道加目录层 | 观察（撞上再做） |
| BeforeCompact hook | hooks 系统加第 5 接入点 | 小（远期） |

## 未证实存疑清单（引用前须核官方）

- **会话持久化文件格式/路径**——文档未披露（社区传按 project hash 分目录 JSON + parts 追加写），源码不可达未核验
- "16M developers every month"（官网自称）——口径与统计方法无文档
- Zen "$9.99/mo Pro / 100 free requests per day"——第三方讹传，官方仅 pay-as-you-go，**弃用**
- OpenCode Go 具体限额（~$4.50/5h 滚动窗等）——第三方评测转述，官方仅 "generous limits"
- Fortune 500 / Meta 以其为内部 CLI 基座——HN 单源
- 免费 Zen 模型可无鉴权从任意客户端调用——两用户独立实测但未文档化，随时可能收紧
- Pragmatic Engineer MAU 完整数字——付费墙截断
- Anthropic OAuth「复用 Claude Code client ID」——单源 + 间接印证，机制无官方确认
- README 与 docs 子 agent 清单不一致（general vs general/explore/scout）——以 docs 为准记文档漂移
- 仓库归属 org 更名时间线（sst→anomalyco 重定向）——未深究

## 来源

**官方文档 / 仓库一手**
- https://opencode.ai/docs/ 全 18 页（intro/server/sdk/agents/tools/permissions/providers/models/tui/config/plugins/lsp/mcp-servers/skills/commands/custom-tools/rules/zen）
- https://opencode.ai/ + /zen + /go ；https://github.com/sst/opencode （README，重定向 anomalyco）+ GitHub API（stars/license 实测 2026-08-16）
- 团队公开表述：Dax Raad HN 作者评论 https://news.ycombinator.com/item?id=44482504 ；thdxr/k-langton/jayair https://news.ycombinator.com/item?id=48978112 ；Zen 发布推文 x.com/thdxr/status/1967705371117814155

**作者一手（第三方刊载逐字稿）**
- Baseten 访谈（2025-10）https://www.baseten.co/blog/building-ai-agents-open-code-and-open-source-a-conversation-with-dax/
- Sulat 转录 Nuno Maduro 直播访谈（2026-01）https://ai.sulat.com/opencodes-creator-on-model-freedom-anthropic-blocks-and-the-double-miracle-of-open-source-bd94bd8fc763

**源码分析**：无（GitHub raw 研读环境不可达，未做源码级核验——诚实声明）

**社区观察（多源交叉后降档采用）**
- HN 44482504（2025-06 发布，61 评）+ 48978112（2026-07 批评，420 分/289 评）
- OpenRouter 除名 issue #11926；命名争议（HN 44741894 转引 + 双方推文）
- Pragmatic Engineer 访谈（付费墙片段）https://newsletter.pragmaticengineer.com/p/opencode
