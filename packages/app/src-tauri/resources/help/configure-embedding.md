---
title: 配置知识库与 embedding（语义检索）
summary: 知识库(RAG)怎么开、embedding 向量模型怎么配、文档放哪、为什么语义检索没生效。关键词检索无需 embedding 也能用。
tags: [知识库, embedding, 向量, 语义检索, RAG, 配置, 检索, 全局知识库, agent 知识库]
---

# 配置知识库与 embedding（语义检索）

知识库（RAG）让 agent 能检索你提供的文档来回答问题。本文解决最常见的「配了知识库但搜不到 / 语义检索没生效」问题。

## 两种检索模式

- **关键词检索**：默认开启，无需任何额外配置。按标题、摘要、标签、文件路径做匹配。
- **语义检索（向量 embedding）**：需要单独配置 embedding 模型。理解语义（同义词、近义问题），比关键词更准。**没配 embedding 时自动退化为关键词检索**——这就是「知识库能用但不够智能」的原因。

> 如果你现在问 agent 知识库相关问题搜不到，先确认下面 embedding 是否配了。

## 配置 embedding（启用语义检索）

在 **设置 → 通用设置** 里配置（这是全局配置，所有知识库共用，和 agent 的 LLM 模型是分开的）：

| 字段 | 说明 |
|---|---|
| Provider | `glm`（智谱）/ `openai` / `deepseek` |
| Model | embedding 模型名，如 `embedding-3`（按 provider 选） |
| API Key | 该 provider 的 embedding 接口密钥 |
| Base URL | 一般留空自动用默认；自建/代理才填 |

默认 Base URL：智谱 `https://open.bigmodel.cn/api/paas/v4`、OpenAI `https://api.openai.com`、DeepSeek `https://api.deepseek.com`。

**切换 embedding 模型**会触发全量重建（清掉旧维度向量、重新生成），数据量大时可能要几十秒。切换前会有二次确认。

## 文档放哪 / 怎么入库

知识库按约定目录自动建立，无需手动创建：

- **全局知识库**：`<默认工作空间>/knowledge/`——所有 agent 都能检索。
- **agent 专属知识库**：`<agent 工作区>/knowledge/`——只有该 agent 检索。

把 `.md` 文件（目前只支持 markdown）丢进对应目录即可。文件保存后 **2 秒内**自动触发索引（编辑器多次保存会合并为一次）。索引是增量的：按内容哈希去重，没改的文件不会重复处理。

## markdown 文档建议格式

文件开头写 YAML frontmatter，能显著提升检索质量：

```
---
title: 文档标题
summary: 一句话摘要，多写关键词
tags: [标签1, 标签2]
---
正文……
```

没有 frontmatter 时，自动取第一个 `# 标题` 作标题、第一段作摘要。

## 常见没生效的原因

1. **没配 embedding** → 只有关键词检索，同义/近义问法搜不到。按上面配好 embedding。
2. **API Key 错或额度不足** → embedding 生成失败。在设置里确认 Key 有效。
3. **文件不在 knowledge 目录** → 不会被索引。确认路径（全局库看 `<默认工作空间>/knowledge/`）。
4. **文件不是 `.md`** → 当前只索引 markdown。
5. **知识库被禁用** → 在知识库页面确认是启用状态。

## 相关

- agent 怎么建、API Key 怎么配：见「快速上手」。
- 工作空间/目录概念：见「项目与 workspace」。
