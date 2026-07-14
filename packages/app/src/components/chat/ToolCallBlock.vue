<script setup lang="ts">
/**
 * ToolCallBlock — 工具调用展示组件 (P2-1f)
 *
 * 功能：
 * - 折叠态：左侧竖线 + 图标 + 函数名 + 自然语言参数摘要 + 状态图标（成功✓ / 失败⚠ / 执行中 spinner）
 * - 展开态：完整参数 JSON + 智能结果展示（file-list / key-value / json / text 四种模式）
 * - 透明背景 + 左侧 2px 竖线：融入助手气泡内部，避免双层灰色背景糊成一团
 * - 全 Token 化：颜色 / 间距 / 圆角 / 字号全部走 --ip-* 设计 Token
 *
 * Props:
 * - name:      工具名称
 * - arguments: 参数 JSON 字符串
 * - ended:     是否已完成（收到 tool-call-end）
 * - result:    执行结果（由 tool-result 事件 / content_blocks 填充）
 * - isError:   结果是否为错误
 */

import { computed, ref } from "vue";

const props = defineProps<{
  name: string;
  arguments: string;
  ended: boolean;
  result?: string;
  isError?: boolean;
}>();

const expanded = ref(false);

// ============================================================================
// 展示用派生
// ============================================================================

/** 友好的工具显示名称 */
const displayName = computed(() => {
  const nameMap: Record<string, string> = {
    read_file: "读取文件",
    write_file: "写入文件",
    edit_file: "编辑文件",
    list_directory: "列出目录",
    run_command: "执行命令",
    execute_command: "执行命令",
    exec: "执行命令",
    web_search: "网页搜索",
    web_fetch: "获取网页",
    search: "搜索",
  };
  return nameMap[props.name] || props.name;
});

/** 工具图标 */
const icon = computed(() => {
  switch (props.name) {
    case "read_file":
      return "📄";
    case "list_directory":
      return "📂";
    case "web_search":
    case "search":
      return "🔍";
    case "web_fetch":
      return "🌐";
    case "run_command":
    case "execute_command":
    case "exec":
      return "▶️";
    case "write_file":
      return "✍️";
    case "edit_file":
      return "🩹";
    default:
      return "🔧";
  }
});

/** 自然语言参数摘要 */
const argSummary = computed(() => {
  if (!props.arguments) return "等待参数…";
  try {
    const parsed = JSON.parse(props.arguments);
    switch (props.name) {
      case "read_file":
      case "write_file":
      case "edit_file":
        return parsed.path || parsed.file_path || JSON.stringify(parsed);
      case "list_directory":
        return parsed.path || parsed.dir || "当前目录";
      case "run_command":
      case "execute_command":
      case "exec":
        return parsed.command || parsed.cmd || JSON.stringify(parsed);
      case "web_search":
      case "search":
        return parsed.query || parsed.q || JSON.stringify(parsed);
      case "web_fetch":
        return parsed.url || JSON.stringify(parsed);
      default: {
        // 通用策略：取第一个字符串值
        const firstVal = Object.values(parsed).find(
          (v) => typeof v === "string" && v.length > 0,
        );
        if (firstVal && typeof firstVal === "string") {
          return firstVal.length > 60 ? firstVal.slice(0, 60) + "…" : firstVal;
        }
        const str = JSON.stringify(parsed);
        return str.length > 60 ? str.slice(0, 60) + "…" : str;
      }
    }
  } catch {
    return props.arguments.length > 60
      ? props.arguments.slice(0, 60) + "…"
      : props.arguments || "等待参数…";
  }
});

/** 状态信息：图标 + 文字 + 颜色类 */
const statusInfo = computed(() => {
  if (!props.ended) {
    return {
      icon: "⏳",
      text: "等待参数…",
      color: "var(--ip-color-text-tertiary)",
      spin: false,
    };
  }
  if (props.result === undefined) {
    return {
      icon: "⏳",
      text: "执行中…",
      color: "var(--ip-info-base)",
      spin: true,
    };
  }
  if (props.isError) {
    return {
      icon: "⚠️",
      text: "执行失败",
      color: "var(--ip-danger-text)",
      spin: false,
    };
  }
  return {
    icon: "✓",
    text: "已完成",
    color: "var(--ip-success-text)",
    spin: false,
  };
});

/** 头部左侧竖线颜色（响应 result 状态） */
const barColor = computed<string>(() => {
  if (props.isError) return "var(--ip-danger-border)";
  if (props.result !== undefined) return "var(--ip-success-border)";
  if (props.ended) return "var(--ip-info-border)";
  return "var(--ip-color-border-default)";
});

/** 格式化的参数 JSON（展开时展示） */
const formattedArgs = computed(() => {
  try {
    return JSON.stringify(JSON.parse(props.arguments), null, 2);
  } catch {
    return props.arguments || "(等待参数…)";
  }
});

/** 解析结果，判断展示模式 */
interface ResultDisplayText {
  mode: "text";
  content: string;
}
interface ResultDisplayFileList {
  mode: "file-list";
  items: Record<string, unknown>[];
}
interface ResultDisplayKeyValue {
  mode: "key-value";
  data: Record<string, unknown>;
}
interface ResultDisplayJson {
  mode: "json";
  content: string;
}

type ResultDisplay =
  | ResultDisplayText
  | ResultDisplayFileList
  | ResultDisplayKeyValue
  | ResultDisplayJson;

const resultDisplay = computed<ResultDisplay | null>(() => {
  if (!props.result) return null;
  return parseToolResult(props.name, props.result);
});

function parseToolResult(_toolName: string, raw: string): ResultDisplay {
  // 尝试 JSON 解析
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return { mode: "text", content: raw };
  }

  // 文件列表模式：数组 + 元素含 name/isDir/path 等字段
  if (Array.isArray(parsed) && parsed.length > 0 && typeof parsed[0] === "object") {
    const first = parsed[0] as Record<string, unknown>;
    if (
      "name" in first ||
      "path" in first ||
      "isDir" in first ||
      "is_dir" in first ||
      "type" in first
    ) {
      return { mode: "file-list", items: parsed as Record<string, unknown>[] };
    }
  }

  // 键值对模式：对象且字段数 ≤ 8
  if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
    const obj = parsed as Record<string, unknown>;
    const keys = Object.keys(obj);
    if (keys.length <= 8 && keys.every((k) => typeof obj[k] !== "object")) {
      return { mode: "key-value", data: obj };
    }
  }

  // 默认：格式化 JSON
  return { mode: "json", content: JSON.stringify(parsed, null, 2) };
}

/** 文件项图标 */
function getItemIcon(item: Record<string, unknown>): string {
  const isDir =
    item.isDir || item.is_dir || item.type === "directory" || item.type === "dir";
  if (isDir) return "📁";

  const name = (item.name || item.path || "") as string;
  const ext = name.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "rs":
      return "🦀";
    case "ts":
    case "tsx":
      return "📜";
    case "js":
    case "jsx":
      return "📜";
    case "vue":
      return "💚";
    case "json":
      return "📋";
    case "md":
      return "📝";
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
    case "svg":
    case "webp":
      return "🖼️";
    case "toml":
    case "yaml":
    case "yml":
      return "⚙️";
    case "lock":
      return "🔒";
    default:
      return "📄";
  }
}

/** 格式化文件大小 */
function formatSize(bytes: unknown): string {
  if (typeof bytes !== "number" || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
</script>

<template>
  <div class="tool-call-block">
    <!-- 折叠/展开头部 -->
    <button
      class="tc-header"
      :style="{ borderLeftColor: barColor }"
      type="button"
      :aria-expanded="expanded"
      @click="expanded = !expanded"
    >
      <!-- 工具图标 -->
      <span class="tc-icon">{{ icon }}</span>

      <!-- 函数名 + 自然语言摘要 -->
      <span class="tc-label">
        <span class="tc-name">{{ displayName }}</span>
        <span class="tc-args">{{ argSummary }}</span>
      </span>

      <!-- 状态 -->
      <span class="tc-status" :style="{ color: statusInfo.color }">
        <span v-if="statusInfo.spin" class="tc-spinner" />
        <span v-else class="tc-status-icon">{{ statusInfo.icon }}</span>
        <span class="tc-status-text">{{ statusInfo.text }}</span>
      </span>

      <!-- 展开箭头 -->
      <svg
        class="tc-chevron"
        :class="{ 'tc-chevron-open': expanded }"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          stroke-linecap="round"
          stroke-linejoin="round"
          stroke-width="2"
          d="M19 9l-7 7-7-7"
        />
      </svg>
    </button>

    <!-- 展开内容 -->
    <Transition name="tc-expand">
      <div v-if="expanded" class="tc-body" :style="{ borderLeftColor: barColor }">
        <!-- 参数 -->
        <div class="tc-section">
          <div class="tc-section-label">参数</div>
          <pre class="tc-pre">{{ formattedArgs }}</pre>
        </div>

        <!-- 结果 -->
        <div v-if="resultDisplay" class="tc-section">
          <div
            class="tc-section-label"
            :class="{ 'tc-label-error': isError, 'tc-label-ok': !isError }"
          >
            {{ isError ? "错误" : "结果" }}
          </div>

          <!-- 文件列表模式 -->
          <div v-if="resultDisplay.mode === 'file-list'" class="tc-file-list">
            <div
              v-for="(item, i) in resultDisplay.items"
              :key="i"
              class="tc-file-item"
            >
              <span class="tc-file-icon">{{ getItemIcon(item) }}</span>
              <span class="tc-file-name">
                {{ item.name || item.path || JSON.stringify(item) }}
              </span>
              <span v-if="item.size" class="tc-file-meta">{{ formatSize(item.size) }}</span>
            </div>
          </div>

          <!-- 键值对模式 -->
          <div v-else-if="resultDisplay.mode === 'key-value'" class="tc-kv-list">
            <div
              v-for="(val, key) in resultDisplay.data"
              :key="key"
              class="tc-kv-row"
            >
              <span class="tc-kv-key">{{ key }}</span>
              <span class="tc-kv-val">{{ val }}</span>
            </div>
          </div>

          <!-- JSON / 纯文本模式 -->
          <pre
            v-else
            class="tc-pre"
            :class="{ 'tc-pre-error': isError }"
          >{{ resultDisplay.content }}</pre>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.tool-call-block {
  margin: var(--ip-spacing-2) 0; /* 8px 上下 */
  border-radius: var(--ip-radius-md); /* 6px */
  overflow: hidden;
}

/* 头部按钮 — 透明背景融入气泡，左侧 2px 竖线随状态变 */
.tc-header {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2); /* 8px */
  width: 100%;
  padding: var(--ip-spacing-1) var(--ip-spacing-2); /* 4px 8px */
  background: transparent;
  border: none;
  border-left: 2px solid var(--ip-color-border-default);
  border-radius: 0;
  text-align: left;
  cursor: pointer;
  transition: var(--ip-transition-colors);
  font-family: inherit;
}

.tc-header:hover {
  background: var(--ip-color-bg-tertiary);
}

/* 工具图标 */
.tc-icon {
  font-size: var(--ip-text-body-sm-size); /* 13px */
  flex-shrink: 0;
  line-height: 1;
}

/* 函数名 + 摘要 */
.tc-label {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 6px;
  overflow: hidden;
}

.tc-name {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-body-sm-size); /* 13px */
  font-weight: var(--ip-font-weight-semibold); /* 600 */
  color: var(--ip-color-text-body);
  flex-shrink: 0;
}

.tc-args {
  font-size: var(--ip-text-caption-size); /* 12px */
  color: var(--ip-color-text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

/* 状态 */
.tc-status {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  font-size: var(--ip-text-caption-size);
  flex-shrink: 0;
}

.tc-status-icon {
  font-size: 11px;
  line-height: 1;
}

.tc-status-text {
  white-space: nowrap;
}

/* Spinner */
.tc-spinner {
  display: inline-block;
  width: 12px;
  height: 12px;
  border: 2px solid currentColor;
  border-top-color: transparent;
  border-radius: 50%;
  animation: ip-spin var(--ip-duration-spinner) linear infinite;
}

/* 展开箭头 */
.tc-chevron {
  width: 14px;
  height: 14px;
  color: var(--ip-color-text-tertiary);
  transition: var(--ip-transition-transform);
  flex-shrink: 0;
}

.tc-chevron-open {
  transform: rotate(180deg);
}

/* 展开内容区 */
.tc-body {
  padding: var(--ip-spacing-1) var(--ip-spacing-2) var(--ip-spacing-2) 10px;
  margin-left: 2px;
  border-left: 2px solid var(--ip-color-border-default);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.tc-section {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.tc-section-label {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  font-weight: var(--ip-font-weight-medium);
}

.tc-label-ok {
  color: var(--ip-success-text);
}

.tc-label-error {
  color: var(--ip-danger-text);
}

/* 代码块 / JSON */
.tc-pre {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-code-size); /* 14px */
  line-height: var(--ip-line-height-monospace);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-body);
  padding: var(--ip-spacing-2);
  border-radius: var(--ip-radius-sm);
  overflow-x: auto;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
}

.tc-pre-error {
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
}

/* 文件列表 */
.tc-file-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--ip-spacing-2);
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-sm);
}

.tc-file-item {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-body);
}

.tc-file-icon {
  flex-shrink: 0;
  width: 16px;
  text-align: center;
}

.tc-file-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-caption-size);
}

.tc-file-meta {
  flex-shrink: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: var(--ip-font-variant-numeric);
}

/* 键值对列表 */
.tc-kv-list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
  padding: var(--ip-spacing-2);
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-sm);
}

.tc-kv-row {
  display: flex;
  gap: var(--ip-spacing-2);
  font-size: var(--ip-text-body-sm-size);
  align-items: baseline;
}

.tc-kv-key {
  font-family: var(--ip-font-mono);
  color: var(--ip-color-text-tertiary);
  flex-shrink: 0;
}

.tc-kv-val {
  color: var(--ip-color-text-body);
  word-break: break-word;
}

/* 展开过渡动画 */
.tc-expand-enter-active,
.tc-expand-leave-active {
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
}

.tc-expand-enter-from,
.tc-expand-leave-to {
  opacity: 0;
}

/* 减少动效 */
@media (prefers-reduced-motion: reduce) {
  .tc-spinner {
    animation: none;
  }
}
</style>
