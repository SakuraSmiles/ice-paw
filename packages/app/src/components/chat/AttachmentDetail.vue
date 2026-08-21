<!--
  AttachmentDetail — 文档附件详情弹窗（手风琴）

  功能：
  - 列出全部附件（多文档时）；单文档直接展开
  - 每项：类型色图标 + 文件名 + 类型·大小，点击展开/收起
  - 展开内容：后端 materialize 提取的文本原文（可滚动），让用户核对 agent 读到了什么
  - Esc 或点遮罩关闭；Teleport 到 body

  Props: attachments（{name,kind,size}[]）、startIndex、extractedTexts（name→提取文本）
  Emits: close
-->
<script setup lang="ts">
import { reactive } from "vue";

const props = defineProps<{
  attachments: { name: string; kind: string; size: number }[];
  startIndex: number;
  extractedTexts: Record<string, string>;
}>();
const emit = defineEmits<{ close: [] }>();

// 默认展开起始项
const expanded = reactive<Record<number, boolean>>({ [props.startIndex]: true });
function toggle(i: number) { expanded[i] = !expanded[i]; }

const KIND_LABELS: Record<string, string> = { docx: "Word", xlsx: "Excel", xls: "Excel", pdf: "PDF" };
function kindLabel(k: string): string {
  return KIND_LABELS[k.toLowerCase()] ?? k.toUpperCase();
}
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}
import { onMounted, onUnmounted } from "vue";
onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <Teleport to="body">
    <div class="att-detail-mask" @click="emit('close')">
      <div class="att-detail-panel" @click.stop>
        <div class="att-detail-header">
          <span class="att-detail-title">附件详情<template v-if="attachments.length > 1">（{{ attachments.length }} 个）</template></span>
          <button class="att-detail-close" title="关闭 (Esc)" @click="emit('close')">✕</button>
        </div>
        <div class="att-detail-list">
          <div v-for="(att, i) in attachments" :key="i" class="att-detail-item" :class="{ open: expanded[i] }">
            <button class="att-detail-toggle" @click="toggle(i)">
              <span class="att-detail-icon" :data-kind="att.kind">{{ kindLabel(att.kind)[0] }}</span>
              <span class="att-detail-info">
                <span class="att-detail-name">{{ att.name }}</span>
                <span class="att-detail-meta">{{ kindLabel(att.kind) }} · {{ formatSize(att.size) }}</span>
              </span>
              <span class="att-detail-arrow">{{ expanded[i] ? "▾" : "▸" }}</span>
            </button>
            <div v-if="expanded[i]" class="att-detail-text">
              <pre>{{ extractedTexts[att.name] || "（无提取文本）" }}</pre>
            </div>
          </div>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.att-detail-mask {
  position: fixed; inset: 0; z-index: var(--ip-z-modal-overlay);
  background: rgba(0, 0, 0, 0.5);
  display: flex; align-items: center; justify-content: center;
  backdrop-filter: blur(2px);
}
.att-detail-panel {
  width: min(640px, 92vw); max-height: 82vh;
  display: flex; flex-direction: column;
  background: var(--ip-color-bg-elevated);
  border-radius: 12px; border: 1px solid var(--ip-color-border-default);
  box-shadow: 0 12px 48px rgba(0, 0, 0, 0.25);
  overflow: hidden;
}
.att-detail-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 14px 18px;
  border-bottom: 1px solid var(--ip-color-border-default);
}
.att-detail-title { font-size: 14px; font-weight: 600; color: var(--ip-color-text-primary); }
.att-detail-close {
  width: 28px; height: 28px; border: none; border-radius: 6px;
  background: transparent; color: var(--ip-color-text-secondary);
  cursor: pointer; font-size: 14px;
}
.att-detail-close:hover { background: var(--ip-color-bg-hover); color: var(--ip-color-text-primary); }

.att-detail-list { overflow-y: auto; padding: 8px; }
.att-detail-item { border-radius: 8px; }
.att-detail-item + .att-detail-item { border-top: 1px solid var(--ip-color-border-subtle, rgba(0,0,0,0.05)); }
.att-detail-toggle {
  width: 100%; display: flex; align-items: center; gap: var(--ip-spacing-3);
  padding: 10px 12px; border: none; background: transparent;
  cursor: pointer; text-align: left; border-radius: 8px;
}
.att-detail-toggle:hover { background: var(--ip-color-bg-hover); }
.att-detail-icon {
  flex: none; width: 32px; height: 32px; border-radius: 7px;
  display: flex; align-items: center; justify-content: center;
  font-size: 13px; font-weight: 700; color: #fff;
}
.att-detail-icon[data-kind="pdf"] { background: #dc2626; }
.att-detail-icon[data-kind="docx"] { background: #2563eb; }
.att-detail-icon[data-kind="xlsx"], .att-detail-icon[data-kind="xls"] { background: #16a34a; }
.att-detail-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.att-detail-name {
  font-size: 13px; font-weight: 500; color: var(--ip-color-text-primary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.att-detail-meta { font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); }
.att-detail-arrow { flex: none; color: var(--ip-color-text-tertiary); font-size: 12px; }

.att-detail-text {
  padding: 0 12px 12px 56px;
}
.att-detail-text pre {
  margin: 0; padding: 10px 12px;
  background: var(--ip-color-bg-tertiary); border-radius: 6px;
  font-family: var(--ip-font-mono, ui-monospace, monospace);
  font-size: 12px; line-height: 1.5;
  white-space: pre-wrap; word-break: break-word;
  color: var(--ip-color-text-secondary);
  max-height: 320px; overflow-y: auto;
}
</style>
