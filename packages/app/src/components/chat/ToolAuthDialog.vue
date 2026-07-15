<script setup lang="ts">
/**
 * ToolAuthDialog — 工具授权确认弹窗 (A2-3)
 *
 * 职责：
 *   - 监听 `useToolAuth()` 的 pendingRequest
 *   - 当有 pendingRequest 时弹出 Modal
 *   - 显示工具名 / 待访问路径 / 参数 JSON（格式化后）
 *   - 「允许」按钮 → useToolAuth.respond(request_id, true)
 *   - 「拒绝」按钮 → useToolAuth.respond(request_id, false)
 *
 * 设计：
 *   - 单实例：仅 ChatPage 引入一次
 *   - 串行弹窗：useToolAuth 内部 FIFO 队列，每次只弹队首；
 *     用户响应后弹出下一个（如有）
 *   - 不允许 Esc / 遮罩关闭：授权决策必须显式选择
 *
 * 与 IcePaw UI 风格对齐：
 *   - 使用 @ice-paw/ui 的 Modal + Button 组件
 *   - 路径 / 参数展示采用等宽字体 + 灰底卡片
 */

import { computed } from "vue";
import { Modal, Button } from "@ice-paw/ui";
import { useToolAuth } from "../../composables/useToolAuth";

const { pendingRequest, respond } = useToolAuth();

// ============================================================================
// 派生展示
// ============================================================================

/** 弹窗是否打开（仅当有 pendingRequest 时打开） */
const open = computed<boolean>(() => pendingRequest.value !== null);

/** 工具的友好显示名（与 ToolCallBlock 保持一致） */
const displayToolName = computed<string>(() => {
  const name = pendingRequest.value?.tool_name ?? "";
  const map: Record<string, string> = {
    read_file: "读取文件",
    write_file: "写入文件",
    edit_file: "编辑文件",
    list_directory: "列出目录",
    run_command: "执行命令",
    execute_command: "执行命令",
    exec: "执行命令",
  };
  return map[name] || name;
});

/** 工具图标（与 ToolCallBlock 风格保持一致） */
const toolIcon = computed<string>(() => {
  switch (pendingRequest.value?.tool_name) {
    case "read_file":
      return "📄";
    case "write_file":
      return "✍️";
    case "edit_file":
      return "🩹";
    case "list_directory":
      return "📂";
    case "run_command":
    case "execute_command":
    case "exec":
      return "▶️";
    default:
      return "🔧";
  }
});

/** 格式化的参数 JSON（用于展开时展示） */
const formattedArguments = computed<string>(() => {
  const raw = pendingRequest.value?.arguments;
  if (!raw) return "(无参数)";
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
});

// ============================================================================
// 行为
// ============================================================================

/** 用户点击「拒绝」 */
async function onDeny(): Promise<void> {
  const req = pendingRequest.value;
  if (!req) return;
  await respond(req.request_id, false);
}

/** 用户点击「允许」 */
async function onAllow(): Promise<void> {
  const req = pendingRequest.value;
  if (!req) return;
  await respond(req.request_id, true);
}
</script>

<template>
  <Modal
    :model-value="open"
    size="sm"
    :title="'工具授权确认'"
    :close-on-overlay="false"
    :close-on-esc="false"
    :show-close="false"
    @update:model-value="() => {}"
  >
    <!-- 工具标识 + 触发原因 -->
    <div class="ta-header">
      <span class="ta-icon">{{ toolIcon }}</span>
      <div class="ta-info">
        <div class="ta-tool-name">
          <code>{{ displayToolName }}</code>
          <span class="ta-tool-raw">{{ pendingRequest?.tool_name }}</span>
        </div>
        <div class="ta-reason">
          {{ pendingRequest?.reason ?? "此工具需要用户确认授权" }}
        </div>
      </div>
    </div>

    <!-- 路径 -->
    <div v-if="pendingRequest?.file_path" class="ta-section">
      <div class="ta-label">待访问路径</div>
      <pre class="ta-pre ta-pre-path">{{ pendingRequest.file_path }}</pre>
    </div>

    <!-- 参数 -->
    <div class="ta-section">
      <div class="ta-label">调用参数</div>
      <pre class="ta-pre">{{ formattedArguments }}</pre>
    </div>

    <template #footer>
      <Button variant="secondary" @click="onDeny">拒绝</Button>
      <Button variant="primary" @click="onAllow">允许</Button>
    </template>
  </Modal>
</template>

<style scoped>
.ta-header {
  display: flex;
  align-items: flex-start;
  gap: var(--ip-spacing-3);
  margin-bottom: var(--ip-spacing-4);
}

.ta-icon {
  font-size: 24px;
  line-height: 1;
  flex-shrink: 0;
  margin-top: 2px;
}

.ta-info {
  flex: 1;
  min-width: 0;
}

.ta-tool-name {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-2);
  margin-bottom: 4px;
}

.ta-tool-name code {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-body-md-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.ta-tool-raw {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.ta-reason {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-body);
  line-height: var(--ip-line-height-relaxed);
}

.ta-section {
  margin-bottom: var(--ip-spacing-3);
}

.ta-section:last-child {
  margin-bottom: 0;
}

.ta-label {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  font-weight: var(--ip-font-weight-medium);
  margin-bottom: 4px;
}

.ta-pre {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-code-size);
  line-height: var(--ip-line-height-monospace);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-body);
  padding: var(--ip-spacing-3);
  border-radius: var(--ip-radius-md);
  overflow-x: auto;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 160px;
  overflow-y: auto;
}

.ta-pre-path {
  color: var(--ip-warning-text, var(--ip-color-text-body));
  background: var(--ip-color-bg-tertiary);
}
</style>