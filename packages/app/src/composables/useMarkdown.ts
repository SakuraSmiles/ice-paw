// Markdown 渲染 composable
//
// 设计：
//   - 模块单例 markdown-it 实例，整个应用共享一份渲染器
//   - 安全：html: false 阻断原始 HTML 防 XSS（markdown-it 内置 link 协议白名单已拦截 javascript:）
//   - 体验：linkify 自动识别 URL、breaks 把单换行转 <br>
//   - 代码块：内置 highlight.js（精简语言集）做语法高亮
//   - 外部链接：自动追加 target="_blank" rel="noopener noreferrer"
//   - 流式容错：未闭合的 ``` 围栏不会让后续正文被吞进 code 块
//
// 用法：
//   import { useMarkdown } from "@/composables/useMarkdown";
//   const { renderMarkdown } = useMarkdown();
//   const html = renderMarkdown("**hello**");

import MarkdownIt from "markdown-it";
import hljs from "highlight.js/lib/core";

// 仅注册 AI 响应里常见的语言，避免把全部 380+ 语言都塞进 bundle
import javascript from "highlight.js/lib/languages/javascript";
import typescript from "highlight.js/lib/languages/typescript";
import python from "highlight.js/lib/languages/python";
import json from "highlight.js/lib/languages/json";
import bash from "highlight.js/lib/languages/bash";
import xml from "highlight.js/lib/languages/xml";
import css from "highlight.js/lib/languages/css";
import scss from "highlight.js/lib/languages/scss";
import yaml from "highlight.js/lib/languages/yaml";
import sql from "highlight.js/lib/languages/sql";
import markdown from "highlight.js/lib/languages/markdown";
import rust from "highlight.js/lib/languages/rust";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import cLike from "highlight.js/lib/languages/c-like";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import ruby from "highlight.js/lib/languages/ruby";
import php from "highlight.js/lib/languages/php";
import plaintext from "highlight.js/lib/languages/plaintext";

hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("js", javascript);
hljs.registerLanguage("jsx", javascript);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("ts", typescript);
hljs.registerLanguage("tsx", typescript);
hljs.registerLanguage("python", python);
hljs.registerLanguage("py", python);
hljs.registerLanguage("json", json);
hljs.registerLanguage("bash", bash);
hljs.registerLanguage("sh", bash);
hljs.registerLanguage("shell", bash);
hljs.registerLanguage("zsh", bash);
hljs.registerLanguage("html", xml);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("svg", xml);
hljs.registerLanguage("css", css);
hljs.registerLanguage("scss", scss);
hljs.registerLanguage("yaml", yaml);
hljs.registerLanguage("yml", yaml);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("md", markdown);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("rs", rust);
hljs.registerLanguage("go", go);
hljs.registerLanguage("java", java);
hljs.registerLanguage("c", cLike);
hljs.registerLanguage("cpp", cpp);
hljs.registerLanguage("c++", cpp);
hljs.registerLanguage("csharp", csharp);
hljs.registerLanguage("cs", csharp);
hljs.registerLanguage("ruby", ruby);
hljs.registerLanguage("rb", ruby);
hljs.registerLanguage("php", php);
hljs.registerLanguage("plaintext", plaintext);
hljs.registerLanguage("text", plaintext);
hljs.registerLanguage("txt", plaintext);

// ============================================================================
// markdown-it 实例（模块单例）
// ============================================================================

const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: true,
  typographer: false,
});

// ============================================================================
// 自定义渲染规则
// ============================================================================

// --- 外部链接追加 target/rel -------------------------------------------------

const defaultLinkOpen =
  md.renderer.rules.link_open ||
  ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));

md.renderer.rules.link_open = function (tokens, idx, options, env, self) {
  const token = tokens[idx];
  const href = token.attrGet("href") ?? "";
  // 带协议的链接视为外部（http/https/mailto/tel 等），相对路径与锚点保持原样
  if (/^[a-z][a-z0-9+\-.]*:/i.test(href)) {
    token.attrSet("target", "_blank");
    token.attrSet("rel", "noopener noreferrer");
  }
  return defaultLinkOpen(tokens, idx, options, env, self);
};

// --- 代码块：highlight.js 高亮 ----------------------------------------------

md.renderer.rules.fence = function (tokens, idx) {
  const token = tokens[idx];
  const info = (token.info || "").trim();
  // 围栏语言标识可能附加元数据，例如 ```ts {1,3-5} title="x.ts"，只取首个 token
  const langName = info ? info.split(/\s+/)[0] : "";
  const code = token.content.replace(/\n$/, "");

  let highlighted: string;
  let langClass = "";

  if (langName && hljs.getLanguage(langName)) {
    try {
      highlighted = hljs.highlight(code, {
        language: langName,
        ignoreIllegals: true,
      }).value;
      langClass = ` language-${langName}`;
    } catch {
      // 高亮失败 → 转义后作为纯文本
      highlighted = md.utils.escapeHtml(code);
      if (langName) langClass = ` language-${langName}`;
    }
  } else {
    // 未注册或无语言 → 不做高亮，仅转义
    highlighted = md.utils.escapeHtml(code);
    if (langName) langClass = ` language-${langName}`;
  }

  return (
    `<pre class="hljs markdown-body-code-pre"><code class="hljs${langClass} markdown-body-code">` +
    `${highlighted}</code></pre>`
  );
};

// --- 行内代码：加 class 以便样式命名空间隔离 --------------------------------

md.renderer.rules.code_inline = function (tokens, idx) {
  const token = tokens[idx];
  return `<code class="markdown-body-code-inline">${md.utils.escapeHtml(token.content)}</code>`;
};

// ============================================================================
// 流式容错
// ============================================================================

/**
 * 修补未闭合的 ``` 围栏，避免 markdown-it 把后续正文吞进 code 块。
 * 只针对反引号围栏（4 空格缩进代码块在 AI 响应里几乎不出现）。
 *
 * 统计规则：
 *   - 行首出现 ``` 计一次（开/闭围栏都计）
 *   - 出现奇数次 → 当前处于已开启未关闭状态
 */
function repairUnclosedFences(text: string): string {
  const matches = text.match(/^```.*$/gm);
  if (!matches) return text;
  if (matches.length % 2 === 1) {
    return text + "\n```";
  }
  return text;
}

// ============================================================================
// composable 导出
// ============================================================================

/**
 * Markdown 渲染 composable。
 *
 * 返回：
 *   - md              markdown-it 实例（高级场景使用）
 *   - renderMarkdown  字符串 → HTML 字符串
 */
export function useMarkdown() {
  function renderMarkdown(text: string): string {
    if (!text) return "";
    const safe = repairUnclosedFences(text);
    return md.render(safe);
  }

  return {
    /** markdown-it 实例（高级场景使用，例如注入自定义 token 规则） */
    md,
    /** 渲染 Markdown 字符串为 HTML 字符串 */
    renderMarkdown,
  };
}

export default useMarkdown;