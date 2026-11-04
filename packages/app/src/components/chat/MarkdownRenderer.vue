<!--
  MarkdownRenderer — Markdown → HTML 渲染（markdown-it + highlight.js）

  Props: content: string（原始 Markdown 文本）
         streaming?: boolean（流式生成中——代码块不折叠，完成后恢复）
  Emits: 无
-->
<script setup lang="ts">
// 支持流式渲染：每次 content 变化时重解析。
// 主线程纪律（2026-11-04 线程饥饿根治批）：
// 1. hljs 结果缓存（lang+src 键 LRU）——流式每 tick 全量重渲染时，已完成代码块
//    的着色是缓存命中，每 tick 实际着色只剩「正在增长的尾块」；
// 2. 流式期未闭合尾 fence 不着色（完成时一次性着色）——增长中的代码块反复
//    hljs.highlight（无语言标注更走 highlightAuto 全语言试探）曾是每 tick 最贵一档；
// 3. 页面不可见冻结（liveRenderFrozen）——keep-alive 隐藏页的组件 effect 仍活，
//    每 tick 照常全量重解析是「生成中切设置页卡顿」的隐形主线程源。

import { ref, watch } from "vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import MarkdownIt from "markdown-it";
import hljs from "highlight.js";
// 只导入常用语言，缩小体积
import "highlight.js/lib/common";
import { preprocessMarkdown } from "../../utils/markdown";
import { liveRenderFrozen } from "../../utils/liveFreeze";

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

// ---------- hljs 结果缓存（LRU，键 = 语言 + 源码） ----------
// markdown-it 解析本身在微秒~毫秒级，长文档的大头是 hljs 着色（大代码块单次
// 可达数十 ms；无语言标注走 highlightAuto 全语言试探更贵）。流式每 tick 全量
// 重渲染下，已完成块反复重着色 = 事实上的 O(doc²)；缓存命中后每 tick 只剩尾块。
const HLJS_CACHE_MAX = 300;
const hljsCache = new Map<string, string>();
function cachedHighlight(key: string, fn: () => string): string {
  const hit = hljsCache.get(key);
  if (hit !== undefined) {
    // LRU touch：删了重插进队尾
    hljsCache.delete(key);
    hljsCache.set(key, hit);
    return hit;
  }
  const out = fn();
  if (out) {
    hljsCache.set(key, out);
    if (hljsCache.size > HLJS_CACHE_MAX) {
      const oldest = hljsCache.keys().next().value;
      if (oldest !== undefined) hljsCache.delete(oldest);
    }
  }
  return out;
}

/** fence 渲染环境（renderMarkdown 构造，fence 规则消费）。 */
interface MdEnv {
  /** 流式期：未闭合尾 fence 的起始行号（缺席 = 无未闭合 / 非流式） */
  mdOpenFenceLine?: number;
}

/** 扫描源码找未闭合尾 fence 的起始行（无则 -1）。行首 ```/~~~（≥3 同字符）计数。 */
function findOpenFenceLine(src: string): number {
  let openCh: string | null = null;
  let openLine = -1;
  const lines = src.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^ {0,3}(`{3,}|~{3,})/);
    if (m) {
      const ch = m[1][0];
      if (openCh === null) {
        openCh = ch;
        openLine = i;
      } else if (ch === openCh) {
        openCh = null; // 闭合
      }
    }
  }
  return openCh === null ? -1 : openLine;
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
md.renderer.rules.fence = (tokens, idx, _options, env) => {
  const token = tokens[idx];
  // info 形如 "rust ignore" / "c++"——取首个合法语言记号（防 attr/class 注入：字符集白名单）
  const raw = (token.info.trim().match(/^[a-zA-Z0-9+#._-]+/)?.[0] ?? "").toLowerCase();
  const src = token.content;
  const lines = countCodeLines(src);
  // 流式期未闭合尾 fence：内容还在增长，着色会随每 tick 重算——直接纯文本
  // （完成时 streaming=false 一次性着色；代码已等宽呈现，视觉无损）。
  // 定位 = fence token 的起始行号（token.map[0]，block token 恒有）比对环境里的
  // 未闭合 fence 行号。
  const openFenceLine = (env as MdEnv | undefined)?.mdOpenFenceLine;
  const openTail =
    typeof openFenceLine === "number" &&
    Array.isArray(token.map) &&
    token.map[0] === openFenceLine;
  let codeHtml: string;
  let uncolored = false;
  if (openTail) {
    codeHtml = md.utils.escapeHtml(src);
    uncolored = true;
  } else if (raw && hljs.getLanguage(raw)) {
    codeHtml = cachedHighlight(`${raw}\u0000${src}`, () => {
      try {
        return hljs.highlight(src, { language: raw, ignoreIllegals: true }).value;
      } catch {
        return md.utils.escapeHtml(src);
      }
    });
  } else {
    // 无语言或不支持 → 自动探测（已完成块缓存命中；增长中的尾块已被 openTail 接住）
    codeHtml = cachedHighlight(`\u0000auto\u0000${src}`, () => hljs.highlightAuto(src).value);
  }
  const label = raw ? (LANG_ALIASES[raw] ?? raw) : "代码";
  const collapsed = lines > COLLAPSE_LINES ? " collapsed" : "";
  const toggle = collapsed
    ? `<button class="md-code-toggle" type="button">展开 ${lines} 行</button>`
    : "";
  return (
    `<div class="md-code-block${collapsed}" data-lang="${raw}" data-lines="${lines}"${uncolored ? ' data-uncolored="1"' : ""}>` +
    `<div class="md-code-head"><span class="md-code-lang">${label}</span>` +
    `<button class="md-code-copy" type="button">复制</button></div>` +
    `<pre class="markdown-body-code-pre"><code class="hljs${raw ? ` language-${raw}` : ""}">${codeHtml}</code></pre>` +
    toggle +
    `</div>`
  );
};

// 容错预处理见 utils/markdown.ts：愈合不合规的分隔行（模型吐空 pipe / 列数不足 /
// em-dash / 单元格漏 -）+ 给表前补空行，避免 GFM 表格整表退化成普通段落。
/** 渲染入口（原 md.render monkey-patch 收编为显式函数——env 需显式传递）。 */
function renderMarkdown(src: string, streaming: boolean): string {
  const pre = preprocessMarkdown(src);
  const env: MdEnv = {};
  if (streaming) {
    const line = findOpenFenceLine(pre);
    if (line >= 0) env.mdOpenFenceLine = line;
  }
  let html = md.render(pre, env);
  // 移除 <li> 内直属 <p> 标签（无论后面是否有嵌套标签）
  // markdown-it 生成 <li><p>content</p></li> 或 <li><p>content</p><ul>...</ul></li>
  // <p> 的 display:block 导致序号与内容在不同行
  html = html.replace(/<li>\s*<p>([\s\S]*?)<\/p>/g, '<li>$1');
  // 表格包一层滚动 wrapper：避免给 <table> 设 display:block（会拆散 thead/tbody、
  // 列宽错位）；改为 wrapper 承担 overflow-x，table 保持原生表格布局。
  // ≥4 列加 md-table-wide：宽表破列标记（markdown.css 宿主规则放宽气泡 85% 上限、
  // 表格转 max-content 自然宽；DS 系同构阈值）。
  html = html.replace(/<table[^>]*>[\s\S]*?<\/table>/g, (tableHtml) => {
    const cols = (tableHtml.match(/<th[\s>]/g) ?? []).length;
    const wide = cols >= 4 ? " md-table-wide" : "";
    return `<div class="markdown-table-wrap${wide}">${tableHtml}</div>`;
  });
  return html;
}

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

// 冻结感知的渲染状态（替代 computed）：页面不可见（keep-alive deactivated /
// 窗口最小化）时跳过重渲染——隐藏实例的响应式 effect 仍然活着，每 tick 的
// 全量 md.render + innerHTML 重建是「生成中切设置页卡顿」的隐形主线程源。
// 解冻时统一补一次（含回合在隐藏期间完成的情况）。
const html = ref("");
function rerender() {
  html.value = props.content ? renderMarkdown(props.content, !!props.streaming) : "";
}
watch(
  () => [props.content, props.streaming] as const,
  () => {
    if (liveRenderFrozen.value) return;
    rerender();
  },
  { immediate: true },
);
watch(liveRenderFrozen, (frozen) => {
  if (!frozen) rerender();
});
</script>

<template>
  <!-- eslint-disable vue/no-v-html -- markdown-it 输出已清理，需 v-html 渲染 -->
  <div
    ref="rootRef"
    :class="['markdown-body', { 'md-streaming': streaming }]"
    @click="onRootClick"
    v-html="html"
  />
  <!-- eslint-enable vue/no-v-html -->
</template>
