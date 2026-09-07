<!--
  ToolExpandDetail — 工具调用展开详情（ChatMessages 工具行展开区的内容层）

  按工具名分派结构化渲染（2026-09-07 拍板，行是摘要、展开是详情、原文永远可达）：
  - edit_file   红绿对照（old/new 行级对照，非字符级 diff）
  - write_file  写入内容预览
  - edit_docx   逐操作清单（AppliedOp 摘要）
  - write_docx  块清单（分类计数）
  - validate_docx 断言清单（失败明细红 + 通过聚合）
  - 其余（含 P1 的读侧与非 P1 工具）= 现状两块（参数 + 结果）

  底部恒有「原始 JSON」折叠（解析降级 / 排障 / 透明度兜底）。
  错误态（result.isError）：任何分派落回「参数 + 错误」形态，结构化分支不渲染。

  Props: name / argsJson / result（与 summarizeToolCall 同源数据）
  Emits: 无
-->
<script setup lang="ts">
import { ref, computed } from "vue";
import { formatFileSize, formatJson } from "../../utils/format";

const props = defineProps<{
  name: string;
  argsJson: string;
  result: { content: string; isError: boolean } | null;
}>();

const rawOpen = ref(false);

function parseJson(s: string): Record<string, unknown> | null {
  try {
    const o: unknown = JSON.parse(s);
    return typeof o === "object" && o !== null ? (o as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

const args = computed(() => parseJson(props.argsJson));
const isError = computed(() => props.result?.isError === true);
const resultJson = computed(() =>
  props.result && !props.result.isError ? parseJson(props.result.content) : null,
);

function strArg(key: string): string | null {
  const v = args.value?.[key];
  return typeof v === "string" ? v : null;
}

function numResult(key: string): number | null {
  const v = resultJson.value?.[key];
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

// ---- edit_file：红绿对照 ----
const editFileView = computed(() => {
  if (isError.value || props.name !== "edit_file" || !args.value) return null;
  return {
    old: strArg("old_string") ?? "",
    newStr: strArg("new_string") ?? "",
    replaceAll: args.value.replace_all === true,
    count: numResult("replacements"),
  };
});

// ---- write_file：写入内容预览 ----
const writeFileView = computed(() => {
  if (isError.value || props.name !== "write_file" || !args.value) return null;
  const bytes = numResult("bytes_written");
  return {
    content: strArg("content") ?? "",
    // backup 显式 null = 新建；字符串 = 覆盖（与 toolSummary 同判据）
    created: !(resultJson.value && "backup" in resultJson.value && resultJson.value.backup != null),
    bytes,
  };
});

// ---- edit_docx：逐操作清单 ----
/** AppliedOp.op → 中文（词表外裸透原值） */
const EDIT_DOCX_OP_LABELS: Record<string, string> = {
  replace_text: "替换文本",
  insert_paragraph_after: "插入段落",
  delete_block: "删除块",
  set_format: "设格式",
  set_style: "设样式",
  set_ppr_element: "段落属性",
  set_table_element: "表格属性",
  set_cell_format: "格格式",
  merge_cells: "合并单元格",
  split_cell: "拆分单元格",
  delete_table_row: "删表格行",
  insert_table_after: "插入表格",
  insert_toc_after: "插入目录",
  insert_image_after: "插入图片",
  clear_body: "清空正文",
  create_style: "新建样式",
  set_style_element: "改样式定义",
  set_numbering_element: "改编号定义",
};

interface AppliedOpView {
  label: string;
  block: number | null;
  target: string | null;
  before: string;
  after: string;
}

const editDocxView = computed(() => {
  if (isError.value || props.name !== "edit_docx" || !resultJson.value) return null;
  const rawOps = resultJson.value.operations;
  if (!Array.isArray(rawOps)) return null;
  const ops: AppliedOpView[] = rawOps
    .filter((o): o is Record<string, unknown> => typeof o === "object" && o !== null)
    .map((o) => ({
      label: EDIT_DOCX_OP_LABELS[String(o.op)] ?? String(o.op),
      block: typeof o.block === "number" ? o.block : null,
      target: typeof o.target === "string" ? o.target : null,
      before: typeof o.before === "string" ? o.before : "",
      after: typeof o.after === "string" ? o.after : "",
    }));
  return { ops, applied: numResult("applied") };
});

// ---- write_docx：块清单 ----
const writeDocxView = computed(() => {
  if (isError.value || props.name !== "write_docx" || !resultJson.value) return null;
  const parts: string[] = [];
  const paragraphs = numResult("paragraphs");
  const tables = numResult("tables");
  const images = numResult("images");
  const tocs = numResult("tocs");
  if (paragraphs) parts.push(`段落 ${paragraphs}`);
  if (tables) parts.push(`表格 ${tables}`);
  if (images) parts.push(`图片 ${images}`);
  if (tocs) parts.push(`目录 ${tocs}`);
  return {
    blocks: numResult("blocks") ?? (Array.isArray(args.value?.blocks) ? args.value.blocks.length : null),
    breakdown: parts.length > 0 ? parts.join(" · ") : null,
    template: strArg("template") ?? (typeof resultJson.value.template === "string" ? String(resultJson.value.template) : null),
    // created 显式布尔（新建时 backup 字段整体缺省，与 write_file 判据不同）
    created: resultJson.value.created === true,
    bytes: numResult("bytes"),
  };
});

// ---- validate_docx：断言清单 ----
/** 断言 kind → 中文（词表外裸透原值） */
const VALIDATE_KIND_LABELS: Record<string, string> = {
  block_count: "块数",
  table_shape: "表形状",
  block_text: "块文本",
  block_style: "块样式",
  cell_text: "格文本",
  cell_paragraph_count: "格段落数",
  block_image: "块图片",
  block_field: "块域",
};

const validateDocxView = computed(() => {
  if (isError.value || props.name !== "validate_docx" || !resultJson.value) return null;
  const failures = Array.isArray(resultJson.value.failures)
    ? resultJson.value.failures
        .filter((f): f is Record<string, unknown> => typeof f === "object" && f !== null)
        .map((f) => ({
          kind: VALIDATE_KIND_LABELS[String(f.kind)] ?? String(f.kind),
          target: typeof f.target === "string" ? f.target : "",
          detail: typeof f.detail === "string" ? f.detail : "",
        }))
    : [];
  const passedKinds = Array.isArray(resultJson.value.passed_kinds)
    ? resultJson.value.passed_kinds.filter((k): k is string => typeof k === "string")
    : [];
  return {
    total: numResult("total"),
    failed: numResult("failed"),
    failures,
    passedKinds,
  };
});
</script>

<template>
  <!-- 错误态：参数 + 错误（现状形态），结构化分支不渲染 -->
  <template v-if="isError">
    <div class="expand-group">
      <div class="expand-hdr">参数</div>
      <pre class="expand-code">{{ formatJson(argsJson) }}</pre>
    </div>
    <div v-if="result" class="expand-group">
      <div class="expand-hdr hdr-err">错误</div>
      <pre class="expand-code code-err">{{ result.content }}</pre>
    </div>
  </template>

  <template v-else>
    <!-- edit_file：红绿对照 -->
    <template v-if="editFileView">
      <div class="diff-block diff-del">
        <div class="expand-hdr">替换前</div>
        <pre class="expand-code">{{ editFileView.old }}</pre>
      </div>
      <div class="diff-block diff-add">
        <div class="expand-hdr">替换后</div>
        <pre class="expand-code">{{ editFileView.newStr }}</pre>
      </div>
      <div v-if="editFileView.count != null" class="expand-meta">
        替换 {{ editFileView.count }} 处{{ editFileView.replaceAll ? "（全部）" : "" }}
      </div>
    </template>

    <!-- write_file：写入内容预览 -->
    <template v-else-if="writeFileView">
      <div class="expand-group">
        <div class="expand-hdr">写入内容</div>
        <pre class="expand-code">{{ writeFileView.content }}</pre>
      </div>
      <div class="expand-meta">
        {{ writeFileView.created ? "新建" : "覆盖" }}{{ writeFileView.bytes != null ? ` · ${formatFileSize(writeFileView.bytes)}` : "" }}
      </div>
    </template>

    <!-- edit_docx：逐操作清单 -->
    <template v-else-if="editDocxView">
      <div class="expand-group">
        <div class="expand-hdr">{{ editDocxView.applied != null ? `${editDocxView.applied} 处操作` : "操作清单" }}</div>
        <div class="op-list">
          <div v-for="(op, i) in editDocxView.ops" :key="i" class="op-row">
            <span class="op-tag">{{ op.label }}</span>
            <span class="op-target">{{ op.target ?? (op.block != null ? `块 ${op.block}` : "") }}</span>
            <span class="op-change">{{ op.before }}<template v-if="op.after"> → {{ op.after }}</template></span>
          </div>
        </div>
      </div>
    </template>

    <!-- write_docx：块清单 -->
    <template v-else-if="writeDocxView">
      <div class="expand-group">
        <div class="expand-hdr">
          {{ writeDocxView.created ? "新建" : "覆盖" }}{{ writeDocxView.blocks != null ? ` · ${writeDocxView.blocks} 块` : "" }}{{ writeDocxView.bytes != null ? ` · ${formatFileSize(writeDocxView.bytes)}` : "" }}
        </div>
        <div v-if="writeDocxView.breakdown" class="expand-meta">{{ writeDocxView.breakdown }}</div>
        <div v-if="writeDocxView.template" class="expand-meta">模板：{{ writeDocxView.template }}</div>
      </div>
    </template>

    <!-- validate_docx：断言清单 -->
    <template v-else-if="validateDocxView">
      <div class="expand-group">
        <div class="expand-hdr">
          {{ validateDocxView.failed === 0 ? `全部通过（${validateDocxView.total ?? "?"} 项）` : `${validateDocxView.failed ?? "?"} 项未过 / 共 ${validateDocxView.total ?? "?"} 项` }}
        </div>
        <div v-if="validateDocxView.failures.length > 0" class="op-list">
          <div v-for="(f, i) in validateDocxView.failures" :key="i" class="op-row op-fail">
            <span class="op-tag">{{ f.kind }}</span>
            <span class="op-target">{{ f.target }}</span>
            <span class="op-change">{{ f.detail }}</span>
          </div>
        </div>
        <div v-if="validateDocxView.passedKinds.length > 0" class="expand-meta">
          通过：{{ validateDocxView.passedKinds.join(" · ") }}
        </div>
      </div>
    </template>

    <!-- 兜底：现状两块（参数 + 结果） -->
    <template v-else>
      <div class="expand-group">
        <div class="expand-hdr">参数</div>
        <pre class="expand-code">{{ formatJson(argsJson) }}</pre>
      </div>
      <div v-if="result" class="expand-group">
        <div class="expand-hdr">结果</div>
        <pre class="expand-code">{{ result.content }}</pre>
      </div>
      <div v-else class="expand-pending">等待执行结果…</div>
    </template>
  </template>

  <!-- 原始 JSON 折叠（所有分支底部恒有——解析降级 / 排障 / 透明度兜底） -->
  <button class="raw-toggle" type="button" @click="rawOpen = !rawOpen">
    {{ rawOpen ? "收起原始 JSON" : "原始 JSON" }}
  </button>
  <template v-if="rawOpen">
    <div class="expand-group">
      <div class="expand-hdr">参数（原始）</div>
      <pre class="expand-code">{{ argsJson }}</pre>
    </div>
    <div v-if="result" class="expand-group">
      <div class="expand-hdr">结果（原始）</div>
      <pre class="expand-code">{{ result.content }}</pre>
    </div>
  </template>
</template>

<style scoped>
.expand-group { margin-bottom: 8px; }
.expand-group:last-child { margin-bottom: 0; }
.expand-hdr { font-size: var(--ip-text-micro-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-tertiary); margin-bottom: 4px; letter-spacing: 0.5px; }
.expand-hdr.hdr-err { color: var(--ip-danger-base); }
.expand-code { font-size: var(--ip-text-caption-size); font-family: var(--ip-font-mono, monospace); white-space: pre-wrap; word-break: break-word; color: var(--ip-code-text); background: var(--ip-code-bg); padding: 6px 8px; border-radius: var(--ip-radius-sm); margin: 0; line-height: 1.5; max-height: 200px; overflow-y: auto; }
.expand-code.code-err { color: var(--ip-danger-base); }
.expand-pending { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); font-style: italic; }
.expand-meta { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); padding-top: 4px; }

/* 红绿对照（语义层 danger/success，浅暗双主题自适应） */
.diff-block { margin-bottom: 8px; border-radius: var(--ip-radius-sm); padding: 6px 8px; }
.diff-block .expand-hdr { margin-bottom: 2px; }
.diff-block .expand-code { background: transparent; padding: 0; max-height: 160px; }
.diff-del { background: color-mix(in srgb, var(--ip-danger-base) 8%, transparent); }
.diff-del .expand-hdr, .diff-del .expand-code { color: var(--ip-danger-base); }
.diff-add { background: color-mix(in srgb, var(--ip-success-base) 8%, transparent); }
.diff-add .expand-hdr, .diff-add .expand-code { color: var(--ip-success-base); }

/* 逐操作 / 断言清单 */
.op-list { display: flex; flex-direction: column; gap: 4px; }
.op-row { display: flex; align-items: baseline; gap: var(--ip-spacing-2); min-width: 0; font-size: var(--ip-text-caption-size); line-height: 1.5; }
.op-tag { flex: none; padding: 0 6px; border-radius: var(--ip-radius-full); background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-secondary); font-size: var(--ip-text-micro-size); white-space: nowrap; }
.op-target { flex: none; color: var(--ip-color-text-secondary); font-family: var(--ip-font-mono, monospace); font-size: var(--ip-text-micro-size); white-space: nowrap; }
.op-change { min-width: 0; color: var(--ip-color-text-tertiary); word-break: break-all; }
.op-fail .op-tag { background: color-mix(in srgb, var(--ip-danger-base) 10%, transparent); color: var(--ip-danger-base); }
.op-fail .op-change { color: var(--ip-danger-base); }

/* 原始 JSON 折叠 */
.raw-toggle { margin-top: 8px; padding: 0; border: none; background: transparent; cursor: pointer; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-disabled); text-decoration: underline dotted; }
.raw-toggle:hover { color: var(--ip-color-text-secondary); }
</style>
