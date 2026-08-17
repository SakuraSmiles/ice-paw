<!--
  TrajectoryInspector — 右侧详情面板（dsh 局部检查器，标签分区）

  按需展现：父级仅在选中行时渲染本组件（无空态占位），✕/Esc/再点同一行即收起。
  统一骨架：头（kind 徽章 + seq/actor/时间）→ 标签条 → 内容。
  - 概要：所有记录必有——类型专属的速览（chips / 键值 / 块构成）
  - 中间标签：按 kind 与载荷按需出现（思考/正文/参数/结果/明细…），大块内容各占一页
  - 原始数据：所有记录必有——raw JSON + 复制按钮（审计兜底）
  换行时记住当前标签（仍适用则不跳），不适用回落「概要」。
-->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type {
  AssistantMessagePayload,
  ContentBlock,
  HookInjectedPayload,
  MessageDiscardedPayload,
  MessageErrorPayload,
  ModalAdaptedPayload,
  PlanUpdatedPayload,
  SummaryPayload,
  ToolExecutionPayload,
  UserMessagePayload,
} from "../../types";
import type { TrajectoryRow } from "../../composables/useTrajectory";
import MarkdownRenderer from "../chat/MarkdownRenderer.vue";
import ImagePreview from "../chat/ImagePreview.vue";
import { termLabel } from "../../utils/termLabels";

const props = defineProps<{ row: TrajectoryRow }>();
const emit = defineEmits<{ close: [] }>();

const ev = computed(() => (props.row?.type === "event" ? props.row : null));
const header = computed(() => (props.row?.type === "turn-header" ? props.row : null));

// ---- 类型化载荷（kind 收窄后断言一次，模板零 cast） ----
const userP = computed(() => (ev.value?.event.kind === "user_message" ? (ev.value.event.payload as UserMessagePayload) : null));
const asstP = computed(() => (ev.value?.event.kind === "assistant_message" ? (ev.value.event.payload as AssistantMessagePayload) : null));
const toolP = computed(() => (ev.value?.event.kind === "tool_execution" ? (ev.value.event.payload as ToolExecutionPayload) : null));
const sumP = computed(() =>
  ev.value && (ev.value.event.kind === "summary_created" || ev.value.event.kind === "summary_updated")
    ? (ev.value.event.payload as SummaryPayload)
    : null,
);
const errP = computed(() => (ev.value?.event.kind === "message_error" ? (ev.value.event.payload as MessageErrorPayload) : null));
const discP = computed(() => (ev.value?.event.kind === "message_discarded" ? (ev.value.event.payload as MessageDiscardedPayload) : null));
const modalP = computed(() => (ev.value?.event.kind === "modal_adapted" ? (ev.value.event.payload as ModalAdaptedPayload) : null));
const hookP = computed(() => (ev.value?.event.kind === "hook_injected" ? (ev.value.event.payload as HookInjectedPayload) : null));
const planP = computed(() => (ev.value?.event.kind === "plan_updated" ? (ev.value.event.payload as PlanUpdatedPayload) : null));
const attachP = computed(() =>
  ev.value?.event.kind === "attachment_stored"
    ? (ev.value.event.payload as { kind?: string; items?: { idx: number; name: string; kind: string; label?: string; token_est?: number }[] })
    : null,
);

type ThinkingBlock = Extract<ContentBlock, { type: "thinking" }>;
type ImageBlock = Extract<ContentBlock, { type: "image" }>;

const thinkings = computed<ThinkingBlock[]>(() => (asstP.value?.blocks ?? []).filter((b): b is ThinkingBlock => b.type === "thinking"));
const userImages = computed<{ data: string; mediaType: string }[]>(() =>
  (userP.value?.blocks ?? []).filter((b): b is ImageBlock => b.type === "image").map((b) => ({ data: b.data, mediaType: b.media_type })),
);

/** assistant 概要：块构成速览（这条消息里装了什么） */
const blockComposition = computed(() => {
  const blocks = asstP.value?.blocks ?? [];
  const n: Record<string, number> = {};
  for (const b of blocks) n[b.type] = (n[b.type] ?? 0) + 1;
  const parts: string[] = [];
  if (n.thinking) parts.push(`思考 ×${n.thinking}`);
  if (n.text) parts.push(`文本 ×${n.text}`);
  if (n.tool_use) parts.push(`工具调用 ×${n.tool_use}`);
  if (n.image) parts.push(`图片 ×${n.image}`);
  return parts.join(" · ");
});

const previewStart = ref(0);
const showPreview = ref(false);
function openImage(i: number) {
  previewStart.value = i;
  showPreview.value = true;
}

function prettyJson(s: string | null | undefined): string {
  if (s == null) return "";
  try {
    return JSON.stringify(JSON.parse(s), null, 2);
  } catch {
    return s;
  }
}

function rawPayload(row: TrajectoryRow): string {
  if (row.type === "event") return JSON.stringify(row.event.payload, null, 2);
  return JSON.stringify({ context: row.context, ended: row.ended }, null, 2);
}

function fullTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/** 头部紧凑时间（MM-DD HH:MM:SS，等宽对齐；完整时间在概要里） */
function compactTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

// ---- 概要页预览素材 ----
/** 多行预览：保留换行（行数由 CSS line-clamp 收口），仅按字符量兜底截断 */
function previewOf(s: string | null | undefined, max = 400): string {
  const t = (s ?? "").trim();
  if (!t) return "";
  return t.length > max ? `${t.slice(0, max)}…` : t;
}

function compactJsonOf(s: string | null | undefined, max = 80): string {
  if (s == null) return "";
  const t = s.replace(/\s+/g, " ").trim();
  return t.length > max ? `${t.slice(0, max)}…` : t;
}

/** tool 结果速览：成功/失败 + 字符量 */
const toolResultState = computed(() => {
  const p = toolP.value;
  if (!p || p.result == null) return null;
  return `${p.is_error ? "失败" : "成功"} · ${String(p.result).length} 字符`;
});

/** modal_adapted outcome 统计（ocr ×2 · stripped ×1） */
const modalOutcomes = computed(() => {
  const n: Record<string, number> = {};
  for (const it of modalP.value?.items ?? []) n[it.outcome] = (n[it.outcome] ?? 0) + 1;
  return Object.entries(n)
    .map(([k, c]) => `${k} ×${c}`)
    .join(" · ");
});

// ---- 标签模型：概要 常驻头 + 原始数据 常驻尾，中间按 kind 按需 ----
interface InspTab {
  id: string;
  label: string;
  badge?: string;
}

const tabs = computed<InspTab[]>(() => {
  if (!props.row) return [];
  if (header.value) return [{ id: "summary", label: "概要" }, { id: "payload", label: "原始数据" }];
  const mid: InspTab[] = [];
  switch (ev.value?.event.kind) {
    case "assistant_message":
      if (thinkings.value.length) mid.push({ id: "thinking", label: "思考", badge: String(thinkings.value.length) });
      mid.push({ id: "text", label: "正文" });
      break;
    case "user_message":
      mid.push({ id: "text", label: "正文" });
      break;
    case "tool_execution":
      mid.push({ id: "params", label: "参数" });
      if (toolP.value?.result != null) mid.push({ id: "result", label: "结果" });
      break;
    case "summary_created":
    case "summary_updated":
      mid.push({ id: "content", label: "摘要正文" });
      break;
    case "message_error":
      mid.push({ id: "detail", label: "错误详情" });
      break;
    case "message_discarded":
      mid.push({ id: "detail", label: "原因" });
      break;
    case "plan_updated":
      mid.push({ id: "plan", label: "计划清单", badge: String(planP.value?.items.length ?? 0) });
      break;
    case "modal_adapted":
      mid.push({ id: "detail", label: "明细" });
      break;
    case "hook_injected":
      mid.push({ id: "detail", label: "注入内容" });
      break;
    case "attachment_stored":
      mid.push({ id: "detail", label: "明细" });
      break;
  }
  return [{ id: "summary", label: "概要" }, ...mid, { id: "payload", label: "原始数据" }];
});

/** 浏览粘性：换行后当前标签仍适用则不跳（连续翻多条工具行可停留在「结果」页） */
const activeTab = ref("summary");
watch(
  () => props.row,
  () => {
    if (!tabs.value.some((t) => t.id === activeTab.value)) activeTab.value = "summary";
  },
);

const copied = ref(false);
async function copyPayload() {
  if (!props.row) return;
  try {
    await navigator.clipboard.writeText(rawPayload(props.row));
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    /* clipboard 不可用（权限/环境）时静默 */
  }
}
</script>

<template>
  <aside class="insp">
    <header class="insp-head">
      <span v-if="ev" class="insp-kind" :class="`ev-${ev.kind}`">{{ ev.label }}</span>
      <span v-else-if="header" class="insp-kind insp-kind-head">{{ header.turnId ? `第 ${header.turnIndex + 1} 轮` : "纪元前事件" }}</span>
      <span v-if="ev" class="insp-meta">#{{ ev.seq }} · {{ ev.event.actor }} · {{ compactTime(ev.createdAt) }}</span>
      <button class="insp-close" title="关闭" @click="emit('close')">✕</button>
    </header>

    <nav class="insp-tabs">
      <button
        v-for="t in tabs"
        :key="t.id"
        class="insp-tab"
        :class="{ active: activeTab === t.id }"
        @click="activeTab = t.id"
      >
        {{ t.label }}<span v-if="t.badge" class="itab-badge">{{ t.badge }}</span>
      </button>
    </nav>

    <div class="insp-body">
      <!-- ============ 概要：所有记录必有 ============ -->
      <template v-if="activeTab === 'summary'">
        <!-- turn 头：配置快照 + 终止/用量 -->
        <template v-if="header">
          <section v-if="header.context" class="isec">
            <h4 class="isec-title">本轮配置快照</h4>
            <div class="ikv"><span>模型</span><b>{{ header.context.effective_model }}</b></div>
            <div class="ikv"><span>Provider</span><b>{{ header.context.provider }}</b></div>
            <div v-if="header.context.model_override" class="ikv"><span>覆盖模型</span><b>{{ header.context.model_override }}</b></div>
            <div class="ikv"><span>工具</span><b>{{ header.context.tools_enabled ? `开（${header.context.tool_names.length} 个）` : "关" }}</b></div>
            <details v-if="header.context.tool_names.length" class="isec-sub">
              <summary>工具清单</summary>
              <div class="itags">
                <span v-for="tn in header.context.tool_names" :key="tn" class="itag">{{ tn }}</span>
              </div>
            </details>
            <div v-if="header.context.temperature != null" class="ikv"><span>温度</span><b>{{ header.context.temperature }}</b></div>
            <div v-if="header.context.max_tokens != null" class="ikv"><span>输出上限</span><b>{{ header.context.max_tokens }}</b></div>
            <div v-if="header.context.tool_max_rounds != null" class="ikv"><span>工具轮数上限</span><b>{{ header.context.tool_max_rounds }}</b></div>
            <div v-if="header.context.budget_max_tokens != null" class="ikv"><span>预算</span><b>{{ header.context.budget_max_tokens }}</b></div>
            <div v-if="header.context.context_window != null" class="ikv"><span>上下文窗口</span><b>{{ header.context.context_window }}</b></div>
          </section>
          <section class="isec">
            <h4 class="isec-title">终止与用量</h4>
            <div class="ikv"><span>起始时间</span><b>{{ fullTime(header.createdAt) }}</b></div>
            <div v-if="header.ended" class="ikv">
              <span>终止原因</span><b>{{ termLabel(header.ended.termination) }}（{{ header.ended.rounds }} 轮）</b>
            </div>
            <div v-else class="insp-muted">未记录 turn_ended（进行中 / 崩溃未收尾）</div>
            <div v-if="header.ended?.usage" class="ikv">
              <span>Token</span>
              <b>输入 {{ header.ended.usage.prompt_tokens }} · 输出 {{ header.ended.usage.completion_tokens }}<template v-if="header.ended.usage.cached_tokens"> · 缓存 {{ header.ended.usage.cached_tokens }}</template></b>
            </div>
            <div v-if="header.ended?.user_token_count != null" class="ikv"><span>用户消息 token</span><b>{{ header.ended.user_token_count }}</b></div>
            <div class="ikv"><span>轮次统计</span><b>{{ header.roundCount }} 条回复 · {{ header.toolCount }} 次工具<template v-if="header.errorCount"> · 错误 {{ header.errorCount }}</template></b></div>
            <div v-if="header.turnMs != null" class="ikv"><span>墙钟耗时</span><b>{{ (header.turnMs / 1000).toFixed(1) }}s</b></div>
          </section>
        </template>

        <!-- assistant：chips + 块构成 + 正文预览 -->
        <section v-else-if="asstP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="itags">
            <span v-if="asstP.model" class="itag itag-mono">{{ asstP.model }}</span>
            <span class="itag">第 {{ asstP.round + 1 }} 轮输出</span>
            <span v-if="asstP.continuation" class="itag itag-warn">自动续写</span>
            <span v-if="asstP.token_count != null" class="itag">{{ asstP.token_count }} tok</span>
            <span v-if="asstP.duration_ms != null" class="itag">{{ (asstP.duration_ms / 1000).toFixed(1) }}s</span>
          </div>
          <div class="ikv"><span>块构成</span><b>{{ blockComposition || "（空消息）" }}</b></div>
          <!-- 预览多行截断；无正文时镜像表格的「思考代摘要」逻辑 -->
          <div v-if="asstP.content" class="ikv ikv-top"><span>正文预览</span><b class="iprev">{{ previewOf(asstP.content) }}</b></div>
          <div v-else-if="thinkings.length" class="ikv ikv-top"><span>思考预览</span><b class="iprev iprev-think">{{ previewOf(thinkings[0].thinking) }}</b></div>
          <div v-else class="ikv"><span>正文预览</span><b>（无正文）</b></div>
        </section>

        <!-- user：正文预览 + 图片 -->
        <section v-else-if="userP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="ikv ikv-top"><span>正文预览</span><b class="iprev">{{ previewOf(userP.content) || "（无文本）" }}</b></div>
          <div v-if="userImages.length" class="ikv"><span>图片</span><b>{{ userImages.length }} 张（正文标签页查看）</b></div>
          <div class="ikv"><span>字符数</span><b>{{ userP.content?.length ?? 0 }}</b></div>
        </section>

        <!-- tool：执行速览 + 参数/结果预览 -->
        <section v-else-if="toolP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="itags">
            <span class="itag itag-mono">{{ toolP.tool_name }}</span>
            <span v-if="toolP.is_error" class="itag itag-err">失败</span>
          </div>
          <div class="ikv"><span>耗时</span><b>{{ toolP.duration_ms }} ms</b></div>
          <div class="ikv"><span>tool_call_id</span><b class="imono">{{ toolP.tool_call_id }}</b></div>
          <div v-if="compactJsonOf(toolP.arguments)" class="ikv"><span>参数预览</span><b class="imono">{{ compactJsonOf(toolP.arguments) }}</b></div>
          <div v-if="toolResultState" class="ikv"><span>结果</span><b :class="{ 'ev-err-text': toolP.is_error }">{{ toolResultState }}</b></div>
        </section>

        <!-- 其余 kind 的速览键值 -->
        <section v-else-if="sumP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="ikv"><span>类型</span><b>{{ ev!.event.kind === "summary_created" ? "新建" : "更新" }}</b></div>
          <div class="ikv"><span>覆盖至 rowid</span><b>{{ sumP.covered_until_rowid }}</b></div>
          <div class="ikv ikv-top"><span>摘要预览</span><b class="iprev">{{ previewOf(sumP.content) }}</b></div>
        </section>

        <section v-else-if="planP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="ikv"><span>条目</span><b>{{ planP.items.length }} 步（{{ planP.items.filter((i) => i.status === "done").length }} 已完成）</b></div>
          <div v-if="planP.items.some((i) => i.task_conversation_id)" class="ikv">
            <span>挂接任务</span><b>{{ planP.items.filter((i) => i.task_conversation_id).length }} 步（清单页 ↗ 可见）</b>
          </div>
        </section>

        <section v-else-if="errP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="ikv"><span>错误类别</span><b>{{ errP.kind }}</b></div>
          <div class="ikv ikv-top"><span>错误预览</span><b class="iprev iprev-err">{{ previewOf(errP.error) }}</b></div>
        </section>

        <section v-else-if="discP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="ikv ikv-top"><span>原因预览</span><b class="iprev">{{ previewOf(discP.reason) }}</b></div>
        </section>

        <section v-else-if="modalP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="ikv"><span>阶段 / 模式</span><b>{{ modalP.stage }} / {{ modalP.mode }}</b></div>
          <div class="ikv"><span>条目</span><b>{{ modalP.items?.length ?? 0 }}</b></div>
          <div v-if="modalOutcomes" class="ikv"><span>处理结果</span><b>{{ modalOutcomes }}</b></div>
        </section>

        <section v-else-if="hookP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="ikv"><span>接入点</span><b>{{ hookP.point }}</b></div>
          <div class="ikv ikv-top"><span>内容预览</span><b class="iprev">{{ previewOf(hookP.prompt) }}</b></div>
        </section>

        <section v-else-if="attachP" class="isec">
          <h4 class="isec-title">概览</h4>
          <div class="ikv"><span>附件</span><b>{{ attachP.items?.length ?? 0 }} 个<template v-if="attachP.kind">（{{ attachP.kind }}）</template></b></div>
        </section>

        <!-- 事件元数据（所有事件行通用：关联键 turn_id/message_id 此前只能翻原始 JSON） -->
        <section v-if="ev" class="isec">
          <h4 class="isec-title">事件元数据</h4>
          <div class="ikv"><span>事件类型</span><b class="imono">{{ ev.event.kind }}</b></div>
          <div class="ikv"><span>seq</span><b>{{ ev.seq }}</b></div>
          <div class="ikv"><span>actor</span><b>{{ ev.event.actor }}</b></div>
          <div class="ikv"><span>turn_id</span><b class="imono">{{ ev.event.turn_id ?? "—" }}</b></div>
          <div class="ikv"><span>message_id</span><b class="imono">{{ ev.event.message_id ?? "—" }}</b></div>
          <div class="ikv"><span>时间</span><b>{{ fullTime(ev.createdAt) }}</b></div>
        </section>
      </template>

      <!-- ============ 思考（assistant 全量思考块；多个时加序号小标） ============ -->
      <template v-else-if="activeTab === 'thinking'">
        <div class="isec">
          <template v-for="(th, i) in thinkings" :key="`th-${i}`">
            <div v-if="thinkings.length > 1" class="isub">思考 {{ i + 1 }}</div>
            <pre class="ipre-think">{{ th.thinking }}</pre>
          </template>
        </div>
      </template>

      <!-- ============ 正文（user=Markdown+图墙 / assistant=Markdown；tab 名即标题） ============ -->
      <template v-else-if="activeTab === 'text'">
        <div v-if="userP" class="isec">
          <MarkdownRenderer v-if="userP.content" :content="userP.content" />
          <div v-else class="insp-muted">（无文本，仅图片）</div>
          <div v-if="userImages.length" class="iimgs">
            <img
              v-for="(img, i) in userImages"
              :key="i"
              class="iimg"
              :src="`data:${img.mediaType};base64,${img.data}`"
              @click="openImage(i)"
            />
          </div>
        </div>
        <div v-else-if="asstP" class="isec">
          <MarkdownRenderer v-if="asstP.content" :content="asstP.content" />
          <div v-else class="insp-muted">（无文本输出——思考与工具调用见其他标签）</div>
        </div>
      </template>

      <!-- ============ 工具参数 / 结果 ============ -->
      <template v-else-if="activeTab === 'params'">
        <div class="isec">
          <pre class="ipre">{{ prettyJson(toolP?.arguments) }}</pre>
        </div>
      </template>

      <template v-else-if="activeTab === 'result'">
        <div class="isec">
          <pre class="ipre" :class="{ 'ipre-err': toolP?.is_error }">{{ toolP?.result }}</pre>
        </div>
      </template>

      <!-- ============ 摘要正文 ============ -->
      <template v-else-if="activeTab === 'content'">
        <div class="isec">
          <MarkdownRenderer :content="sumP?.content ?? ''" />
        </div>
      </template>

      <!-- ============ 计划清单（全量快照，行 = 当时整个计划） ============ -->
      <template v-else-if="activeTab === 'plan'">
        <div class="isec">
          <div v-if="!planP?.items.length" class="insp-muted">（空清单——agent 清空了计划）</div>
          <div v-for="(it, i) in planP?.items ?? []" :key="i" class="iplan-row">
            <span class="iplan-mark" :data-status="it.status" />
            <span class="iplan-text">{{ it.text }}</span>
            <span v-if="it.task_conversation_id" class="iplan-task" :title="`任务会话 ${it.task_conversation_id}`">↗ 任务</span>
          </div>
        </div>
      </template>

      <!-- ============ 明细（错误全文/原因/视觉适配/钩子/附件） ============ -->
      <template v-else-if="activeTab === 'detail'">
        <div v-if="errP" class="isec">
          <pre class="ipre ipre-err">{{ errP.error }}</pre>
        </div>
        <div v-else-if="discP" class="isec">
          <pre class="ipre">{{ discP.reason }}</pre>
        </div>
        <div v-else-if="modalP" class="isec">
          <div v-for="(it, i) in modalP.items" :key="i" class="iadapted">
            <b>[{{ it.index }}] {{ it.outcome }}</b>
            <pre v-if="it.ocr_text" class="ipre">{{ it.ocr_text }}</pre>
          </div>
        </div>
        <div v-else-if="hookP" class="isec">
          <pre class="ipre">{{ hookP.prompt }}</pre>
        </div>
        <div v-else-if="attachP" class="isec">
          <div v-for="it in attachP.items ?? []" :key="it.idx" class="iattach-row">
            <span class="imono">{{ it.name }}</span>
            <span class="iattach-meta">{{ it.kind }}<template v-if="it.token_est != null"> · ~{{ it.token_est }} tok</template></span>
          </div>
        </div>
      </template>

      <!-- ============ 原始数据（审计兜底：typed 分区漏掉的 field 都在这） ============ -->
      <template v-else-if="activeTab === 'payload'">
        <div class="isec">
          <div class="ipre-toolbar">
            <button class="ipre-copy" @click="copyPayload">{{ copied ? "已复制 ✓" : "复制 JSON" }}</button>
          </div>
          <pre class="ipre ipre-raw">{{ rawPayload(row) }}</pre>
        </div>
      </template>
    </div>

    <Teleport to="body">
      <ImagePreview v-if="showPreview" :images="userImages" :start-index="previewStart" @close="showPreview = false" />
    </Teleport>
  </aside>
</template>

<style scoped>
/* 宽度由父级 TrajectoryView 绑定（可拖拽调整） */
.insp {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-left: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
  min-height: 0;
  min-width: 300px;
}

/* 头与 tab 条为一个视觉组：分隔线只由 tab 条底部承担（避免双发丝线） */
.insp-head {
  position: sticky;
  top: 0;
  z-index: 2;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 16px 6px;
  background: var(--ip-color-bg-secondary);
  flex-shrink: 0;
}
/* kind 徽章与表格行徽章同语言：药丸 */
.insp-kind {
  display: inline-flex;
  align-items: center;
  font-size: 10px;
  font-weight: var(--ip-font-weight-semibold);
  letter-spacing: 0.5px;
  padding: 2px 9px;
  height: 19px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
  white-space: nowrap;
  flex-shrink: 0;
}
.insp-kind-head { color: var(--ip-primary-600); background: var(--ip-color-primary-soft-bg, var(--ip-primary-50)); }
.ev-user { color: var(--ip-info-text); background: var(--ip-info-bg); }
.ev-tool { color: var(--ip-warning-text); background: var(--ip-warning-bg); }
.ev-error { color: var(--ip-danger-text); background: var(--ip-danger-bg); }
.ev-plan { color: var(--ip-success-text); background: var(--ip-success-bg); }

/* 计划清单行：与 PlanCard/TaskPanel 同款状态标记（局部 scoped，不复用跨组件样式） */
.iplan-row { display: flex; align-items: flex-start; gap: 8px; padding: 6px 0; }
.iplan-mark {
  width: 8px; height: 8px; margin-top: 5px; flex-shrink: 0;
  border-radius: var(--ip-radius-full);
  border: 1.5px solid var(--ip-color-text-tertiary);
}
.iplan-mark[data-status="in_progress"] { border-color: var(--ip-warning-base, #d97706); background: var(--ip-warning-base, #d97706); }
.iplan-mark[data-status="done"] { border-color: var(--ip-success-base, #16a34a); background: var(--ip-success-base, #16a34a); }
.iplan-row .iplan-mark[data-status="done"] + .iplan-text { text-decoration: line-through; color: var(--ip-color-text-tertiary); }
.iplan-text { flex: 1; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-body); }
.iplan-task { flex-shrink: 0; font-size: 11px; color: var(--ip-primary-600); }
.insp-meta {
  font-size: 11px;
  font-family: var(--ip-font-mono, monospace);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-variant-numeric: tabular-nums;
}
.insp-close {
  margin-left: auto;
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: none;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  font-size: 12px;
  border-radius: var(--ip-radius-full);
  transition: var(--ip-transition-colors);
}
.insp-close:hover { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }

/* ---- 标签条：与 ChatPage 会话标签同语言（下划线式）。
   容器 padding 4px + tab 内距 12px → 首标签文字与头/正文 16px 对齐。
   ⚠️ 勿用 ChatPage 的 margin-bottom:-1px 压线技巧：本容器是横向滚动容器
   （overflow-x:auto 使 overflow-y 也算成 auto），-1px 超出会触发竖向滚动条 ---- */
.insp-tabs {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 0 4px;
  border-bottom: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
  flex-shrink: 0;
  overflow-x: auto;
  overflow-y: hidden;
}
.insp-tab {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 8px 12px;
  border: none;
  background: none;
  cursor: pointer;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  border-bottom: 2px solid transparent;
  white-space: nowrap;
  transition: color var(--ip-duration-fast) var(--ip-ease-out), border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.insp-tab:hover { color: var(--ip-color-text-secondary); }
.insp-tab.active { color: var(--ip-primary-600); border-bottom-color: var(--ip-primary-500); font-weight: var(--ip-font-weight-medium); }
.itab-badge {
  font-size: 9px;
  padding: 0 5px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}
.insp-tab.active .itab-badge { background: var(--ip-color-primary-soft-bg, var(--ip-primary-50)); color: var(--ip-primary-600); }

.insp-body { flex: 1; overflow-y: auto; padding: 14px 16px 16px; min-height: 0; }

/* 分节卡片：浮在 bg-secondary 面板上的 bg-primary 卡，层次即结构 */
.isec {
  margin-bottom: 12px;
  padding: 12px 14px;
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
}
.isec-title {
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0 0 10px;
  display: flex;
  align-items: center;
  gap: 8px;
}
.ikv { display: flex; gap: 10px; font-size: var(--ip-text-body-sm-size); padding: 4px 0; }
.ikv span { color: var(--ip-color-text-tertiary); flex-shrink: 0; width: 92px; }
.ikv b { font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); word-break: break-all; }
/* 多行预览行：标签顶对齐；预览是内容引文而非事实，降为常规字重 + 次级色 */
.ikv-top { align-items: flex-start; }
.iprev {
  display: -webkit-box;
  -webkit-line-clamp: 5;
  -webkit-box-orient: vertical;
  overflow: hidden;
  white-space: pre-line;
  font-weight: var(--ip-font-weight-regular, 400);
  color: var(--ip-color-text-secondary);
  line-height: 1.6;
}
.iprev-think { font-style: italic; color: var(--ip-color-text-tertiary); }
.iprev-err { color: var(--ip-danger-base); }

.isub { font-size: 10px; font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-tertiary); letter-spacing: 0.5px; margin: 12px 0 6px; }
.isub-err { color: var(--ip-danger-text); }

.ipre {
  font-size: var(--ip-text-caption-size);
  font-family: var(--ip-font-mono, monospace);
  white-space: pre-wrap;
  word-break: break-word;
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
  padding: 10px;
  border-radius: var(--ip-radius-md);
  margin: 0;
  overflow-y: auto;
  line-height: 1.6;
}
.ipre-err { color: var(--ip-danger-base); }
.ipre-raw { max-height: none; }
.ipre-copy {
  font-size: 10px;
  padding: 2px 10px;
  border-radius: var(--ip-radius-full);
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
}
.ipre-copy:hover { color: var(--ip-color-text-primary); background: var(--ip-color-bg-tertiary); }
.ipre-toolbar { display: flex; justify-content: flex-end; margin-bottom: 8px; }
.ev-err-text { color: var(--ip-danger-base); }

/* 思考全文：专属 tab + 卡片已划界，不再加左边线分组条 */
.ipre-think {
  font-size: var(--ip-text-caption-size);
  white-space: pre-wrap;
  word-break: break-word;
  padding: 4px 0;
  color: var(--ip-color-text-secondary);
  margin: 4px 0;
  line-height: 1.7;
}

.itags { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 8px; }
.itag {
  font-size: 10px;
  padding: 2px 9px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
}
.itag-mono { font-family: var(--ip-font-mono, monospace); }
.itag-warn { color: var(--ip-warning-text); background: var(--ip-warning-bg); }
.itag-err { color: var(--ip-danger-text); background: var(--ip-danger-bg); }

.imono { font-family: var(--ip-font-mono, monospace); }

.iimgs { display: flex; gap: 10px; flex-wrap: wrap; margin-top: 10px; }
.iimg { width: 96px; height: 96px; object-fit: cover; border-radius: var(--ip-radius-md); cursor: pointer; border: 1px solid var(--ip-color-border-default); }

.iadapted { margin: 8px 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.iattach-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
  padding: 5px 0;
  font-size: var(--ip-text-caption-size);
  border-bottom: 1px solid var(--ip-color-border-default);
}
.iattach-row:last-child { border-bottom: none; }
.iattach-meta { color: var(--ip-color-text-tertiary); white-space: nowrap; }
.isec-sub { margin: 4px 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.isec-sub summary { cursor: pointer; padding: 2px 0; }
.insp-muted { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-disabled); font-style: italic; }
</style>
