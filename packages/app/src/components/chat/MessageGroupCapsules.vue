<!--
  MessageGroupCapsules — 组级收纳胶囊行（U3-3 第一刀：从 ChatMessages.vue 抽出）

  纯展示组件：三胶囊（思考聚合 / 工具折叠 / 过程收纳）+ 展开后的思考堆叠。
  不触 store、不读 group 对象——父级把「资格谓词 + 统计原语 + 展开态 + 标签」都算成
  原语传下来，本组件只负责渲染与转发 toggle 事件。收纳逻辑（阈值判定 / 截断检测 /
  组键转移）全部留在 ChatMessages 内。

  事件契约（向上转发，父级闭包 group 调用对应 toggle）：
  - toggle-thinking / toggle-tools / toggle-process：无 payload（父级已知 group.key）
  - toggle-seg：payload = 思考段展开键（与 item 内 think-block 同构）
-->
<script setup lang="ts">
import { Brain, MessageSquareText, Wrench } from "@lucide/vue";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import StatusGlyph from "./StatusGlyph.vue";
import { formatThinkingMs } from "../../utils/format";
import { thinkSegLabel, type ThinkSegment } from "../../utils/groupCollapse";

defineProps<{
  /** 思考聚合胶囊可见（父级 thinkingAggregateEligible） */
  thinkingEligible: boolean;
  /** 工具折叠胶囊可见（父级 toolCollapseEligible） */
  toolEligible: boolean;
  /** 过程收纳胶囊可见（父级 processCollapseEligible） */
  processEligible: boolean;
  /** 本组思考聚合是否已展开（expandedThinkingGroups.has(group.key)） */
  thinkingExpanded: boolean;
  /** 聚合段数（groupThinkingStats 的 segs.length） */
  thinkingSegCount: number;
  /** 聚合总耗时（null = 无持久耗时，不显示） */
  thinkingTotalMs: number | null;
  /** 聚合段列表（展开堆叠渲染源） */
  segs: ThinkSegment[];
  /** 各段展开键集合（父级 expandedThinking，键与 item 内 think-block 同构） */
  expandedSegKeys: ReadonlySet<string>;
  /** 工具折叠是否处于收起态（isToolsCollapsed） */
  toolsCollapsed: boolean;
  /** 工具调用总数 */
  toolTotal: number;
  /** 工具失败数（>0 才显示「N 失败」） */
  toolErrors: number;
  /** 过程收纳是否处于收起态（isProcessCollapsed） */
  processCollapsed: boolean;
  /** 是否为截断组（groupTruncatedAfter——「回合被截断」warning 标注） */
  processTruncated: boolean;
  /** 过程收纳行标签（父级 processRowLabel 算好的最终文案） */
  processLabel: string;
}>();

const emit = defineEmits<{
  "toggle-thinking": [];
  "toggle-tools": [];
  "toggle-process": [];
  "toggle-seg": [key: string];
}>();
</script>

<template>
  <div v-if="thinkingEligible || toolEligible || processEligible" class="group-summary-pills">
    <button v-if="thinkingEligible" type="button" class="think-toggle summary-pill think-group-summary" :class="{ 'is-open': thinkingExpanded }" :aria-expanded="thinkingExpanded" @click="emit('toggle-thinking')">
      <Brain :size="14" class="pill-glyph" aria-hidden="true" />
      <span class="think-label">思考 · {{ thinkingSegCount }} 段</span>
      <span v-if="thinkingTotalMs != null" class="think-label think-group-total">{{ formatThinkingMs(thinkingTotalMs) }}</span>
      <span class="think-chevron">{{ thinkingExpanded ? '▾' : '▸' }}</span>
    </button>
    <button v-if="toolEligible" type="button" class="tool-toggle summary-pill tool-group-summary" :class="{ 'is-open': !toolsCollapsed }" :aria-expanded="!toolsCollapsed" @click="emit('toggle-tools')">
      <Wrench :size="14" class="pill-glyph" aria-hidden="true" />
      <template v-if="toolsCollapsed">
        <span class="tool-name">{{ toolTotal }} 次工具调用</span>
        <span v-if="toolErrors > 0" class="tool-fail-count">{{ toolErrors }} 失败</span>
      </template>
      <span v-else class="tool-name">收起 · {{ toolTotal }} 次工具调用</span>
      <span class="tool-chevron">{{ toolsCollapsed ? '▸' : '▾' }}</span>
    </button>
    <button v-if="processEligible" type="button" class="tool-toggle summary-pill process-group-summary" :class="{ 'is-open': !processCollapsed }" :aria-expanded="!processCollapsed" @click="emit('toggle-process')">
      <MessageSquareText :size="14" class="pill-glyph" aria-hidden="true" />
      <span class="tool-name" :class="{ 'process-truncated': processCollapsed && processTruncated }">{{ processLabel }}</span>
      <span class="tool-chevron">{{ processCollapsed ? '▸' : '▾' }}</span>
    </button>
  </div>
  <Transition name="think-fade">
    <div v-if="thinkingEligible && thinkingExpanded" class="think-group-stack">
      <div v-for="seg in segs" :key="seg.key" class="think-block">
        <div class="think-toggle" @click="emit('toggle-seg', seg.key)">
          <StatusGlyph status="done" class="think-glyph" />
          <span class="think-label">{{ thinkSegLabel(seg) }}</span>
          <span class="think-chevron">{{ expandedSegKeys.has(seg.key) ? '▾' : '▸' }}</span>
        </div>
        <Transition name="think-fade">
          <div v-if="expandedSegKeys.has(seg.key)" class="think-body">
            <MarkdownRenderer :content="seg.text" />
          </div>
        </Transition>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
/* ===== 思考/工具基类（与 ChatMessages 内 item 渲染同族——scoped 隔离不继承，
   子组件需复制一份；⚠️ 两处样式须保持同步，改动请一并修改父组件） ===== */
.think-block { margin:0; }
.think-toggle { display:flex; align-items:center; gap:6px; padding:2px 6px; cursor:pointer; user-select:none; border-radius:var(--ip-radius-sm); transition:all var(--ip-duration-fast) var(--ip-ease-out); width:100%; }
.think-toggle:hover { background:var(--ip-color-bg-tertiary); }
.think-chevron { margin-left:auto; font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); line-height:1; width:10px; flex-shrink:0; transition:transform var(--ip-duration-fast) var(--ip-ease-out); }
/* 勿加 text-transform:uppercase——label 是中文不受影响，但会把后缀的耗时单位
   （m/s）打成大写（2026-09-09 生产反馈：思考 · 1M 30S） */
.think-label { font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-color-text-tertiary); letter-spacing:0.3px; }
.think-body { margin:4px 0 4px 22px; padding:6px 0 6px 14px; border-left:2px solid var(--ip-primary-200); font-size:var(--ip-text-body-sm-size); color:var(--ip-color-text-secondary); line-height:1.6; white-space:pre-wrap; word-break:break-word; }
.think-body .markdown-body { font-size:inherit; color:inherit; line-height:inherit; }

.think-fade-enter-active { animation:think-in 0.2s ease-out; }
.think-fade-leave-active { animation:think-in 0.12s ease-in reverse; }
@keyframes think-in {
  from { opacity:0; transform:translateY(-3px); }
  to   { opacity:1; transform:translateY(0); }
}

.tool-toggle { display:flex; align-items:center; gap:6px; padding:2px 6px; cursor:pointer; user-select:none; border-radius:var(--ip-radius-sm); transition:background var(--ip-duration-fast) var(--ip-ease-out); width:100%; }
.tool-toggle:hover { background:var(--ip-color-bg-tertiary); }
.tool-chevron { font-size: var(--ip-text-micro-size); color:var(--ip-color-text-disabled); line-height:1; width:10px; flex-shrink:0; }
.tool-name { font-size:var(--ip-text-caption-size); font-weight:var(--ip-font-weight-medium); color:var(--ip-color-text-tertiary); white-space:nowrap; }
.tool-fail-count { font-size:var(--ip-text-caption-size); color:var(--ip-warning-text); white-space:nowrap; }

/* ===== 组级收纳胶囊行（从 ChatMessages 下沉，U3-3）——chrome 对齐房内 chip
   语言 ChatHeader .header-kind-badge：收起=实底软色胶囊（这里有内容，点开看）/
   展开=幽灵描边胶囊（is-open，内容已在场、控件退位）；hover 各自加深；边框两态
   恒 1px 防开合尺寸跳动。色值照抄 header-kind-badge 的 var+rgba 兜底写法
   （--ip-primary-soft-border 全局无定义，rgba 兜底即频道 tag 徽章的实际渲染形态）。 */
.group-summary-pills { display:flex; flex-wrap:wrap; align-items:center; gap:var(--ip-spacing-2); margin-bottom:var(--ip-spacing-3); }
.group-summary-pills .summary-pill {
  width:auto; flex-shrink:0;
  padding:2px 10px; gap:4px;
  border-radius:var(--ip-radius-full, 999px);
  border:1px solid rgba(var(--ip-primary-500-rgb), 0.25);
  background:var(--ip-color-primary-soft-bg, rgba(var(--ip-primary-500-rgb), 0.08));
  color:var(--ip-primary-600);
  font-family:inherit; font-size:inherit; line-height:inherit;
  transition:background var(--ip-duration-fast) var(--ip-ease-out),
             border-color var(--ip-duration-fast) var(--ip-ease-out),
             color var(--ip-duration-fast) var(--ip-ease-out);
}
.group-summary-pills .summary-pill:hover { background:rgba(var(--ip-primary-500-rgb), 0.14); }
.group-summary-pills .summary-pill.is-open { background:transparent; border-color:var(--ip-color-border-default); color:var(--ip-color-text-tertiary); }
.group-summary-pills .summary-pill.is-open:hover { background:var(--ip-color-bg-tertiary); }
/* 胶囊内子元素随态取色：图标/标签/chevron 一律 inherit 胶囊色；标签升 body-sm-13
   + semibold（胶囊是折叠内容的唯一入口，读作区块控件而非行内元数据） */
.group-summary-pills .summary-pill .pill-glyph { color:inherit; }
.group-summary-pills .summary-pill .tool-name,
.group-summary-pills .summary-pill .think-label {
  color:inherit;
  font-size:var(--ip-text-body-sm-size);
  font-weight:var(--ip-font-weight-semibold);
}
.group-summary-pills .summary-pill .tool-fail-count { font-size:var(--ip-text-body-sm-size); }
.group-summary-pills .summary-pill .tool-chevron,
.group-summary-pills .summary-pill .think-chevron { color:inherit; opacity:0.65; font-size:var(--ip-text-caption-size); }
.group-summary-pills .summary-pill .think-group-total { color:inherit; opacity:0.75; }
.pill-glyph { flex-shrink:0; }
.think-group-total { font-weight:var(--ip-font-weight-regular); }
.think-group-stack { margin:2px 0 6px 22px; display:grid; gap:2px; }
/* 截断组标注：回合被截断是警示事实——warning 语义色；⚠️ 必须排在上方 .tool-name
   inherit 覆写之后——两规则同为 (0,3,0)，同元素双类命中时后者胜 */
.group-summary-pills .summary-pill .process-truncated { color:var(--ip-warning-text); }
.process-truncated { color:var(--ip-warning-text); }
</style>
