<!--
  MarkdownRenderer — Markdown → HTML 渲染（markdown-it + highlight.js）

  Props: content: string（原始 Markdown 文本）
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

const rootRef = ref<HTMLElement | null>(null);

function onRootClick(e: MouseEvent) {
  const link = (e.target as HTMLElement)?.closest?.("a");
  if (!link || !link.href) return;
  // 只拦截外部链接，不拦截锚点
  if (link.href.startsWith("http://") || link.href.startsWith("https://")) {
    e.preventDefault();
    e.stopPropagation();
    openUrl(link.href);
  }
}

// 高亮函数（独立于 md 实例，避免循环引用 TS 错误）
function highlightCode(str: string, lang: string): string {
  if (lang && hljs.getLanguage(lang)) {
    try {
      return (
        '<pre class="markdown-body-code-pre"><code class="hljs language-' +
        lang +
        '">' +
        hljs.highlight(str, { language: lang, ignoreIllegals: true }).value +
        "</code></pre>"
      );
    } catch {
      // fallback
    }
  }
  // 无语言或不支持 → 不着色
  return '<pre class="markdown-body-code-pre"><code class="markdown-body-code">' + hljs.highlightAuto(str).value + '</code></pre>';
}

const md = new MarkdownIt({
  html: false,
  linkify: true,
  typographer: true,
  breaks: true,
  highlight: highlightCode,
});

// 容错预处理：模型常在「列表项 / 引用 / 普通段落」之后不加空行直接接 GFM 表格，
// markdown-it 的 lazy-continuation 会把表格头行吞进上一段（再叠加 typographer 把
// 分隔行里的 --- 转成破折号 —），最终渲染成「竖线 + 破折号」的乱码文本而非表格。
// 此处在解析前给「紧跟在非表格行之后的表格」补一个空行，让 table 规则正确触发。
// （独立成段、前后已有空行的表格不受影响。）覆盖最常见的内容质量问题。
function preprocessTables(src: string): string {
  // $1 = 普通文本行末字符 + 换行；$2 = 表头行 + 分隔行（GFM 表格最小骨架）
  return src.replace(/([^\n|]\n)(\|[^\n]*\|\s*\n\|[\s:|-]+\|)/g, "$1\n$2");
}

// 列表项内 <p> 标签清理（保持序号与内容同行）
// markdown-it 会在 <li> 内包裹 <p>，导致 display:block 换行
// 在渲染后把 <li> 直属的 <p> 展开为纯文本，保留内部 HTML
const origRender = md.render.bind(md);
md.render = function (src: string): string {
  let html = origRender(preprocessTables(src));
  // 移除 <li> 内直属 <p> 标签（无论后面是否有嵌套标签）
  // markdown-it 生成 <li><p>content</p></li> 或 <li><p>content</p><ul>...</ul></li>
  // <p> 的 display:block 导致序号与内容在不同行
  html = html.replace(/<li>\s*<p>([\s\S]*?)<\/p>/g, '<li>$1');
  // 表格包一层滚动 wrapper：避免给 <table> 设 display:block（会拆散 thead/tbody、
  // 列宽错位）；改为 wrapper 承担 overflow-x，table 保持原生表格布局。
  html = html.replace(/<table[^>]*>[\s\S]*?<\/table>/g, '<div class="markdown-table-wrap">$&</div>');
  return html;
};

const props = defineProps<{
  content: string;
}>();

const rendered = computed(() => {
  if (!props.content) return "";
  return md.render(props.content);
});
</script>

<template>
  <!-- eslint-disable-next-line vue/no-v-html -- markdown-it 输出已清理，需 v-html 渲染 -->
  <div ref="rootRef" class="markdown-body" @click="onRootClick" v-html="rendered" />
</template>
