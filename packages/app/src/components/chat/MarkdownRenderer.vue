<!--
  MarkdownRenderer — Markdown → HTML 渲染（markdown-it + highlight.js）

  Props: content: string（原始 Markdown 文本）
         streaming?: boolean（流式生成中——代码块不折叠，完成后恢复）
  Emits: 无
-->
<script setup lang="ts">
// 支持流式渲染：每次 content 变化时增量重解析。
// markdown-it 单次解析在微秒级，流式场景下完全可以承受全量重渲染。
// highlight.js 只对完整代码块着色，避免流式中途闪烁。

import { computed, ref } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import MarkdownIt from "markdown-it";
import hljs from "highlight.js";
// 只导入常用语言，缩小体积
import "highlight.js/lib/common";
import { preprocessMarkdown } from "../../utils/markdown";

const rootRef = ref<HTMLElement | null>(null);

// ---------- 代码块：语言别名 + 折叠阈值 ----------

/** 常见别名的展示名（其余小写原样显示；无语言显示「代码」）。 */
const LANG_ALIASES: Record<string, string> = {
  js: "JavaScript", javascript: "JavaScript", mjs: "JavaScript",
  ts: "TypeScript", typescript: "TypeScript",
  py: "Python", python: "Python",
  rs: "Rust", rust: "Rust",
  sh: "Shell", bash: "Shell", shell: "Shell", zsh: "Shell", powershell: "PowerShell",
  yml: "YAML", yaml: "YAML", json: "JSON", toml: "TOML",
  md: "Markdown", html: "HTML", css: "CSS", vue: "Vue", sql: "SQL", diff: "Diff",
  "c++": "C++", cpp: "C++", cs: "C#", csharp: "C#", go: "Go", java: "Java",
};

/** 折叠阈值：超过该行数的代码块默认折叠（渐隐遮罩 + 展开按钮）。 */
const COLLAPSE_LINES = 24;

function countCodeLines(src: string): number {
  if (!src) return 0;
  const lines = src.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop(); // 尾换行不算一行
  return lines.length;
}

const md = new MarkdownIt({
  html: false,
  linkify: true,
  typographer: true,
  breaks: true,
});

// fence 重写：代码块包一层容器（头部条 = 语言标签 + 复制按钮；超长折叠 + 展开按钮）。
// 按钮在 v-html 内容里（不在 Vue 模板），交互走根节点事件委托（onRootClick）——
// 流式全量重渲染（innerHTML 重建）不丢监听；按钮反馈直接改 textContent，
// 流式重渲染会重置回「复制」，可接受。
md.renderer.rules.fence = (tokens, idx) => {
  const token = tokens[idx];
  // info 形如 "rust ignore" / "c++"——取首个合法语言记号（防 attr/class 注入：字符集白名单）
  const raw = (token.info.trim().match(/^[a-zA-Z0-9+#._-]+/)?.[0] ?? "").toLowerCase();
  const src = token.content;
  const lines = countCodeLines(src);
  let codeHtml: string;
  if (raw && hljs.getLanguage(raw)) {
    try {
      codeHtml = hljs.highlight(src, { language: raw, ignoreIllegals: true }).value;
    } catch {
      codeHtml = md.utils.escapeHtml(src);
    }
  } else {
    // 无语言或不支持 → 自动探测
    codeHtml = hljs.highlightAuto(src).value;
  }
  const label = raw ? (LANG_ALIASES[raw] ?? raw) : "代码";
  const collapsed = lines > COLLAPSE_LINES ? " collapsed" : "";
  const toggle = collapsed
    ? `<button class="md-code-toggle" type="button">展开 ${lines} 行</button>`
    : "";
  return (
    `<div class="md-code-block${collapsed}" data-lang="${raw}" data-lines="${lines}">` +
    `<div class="md-code-head"><span class="md-code-lang">${label}</span>` +
    `<button class="md-code-copy" type="button">复制</button></div>` +
    `<pre class="markdown-body-code-pre"><code class="hljs${raw ? ` language-${raw}` : ""}">${codeHtml}</code></pre>` +
    toggle +
    `</div>`
  );
};

// 容错预处理见 utils/markdown.ts：愈合不合规的分隔行（模型吐空 pipe / 列数不足 /
// em-dash / 单元格漏 -）+ 给表前补空行，避免 GFM 表格整表退化成普通段落。
const origRender = md.render.bind(md);
md.render = function (src: string): string {
  let html = origRender(preprocessMarkdown(src));
  // 移除 <li> 内直属 <p> 标签（无论后面是否有嵌套标签）
  // markdown-it 生成 <li><p>content</p></li> 或 <li><p>content</p><ul>...</ul></li>
  // <p> 的 display:block 导致序号与内容在不同行
  html = html.replace(/<li>\s*<p>([\s\S]*?)<\/p>/g, '<li>$1');
  // 表格包一层滚动 wrapper：避免给 <table> 设 display:block（会拆散 thead/tbody、
  // 列宽错位）；改为 wrapper 承担 overflow-x，table 保持原生表格布局。
  html = html.replace(/<table[^>]*>[\s\S]*?<\/table>/g, '<div class="markdown-table-wrap">$&</div>');
  return html;
};

// ---------- 交互：链接外开 + 代码块按钮（事件委托） ----------

/** 复制代码块原文（code innerText 还原未着色源码）。 */
async function copyCode(btn: HTMLButtonElement) {
  const code = btn.closest(".md-code-block")?.querySelector("pre code")?.textContent ?? "";
  try {
    await navigator.clipboard.writeText(code);
    btn.textContent = "已复制";
  } catch {
    /* clipboard 不可用（权限/环境）时静默 */
    return;
  }
  window.setTimeout(() => {
    btn.textContent = "复制";
  }, 2000);
}

/** 展开 / 收起超长代码块。 */
function toggleCollapse(btn: HTMLButtonElement) {
  const block = btn.closest<HTMLElement>(".md-code-block");
  if (!block) return;
  const collapsed = block.classList.toggle("collapsed");
  const lines = block.getAttribute("data-lines") ?? "";
  btn.textContent = collapsed ? `展开 ${lines} 行` : "收起";
}

function onRootClick(e: MouseEvent) {
  const target = e.target as HTMLElement | null;
  if (!target) return;
  const copyBtn = target.closest?.(".md-code-copy");
  if (copyBtn) {
    void copyCode(copyBtn as HTMLButtonElement);
    return;
  }
  const toggleBtn = target.closest?.(".md-code-toggle");
  if (toggleBtn) {
    toggleCollapse(toggleBtn as HTMLButtonElement);
    return;
  }
  const link = target.closest?.("a");
  if (!link || !link.href) return;
  // 只拦截外部链接，不拦截锚点
  if (link.href.startsWith("http://") || link.href.startsWith("https://")) {
    e.preventDefault();
    e.stopPropagation();
    openUrl(link.href);
  }
}

const props = withDefaults(
  defineProps<{
    content: string;
    /** 流式生成中：代码块不折叠（CSS 门控 .md-streaming），完成后恢复折叠。 */
    streaming?: boolean;
  }>(),
  { streaming: false },
);

const rendered = computed(() => {
  if (!props.content) return "";
  return md.render(props.content);
});
</script>

<template>
  <!-- eslint-disable vue/no-v-html -- markdown-it 输出已清理，需 v-html 渲染 -->
  <div
    ref="rootRef"
    :class="['markdown-body', { 'md-streaming': streaming }]"
    @click="onRootClick"
    v-html="rendered"
  />
  <!-- eslint-enable vue/no-v-html -->
</template>
