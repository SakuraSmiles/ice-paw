<script setup lang="ts">
// ChatMessages.vue — 聊天消息列表
import { ref, nextTick, watch } from "vue";
import MarkdownRenderer from "./MarkdownRenderer.vue";

interface Message {
  id: string;
  role: "user" | "assistant";
  content: string;
  time: string;
}

const messages = ref<Message[]>([
  {
    id: "1",
    role: "assistant",
    content: "你好！我是 IcePaw 助手，有什么可以帮助你的？你可以问我技术问题、让我帮你写代码、或者只是聊聊天。",
    time: "10:30",
  },
  {
    id: "2",
    role: "user",
    content: "能帮我写一个 Vite 插件，在构建时把项目中的 .env 文件注释生成到 README.md 吗？",
    time: "10:31",
  },
  {
    id: "3",
    role: "assistant",
    content: `当然可以！下面是一个完整的 Vite 插件实现：

\`\`\`ts
// plugins/generate-env-docs.ts
import { Plugin } from 'vite'
import { readFileSync, writeFileSync } from 'fs'
import { resolve } from 'path'

interface EnvDocOptions {
  envPath?: string
  readmePath?: string
}

export function generateEnvDocs(options: EnvDocOptions = {}): Plugin {
  const {
    envPath = resolve(process.cwd(), '.env.example'),
    readmePath = resolve(process.cwd(), 'README.md'),
  } = options

  return {
    name: 'generate-env-docs',
    enforce: 'post',

    async closeBundle() {
      try {
        const content = readFileSync(envPath, 'utf-8')
        const table = parseEnvToTable(content)
        injectIntoReadme(readmePath, table)
        this.info('✅ .env 文档已更新到 README.md')
      } catch (e) {
        this.warn(\`[警告] 生成 env 文档失败: \${e}\`)
      }
    },
  }
}
\`\`\`

使用方式很简单，在 \`vite.config.ts\` 中注册：

\`\`\`ts
import { defineConfig } from 'vite'
import { generateEnvDocs } from './plugins/generate-env-docs'

export default defineConfig({
  plugins: [
    generateEnvDocs({
      envPath: './.env.example',
      readmePath: './README.md',
    }),
  ],
})
\`\`\`

## 主要功能

- **自动解析** — 读取 \`.env.example\` 中的注释和变量名
- **表格生成** — 把环境变量转换成格式化的 Markdown 表格
- **智能注入** — 在 README 中找到 \`<!-- ENV_DOCS -->\` 标记位并替换内容
- **构建时执行** — 只在 \`vite build\` 时触发，dev 模式下跳过

### 生成的表格示例

| 变量名 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| \`VITE_API_BASE\` | string | \`/api\` | API 请求地址 |
| \`VITE_ENABLE_DEBUG\` | boolean | \`false\` | 是否开启调试模式 |
| \`VITE_APP_TITLE\` | string | \`MyApp\` | 应用标题 |

### TODO 清单

1. 支持 \`yaml\` 格式的环境变量文件
2. 添加变量值的有效性校验
3. 支持多语言文档输出
4. 集成 CI 自动检查环境变量是否缺失

> **提示**：这只是一个基础实现，你可以根据项目需求灵活扩展。`,
    time: "10:31",
  },
  {
    id: "4",
    role: "user",
    content: "这个插件不错！那能不能再加一个功能：支持从 `.env` 文件（不是 `.env.example`）中读取实际值，生成一份 `env-config.md` 作为内部参考文档？",
    time: "10:32",
  },
  {
    id: "5",
    role: "assistant",
    content: `好思路！下面是对插件的扩展实现：

\`\`\`diff
 export function generateEnvDocs(options: EnvDocOptions = {}): Plugin {
   const {
     envPath = resolve(process.cwd(), '.env.example'),
     readmePath = resolve(process.cwd(), 'README.md'),
+    mode = 'example',
   } = options
\`\`\`

## 两种模式对比

| 特性 | example 模式 | actual 模式 |
|------|-------------|-------------|
| 数据来源 | \`.env.example\` 注释 | \`.env\` 实际值 |
| 安全级别 | 高（无敏感信息） | 低（含真实密钥） |
| 使用场景 | 公开文档 | 内部开发参考 |
| 推荐程度 | 推荐 | 仅本地使用 |

### 敏感信息处理

对于 actual 模式，建议在 \`.gitignore\` 中添加：

\`\`\`gitignore
# 生成的内部参考文档
env-config.md
\`\`\`

### 后续优化方向

- [x] 基础插件框架
- [x] 表格自动生成
- [x] 双模式支持
- [ ] 环境变量变动监控
- [ ] VS Code 插件集成
- [ ] Web UI 可视化配置面板`,
    time: "10:32",
  },
]);

const listRef = ref<HTMLElement | null>(null);

// 自动滚到底部
watch(messages, async () => {
  await nextTick();
  if (listRef.value) {
    listRef.value.scrollTop = listRef.value.scrollHeight;
  }
}, { deep: true });
</script>

<template>
  <div ref="listRef" class="messages-area">
    <div class="messages-container">
      <div
        v-for="msg in messages"
        :key="msg.id"
        :class="['message-row', msg.role]"
      >
        <div class="message-content">
          <div class="message-bubble">
            <MarkdownRenderer v-if="msg.role === 'assistant'" :content="msg.content" />
            <span v-else>{{ msg.content }}</span>
          </div>
          <div class="message-time">{{ msg.time }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.messages-area {
  flex: 1;
  overflow-y: auto;
  padding: 24px 0;
}

.messages-container {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 0 48px;
}

.message-row {
  display: flex;
}

.message-row.user {
  justify-content: flex-end;
}

.message-row.assistant {
  justify-content: flex-start;
}

.message-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.message-row.assistant .message-content {
  width: 100%;
  max-width: 85%;
}

.message-row.user .message-content {
  max-width: 70%;
}

.message-row.user .message-content {
  align-items: flex-end;
}

.message-bubble {
  padding: 10px 16px;
  border-radius: 12px;
  font-size: var(--ip-text-body-size);
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}

.message-row.user .message-bubble {
  background-color: var(--color-message-user-bg);
  color: var(--color-message-user-text);
  border-bottom-right-radius: 4px;
}

.message-row.assistant .message-bubble {
  background-color: var(--color-message-ai-bg);
  color: var(--color-message-ai-text);
  border-bottom-left-radius: 4px;
}

.message-time {
  font-size: 11px;
  color: var(--ip-color-text-disabled);
  padding: 0 4px;
}
</style>
