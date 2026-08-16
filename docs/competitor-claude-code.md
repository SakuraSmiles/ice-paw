# 竞品研读 01 — Claude Code：上下文压缩与续跑机制

> 借鉴拍第一份（2026-08-16）。四问框架：解决什么问题 / 靠什么架构 / 我们要不要 / 引入成本。
> 整产品全景见 [01b](competitor-claude-code-panorama.md)（架构/技术实现/设计理念/功能盘点）。
> **只积累不实施**——S8（无限续写）的设计输入，动工须用户拍板。
>
> 来源置信度三档（较 agent 原始报告收紧——逆向博客一律不标「官方」）：
> 【平台文档】platform.claude.com / docs.anthropic.com（描述的是服务端 Compaction API，非 Claude Code 客户端全部内部）
> 【源码分析】社区对 Claude Code 源码/反编译的逆向（HarrisonSec / Barazany / Decode Claude——质量高但非官方承诺）
> 【社区观察】GitHub issue / 博客推断

## 一、它解决什么问题

有限上下文窗口 vs 无限长任务的矛盾，拆成五个子问题分层求解：
1. 单条工具输出就能撑爆窗口（输出在产生时就要治理）
2. 历史工具结果累积膨胀（最便宜的先压）
3. 接近窗口上限时的整段折叠（不破坏底层真相）
4. 折叠本身要调 LLM，会失败（失败得有熔断与退路）
5. 任务没完但上下文用尽（续跑语义）

## 二、靠什么架构 —— 5 级渐进管道（按成本升序）

| 级 | 机制 | 关键参数/形态 | 置信度 |
|---|---|---|---|
| L1 | **工具输出预算**（产生时） | 单条 >50K 字符 → 全文落盘 + 上下文只留 2KB 预览 | 源码分析 |
| L2 | **microcompaction**（历史瘦身） | 旧 tool_result 逐条清理/替换为占位；「热尾」= 最近 N 条保持内联；缓存冷/热双路径（cache_edits 保缓存） | 源码分析 |
| L3 | **Context Collapse**（~90%） | 非破坏**投影式**折叠：底层消息不变，查询时叠加摘要，可回滚 | 源码分析 |
| L4 | **auto-compact**（~87%，双余量：输出余量+压缩余量） | LLM 摘要；保留：system prompt **从磁盘重载**（不进摘要）、最近 5 轮原文、todo 状态、工具定义重声明 | 源码分析 |
| L5 | 服务端 Compaction API | `pause_after_compaction` + `stop_reason:"compaction"`、自定义 instructions、触发值 ≥50K | 平台文档 |

摘要生成：两段 CoT——`<analysis>` 按时间线过每条消息 → `<summary>` 结构化 9 段（Primary Request / Key Concepts / Files and Code / Errors and Fixes / Problem Solving / All User Messages / Pending Tasks / Current Work / Next Step）；**analysis 块用后即弃**（推理提质量，但推理本身不占上下文）。【源码分析】

失败处理：
- **熔断器**：连续 3 次压缩失败即停手（`MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES=3`）——源自真实事故：曾有会话连续失败 3272 次、全球浪费约 25 万次 API 调用/天。【源码分析】
- **压缩请求自身超长**：`truncateHeadForPTLRetry()` 递归丢最旧回合让压缩请求装得下。【源码分析】
- **摘要中乱调工具**：instructions 明示「不要调用工具」防 `content:null`。【平台文档】
- 熔断后**无公开回退**——社区建议只剩 `/clear` 或子代理转移。【社区观察】

终止与续跑：
- **无单会话轮数上限**；只有 5 小时/7 天预算制。【社区观察】
- **无公开的「无进展检测」终止**。【社区观察】
- 续行固定模板：「此会话延续自上一个用尽上下文的对话…请从离开处继续，不要再问用户问题」。【源码分析】

边界治理：MCP 输出默认 25K tokens（>10K 警告）、持久化输出最高 500K 字符、单请求 ≤600 图/PDF 页、工具设计官方建议过滤参数+游标分页。【平台文档】

## 三、我们要不要（对照 IcePaw 现状）

**已对齐（无需动）**：
- 熔断器思想 —— 我们的摘要 3 连空熔断 10min 同构（且我们更早落地了自适应额度，先翻倍再熔断，比它多一层）
- 工具结果落盘 —— 我们的 per-round 持久化已等价于它的「全文落盘」
- 投影式非破坏 —— 我们的 session-event-log + 前端投影**本来就是这条路线的泛化**（append-only 真相 + 压缩只作用实时窗口），它用 Context Collapse 局部实现的东西我们有全局地基
- system prompt 不进摘要 —— 我们 system 与历史本就分离

**要借鉴（S8 的四个输入）**：
1. **成本升序分层**：确定性手段（工具结果瘦身→骨架折叠）排在 LLM 摘要**前面**，LLM 是增强不是依赖 → 对应 S8 回退链重排
2. **热尾保留**：折叠保「首 N 轮 + 尾 M 轮原文」，中段压骨架 → 对应 S8 确定性折叠结构
3. **续行模板**：折叠边界放固定续行文案（「从离开处继续，不要再问用户」），便宜且有效
4. **压缩请求自身防超长**：摘要 fold 范围过大时先截头再摘要 → S8 补充项

**我们有机会比它强的一处**：它熔断后**没有回退**（会话搁浅等用户 `/clear`）；S8 的确定性折叠永不失败，正好填这个洞——摘要失败 → 骨架折叠兜底，对话不搁浅。

**不借鉴**：
- 9 段式重型摘要提示词——我们的场景摘要要轻；最多吸收「Files / Pending / Current Work」三要素轻量化
- cache_edits 双路径、pause_after_compaction——Anthropic 服务端专属，我们的 GLM/OpenAI 兼容端点无此物
- 阈值环境变量（CC_COMPACT_THRESHOLD 类）——配置放置阶梯 L1：好默认胜过旋钮

## 四、引入成本（若做 S8）

| 项 | 实现面 | 量级 |
|---|---|---|
| L0 历史工具结果瘦身（预览+指针） | 上下文管道加一个确定性 Stage（纯投影，不碰存储） | 小 |
| L1 确定性骨架折叠 | 同上——投影 Stage；摘要失败/熔断时自动接棒 | 中（核心新逻辑） |
| 终止重排（stuck 主终止、budget 降格失控保护） | loop_engine 终止条件排序 + budget 语义分档（本地/显式上限） | 小 |
| 续行模板 + 折叠边界标记 | 文案级 | 极小 |
| 摘要请求截头防超长 | MemoryStage fold 范围守卫 | 小 |
| 摘要提示词轻量结构化 | prompt 文本 + 观察效果 | 极小（可后置） |

共同前提不变式：**一切折叠只作用于发给 LLM 的投影，session_events 与 legacy 行永不改写**（与产品愿景「日志无损」锁定一致）。

## 来源

- [Compaction — Claude Platform Docs](https://platform.claude.com/docs/en/build-with-claude/compaction)（平台文档）
- [Context Editing — Claude Platform Docs](https://platform.claude.com/docs/en/build-with-claude/context-editing)（平台文档）
- [Context Windows — Claude Platform Docs](https://platform.claude.com/docs/en/build-with-claude/context-windows)（平台文档）
- [The 5-Level Pipeline — HarrisonSec](https://harrisonsec.com/blog/claude-code-context-engineering-compression-pipeline)（源码分析）
- [Compaction Deep Dive — Decode Claude](https://decodeclaude.com/compaction-deep-dive)（源码分析）
- [Compaction Engine Source Analysis — Barazany](https://barazany.dev/blog/claude-codes-compaction-engine)（源码分析）
- [Never Auto-Compact — Nathan Onn](https://www.nathanonn.com/claude-code-never-auto-compact)、[GitHub #34925](https://github.com/anthropics/claude-code/issues/34925)、[GitHub #42055](https://github.com/anthropics/claude-code/issues/42055)（社区观察）
