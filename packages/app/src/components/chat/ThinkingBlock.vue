<script setup lang="ts">
/**
 * ThinkingBlock — 思考过程展示组件 (P2-1f)
 *
 * 仅 Anthropic 模型支持 extended thinking。
 *
 * 功能：
 * - 折叠态："思考中..."（灰色小字，带 loading 动画）
 * - 展开态：完整思考内容（淡灰色背景，等宽字体）
 *
 * Props:
 * - content: 思考过程文本
 * - streaming: 是否仍在接收（控制 loading 动画）
 */

import { computed, ref, watch } from "vue";
import { Brain } from "lucide-vue-next";

const props = defineProps<{
  content: string;
  streaming?: boolean;
}>();

const expanded = ref(false);

/** 内容预览（折叠时显示） */
const preview = computed(() => {
  if (!props.content) return "";
  const firstLine = props.content.split("\n")[0] || "";
  return firstLine.length > 100 ? firstLine.slice(0, 100) + "…" : firstLine;
});

/** 当有新内容时自动展开（首次接收时） */
watch(
  () => props.content,
  (newVal, oldVal) => {
    if (newVal && !oldVal && !expanded.value) {
      // 首次有内容：不自动展开，保持折叠让用户决定
    }
  },
);
</script>

<template>
  <div
    v-if="content || streaming"
    class="thinking-block rounded-lg border border-purple-200 dark:border-purple-800/50 bg-purple-50/50 dark:bg-purple-900/10 my-2 overflow-hidden"
  >
    <!-- 折叠/展开头部 -->
    <button
      class="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-purple-50 dark:hover:bg-purple-900/20 transition-colors"
      @click="expanded = !expanded"
    >
      <!-- 图标 -->
      <span class="text-sm shrink-0">
        <Brain v-if="streaming" :size="14" class="animate-pulse" aria-hidden="true" />
        <Brain v-else :size="14" aria-hidden="true" />
      </span>

      <!-- 标题 -->
      <span class="text-xs text-purple-600 dark:text-purple-400 flex-1 truncate">
        {{ streaming ? "思考中…" : "思考过程" }}
        <span v-if="preview && !expanded" class="text-purple-400/60 dark:text-purple-500/60 ml-1">
          {{ preview }}
        </span>
      </span>

      <!-- 展开/折叠箭头 -->
      <svg
        class="w-3.5 h-3.5 text-purple-400 transition-transform shrink-0"
        :class="{ 'rotate-180': expanded }"
        fill="none" viewBox="0 0 24 24" stroke="currentColor"
      >
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
      </svg>
    </button>

    <!-- 展开内容 -->
    <div v-if="expanded" class="border-t border-purple-200 dark:border-purple-800/50 px-3 py-2">
      <pre class="text-xs font-mono text-purple-700 dark:text-purple-300 whitespace-pre-wrap break-words">{{ content }}</pre>
    </div>
  </div>
</template>
