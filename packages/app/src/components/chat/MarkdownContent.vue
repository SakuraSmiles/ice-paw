<script setup lang="ts">
// Markdown 渲染组件
//
// 职责：
//   - 接收 Markdown 字符串 prop，使用 useMarkdown 渲染为 HTML
//   - 通过 v-html 注入渲染结果（XSS 安全由 markdown-it html:false 保证）
//   - 适配流式场景：每次 content 变更即重渲染
//
// props:
//   - content: Markdown 字符串（必填）

import { computed } from "vue";
import { useMarkdown } from "../../composables/useMarkdown";
import "../../assets/styles/markdown.css";

const props = defineProps<{
  content: string;
}>();

const { renderMarkdown } = useMarkdown();

/** 渲染后的 HTML 字符串 */
const renderedHtml = computed<string>(() => renderMarkdown(props.content ?? ""));
</script>

<template>
  <!--
    v-html is intentional here. The HTML comes from markdown-it with `html: false`,
    which strips/escapes all raw HTML in the source, and markdown-it's built-in
    link validator blocks dangerous protocols (javascript:, vbscript:, file:, ...).
    Code-block content is escaped before highlight.js runs, so <script> etc.
    cannot execute. See useMarkdown.ts for the full hardening.
  -->
  <!-- eslint-disable-next-line vue/no-v-html -->
  <div class="markdown-body" v-html="renderedHtml" />
</template>

<style scoped>
.markdown-body {
  /* 组件根容器仅做最小约束；具体元素样式交给 markdown.css 全局类 */
  font-size: inherit;
  line-height: inherit;
  color: inherit;
  word-break: break-word;
  overflow-wrap: anywhere;
}
</style>