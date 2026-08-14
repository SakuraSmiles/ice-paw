<!--
  TrajectoryToolbar — 轨迹视图顶栏（40px，仿 dsh TrajectoryToolbar）

  控件：搜索框（实时，未命中行降透明度；Esc 清空；/ 全局聚焦）·
  展开/收起合一按钮（anyCollapsed 驱动的智能切换：有折叠 → 展开全部，否则收起全部）·
  耗时开关（时间轴投影：序号等宽 ↔ 真实耗时+空闲压缩）·
  辅助事件药丸开关（默认隐藏低频事件）· 导出 JSONL。
  全部状态在父级 TrajectoryView，本组件纯受控；focusSearch 供键盘导航 / 键调用。
  图标全 SVG（跨平台渲染一致，无 emoji 字体差异）。
-->
<script setup lang="ts">
import { ref } from "vue";

defineProps<{
  query: string;
  showAux: boolean;
  /** 时间轴投影：false = 序号等宽（默认）；true = 真实耗时 + 空闲压缩 */
  durationMode: boolean;
  /** 有任一轮处于折叠态（驱动展开/收起按钮的形态与文案） */
  anyCollapsed: boolean;
  /** 会话级汇总（已载窗口内）：轮数 / 事件数 / 工具调用数 */
  stats: { turns: number; events: number; tools: number };
  exporting: boolean;
}>();

const emit = defineEmits<{
  "update:query": [v: string];
  "update:showAux": [v: boolean];
  "update:durationMode": [v: boolean];
  "toggle-turns": [];
  /** Enter/Shift+Enter 在命中行间循环跳转（dir = 前进/后退） */
  "search-jump": [dir: 1 | -1];
  export: [];
}>();

const searchRef = ref<HTMLInputElement | null>(null);
function focusSearch() {
  searchRef.value?.focus();
  searchRef.value?.select();
}
defineExpose({ focusSearch });
</script>

<template>
  <div class="tbar">
    <div class="tbar-search" title="按 / 快速聚焦；Enter/Shift+Enter 在命中行间跳转">
      <svg class="tbar-search-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7" /><line x1="21" y1="21" x2="16.5" y2="16.5" /></svg>
      <input
        ref="searchRef"
        type="text"
        placeholder="搜索内容、参数、结果…"
        :value="query"
        @input="emit('update:query', ($event.target as HTMLInputElement).value)"
        @keydown.esc.stop.prevent="emit('update:query', '')"
        @keydown.enter.stop.prevent="emit('search-jump', $event.shiftKey ? -1 : 1)"
      />
      <button v-if="query" class="tbar-search-clear" title="清空（Esc）" @click="emit('update:query', '')">✕</button>
    </div>

    <button
      class="tbar-fold"
      :class="{ folded: anyCollapsed }"
      :title="anyCollapsed ? '展开所有轮次' : '折叠所有轮次'"
      @click="emit('toggle-turns')"
    >
      <svg v-if="anyCollapsed" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9" /></svg>
      <svg v-else width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="18 15 12 9 6 15" /></svg>
      {{ anyCollapsed ? "展开全部" : "收起全部" }}
    </button>

    <label class="tbar-toggle" title="时间轴按真实耗时投影并压缩空闲（默认：事件序号等宽）">
      <input
        type="checkbox"
        :checked="durationMode"
        @change="emit('update:durationMode', ($event.target as HTMLInputElement).checked)"
      />
      <span class="tbar-pill">耗时</span>
    </label>

    <label class="tbar-toggle" title="附件落库 / 视觉适配 / 钩子注入等低频事件">
      <input
        type="checkbox"
        :checked="showAux"
        @change="emit('update:showAux', ($event.target as HTMLInputElement).checked)"
      />
      <span class="tbar-pill">辅助事件</span>
    </label>

    <div class="tbar-spacer" />

    <span class="tbar-stats" title="已载窗口内：轮次 · 事件 · 工具调用">
      {{ stats.turns }} 轮 · {{ stats.events }} 事件 · {{ stats.tools }} 工具
    </span>

    <button class="tbar-export" :disabled="exporting" @click="emit('export')">
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>{{ exporting ? "导出中…" : "导出 JSONL" }}
    </button>
  </div>
</template>

<style scoped>
.tbar {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 40px;
  padding: 0 14px;
  border-bottom: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
  background: var(--ip-color-bg-secondary);
}

.tbar-search {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 260px;
  height: 26px;
  padding: 0 9px;
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out), background var(--ip-duration-fast) var(--ip-ease-out);
}
.tbar-search:focus-within {
  border-color: var(--ip-color-border-focus);
  background: var(--ip-color-bg-primary);
}
.tbar-search-icon { color: var(--ip-color-text-tertiary); flex-shrink: 0; }
.tbar-search:focus-within .tbar-search-icon { color: var(--ip-color-text-secondary); }
.tbar-search input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: none;
  outline: none;
  background: transparent;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-primary);
}
.tbar-search input::placeholder { color: var(--ip-color-text-placeholder); }
.tbar-search-clear {
  border: none;
  background: none;
  padding: 0 2px;
  font-size: 10px;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  border-radius: var(--ip-radius-full);
  flex-shrink: 0;
}
.tbar-search-clear:hover { color: var(--ip-color-text-primary); }

/* 展开/收起合一：药丸按钮（与右侧两个开关同语言），folded 态亮主题色反馈当前有折叠 */
.tbar-fold {
  height: 26px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0 12px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  cursor: pointer;
  white-space: nowrap;
  transition: var(--ip-transition-colors);
}
.tbar-fold:hover { color: var(--ip-color-text-primary); background: var(--ip-color-bg-elevated); }
.tbar-fold.folded {
  color: var(--ip-primary-600);
  background: var(--ip-color-primary-soft-bg, var(--ip-primary-50));
  border-color: var(--ip-primary-200);
}

.tbar-toggle { display: flex; align-items: center; cursor: pointer; user-select: none; }
.tbar-toggle input { position: absolute; opacity: 0; pointer-events: none; }
.tbar-pill {
  height: 26px;
  display: inline-flex;
  align-items: center;
  padding: 0 12px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  white-space: nowrap;
  transition: var(--ip-transition-colors);
}
.tbar-toggle:hover .tbar-pill { color: var(--ip-color-text-primary); }
.tbar-toggle input:checked + .tbar-pill {
  color: var(--ip-primary-600);
  background: var(--ip-color-primary-soft-bg, var(--ip-primary-50));
  border-color: var(--ip-primary-200);
}

.tbar-spacer { flex: 1; }

/* 会话级汇总：只读文本簇（非控件），mono 数字右对齐呼应指标列 */
.tbar-stats {
  font-family: var(--ip-font-mono, monospace);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
  padding-right: 2px;
}

.tbar-export {
  height: 26px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 0 13px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-primary-600);
  background: var(--ip-color-primary-soft-bg, var(--ip-primary-50));
  border: 1px solid transparent;
  border-radius: var(--ip-radius-full);
  cursor: pointer;
  white-space: nowrap;
  transition: var(--ip-transition-colors);
}
.tbar-export:hover:not(:disabled) { background: var(--ip-color-primary-tint-bg, var(--ip-primary-100)); }
.tbar-export:disabled { opacity: 0.4; cursor: not-allowed; }
</style>
