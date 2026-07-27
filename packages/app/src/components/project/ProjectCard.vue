<script setup lang="ts">
// ProjectCard — 项目卡片（项目管理网格单元）
//
// 职责：
//   - 在 ProjectManager 网格中展示单个项目的核心信息：
//       色点 tile + 名称 + 描述 + 系统/归档/模板 chip + 3 stats + Agent 头像栈 + 活动时间
//   - 提供 click / edit / delete 三个 emit
//
// props（按 spec §2.3）：
//   - project            Project 实体
//   - conversationCount  该项目已缓存的会话数（避免逐个 query）
//   - lastActiveAt       ISO 8601 字符串或 null
//   - category           'system' | 'active' | 'archived' | 'template'
//   - accent             'glacier' | 'aurora' | 'ember' | 'violet' | 'moss'
//
// emits：
//   - click   卡片本体点击
//   - edit    编辑按钮点击
//   - delete  删除按钮点击
//
// 设计要点：
//   - 5 个 accent 映射到 .proj-card.{accent} 类的局部 --card-accent / --card-soft
//   - pulse 点使用 paw-pulse keyframe（Wave 1 tokens.css 已定义）
//   - 删除按钮 hover 才出现（保留旧行为）
//   - 模板数暂时硬编码为 0（templates 列表与项目的关系尚未建立）

import { computed } from "vue";
import { Trash2 } from "lucide-vue-next";
import type { Agent, Project } from "../../types";
import { initialsFromName } from "../../utils/agentAvatar";
import { useAgentsStore } from "../../stores/agents";
import AgentAvatarStack from "../common/AgentAvatarStack.vue";

const props = defineProps<{
  project: Project;
  conversationCount: number;
  lastActiveAt: string | null;
  category: "system" | "active" | "archived" | "template";
  accent: "glacier" | "aurora" | "ember" | "violet" | "moss";
}>();

const emit = defineEmits<{
  click: [project: Project];
  edit: [project: Project];
  delete: [project: Project];
}>();

/** 项目显示名称：默认项目特殊处理 */
const displayName = computed<string>(() => {
  if (props.project.id === "__default__") return "默认项目";
  return props.project.name;
});

/** 项目 ID 是否为默认项目（影响 chip） */
const isSystem = computed<boolean>(() => props.category === "system");

/** chip 文本 */
const pinLabel = computed<string>(() => {
  switch (props.category) {
    case "system":
      return "系统";
    case "archived":
      return "已归档";
    case "template":
      return "模板";
    case "active":
    default:
      return "";
  }
});

/** Agent store — 用于将 ProjectMember 解析为完整 Agent */
const agentsStore = useAgentsStore();

/** 将 ProjectMember[] 解析为 Agent[]（去重、按 store 顺序） */
const resolvedAgents = computed<Agent[]>(() => {
  const seen = new Set<string>();
  const out: Agent[] = [];
  for (const m of props.project.agents) {
    if (seen.has(m.agent_id)) continue;
    seen.add(m.agent_id);
    const a = agentsStore.byId(m.agent_id);
    if (a) out.push(a);
  }
  return out;
});

/** 头像栈用的 agents 列表（限前 4 个） */
const topAgents = computed<Agent[]>(() => resolvedAgents.value.slice(0, 4));

/** 项目中 Agent 总数（用于头像栈 total prop） */
const totalAgents = computed<number>(() => props.project.agents.length);

/** 色块 tile 上的字母缩写（取项目名前 1-2 字符） */
const tileText = computed<string>(() => initialsFromName(props.project.name) || "?");

/** 色块 tile 的强色（accent 对应色，与 accent 系统的 --card-accent 对齐） */
const tileAccentColor = computed<string>(() => {
  switch (props.accent) {
    case "glacier":
      return "var(--ip-primary-500)";
    case "aurora":
      return "var(--ip-success-base)";
    case "ember":
      return "var(--ip-warning-base)";
    case "violet":
      return "#6B5BBA";
    case "moss":
      return "#5C8C4F";
  }
  return "var(--ip-primary-500)";
});

/** 色块 tile 的背景色（accent 对应弱色） */
const tileSoftColor = computed<string>(() => {
  switch (props.accent) {
    case "glacier":
      return "var(--ip-primary-100)";
    case "aurora":
      return "var(--ip-success-bg)";
    case "ember":
      return "var(--ip-warning-bg)";
    case "violet":
      return "#E5E0F2";
    case "moss":
      return "#DEEBD8";
  }
  return "var(--ip-primary-100)";
});

/** lastActiveAt 友好显示（不引入 dayjs，复用 Intl.RelativeTimeFormat） */
const relativeTime = computed<string>(() => {
  if (!props.lastActiveAt) return "暂无活动";
  const t = new Date(props.lastActiveAt).getTime();
  if (Number.isNaN(t)) return "暂无活动";
  const diff = Date.now() - t;
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return "刚刚";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min} 分钟前`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr} 小时前`;
  const day = Math.floor(hr / 24);
  if (day < 30) return `${day} 天前`;
  const month = Math.floor(day / 30);
  if (month < 12) return `${month} 个月前`;
  const year = Math.floor(day / 365);
  return `${year} 年前`;
});

/** 描述（默认项目特殊处理） */
const description = computed<string>(() => {
  if (props.project.id === "__default__") return "未分配项目的会话";
  return props.project.description || "暂无描述";
});

function onCardClick(): void {
  emit("click", props.project);
}

function onDelete(ev: MouseEvent): void {
  ev.stopPropagation();
  emit("delete", props.project);
}
</script>

<template>
  <div
    class="proj-card"
    :class="[`accent-${accent}`, `category-${category}`]"
    tabindex="0"
    role="button"
    :aria-label="`项目 ${displayName}`"
    @click="onCardClick"
    @keydown.enter="onCardClick"
  >
    <!-- 头部：色块 tile + 名称 + 系统 chip -->
    <div class="proj-card-head">
      <div
        class="proj-icon"
        :style="{
          backgroundColor: tileSoftColor,
          color: tileAccentColor,
        }"
        aria-hidden="true"
      >
        {{ tileText }}
      </div>
      <div class="proj-card-name-wrap">
        <h3 class="proj-card-name">{{ displayName }}</h3>
      </div>
      <span v-if="isSystem || category === 'archived' || category === 'template'" class="proj-pin">
        {{ pinLabel }}
      </span>
      <!-- 删除按钮（hover 才显示） -->
      <button
        v-if="!isSystem"
        class="proj-delete"
        type="button"
        title="删除项目"
        aria-label="删除项目"
        @click="onDelete"
      >
        <Trash2 :size="14" aria-hidden="true" />
      </button>
    </div>

    <!-- 描述 -->
    <p class="proj-card-desc">{{ description }}</p>

    <!-- 3 个 stat：会话数 / Agent 数 / 模板数（模板数暂时硬编码 0） -->
    <div class="proj-stats">
      <div class="stat">
        <div class="proj-stat">{{ conversationCount }}</div>
        <div class="proj-stat-label">会话</div>
      </div>
      <div class="stat">
        <div class="proj-stat">{{ totalAgents }}</div>
        <div class="proj-stat-label">Agent</div>
      </div>
      <div class="stat">
        <div class="proj-stat">0</div>
        <div class="proj-stat-label">模板</div>
      </div>
    </div>

    <!-- 底部：头像栈 + 团队 meta + 活动时间 -->
    <div class="proj-card-foot">
      <AgentAvatarStack
        :agents="topAgents"
        :total="totalAgents"
        :size="24"
      />
      <span class="team-meta">
        {{ totalAgents }} 个 Agent
      </span>
      <span v-if="lastActiveAt" class="proj-activity">
        <span class="proj-activity-dot" aria-hidden="true"></span>
        <span class="proj-activity-time">{{ relativeTime }}</span>
      </span>
      <span v-else class="proj-activity proj-activity--empty">
        <span class="proj-activity-dot proj-activity-dot--inactive" aria-hidden="true"></span>
        <span class="proj-activity-time">{{ relativeTime }}</span>
      </span>
    </div>
  </div>
</template>

<style scoped>
/* ============================================================
 * 根容器
 * ============================================================ */
.proj-card {
  --card-accent: var(--ip-primary-500);
  --card-soft: var(--ip-primary-100);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
  padding: var(--ip-spacing-4);
  cursor: pointer;
  transition:
    transform var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out),
    border-color var(--ip-duration-base) var(--ip-ease-out);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
  min-height: 200px;
  position: relative;
  text-align: left;
  font-family: inherit;
  color: inherit;
}

.proj-card:hover {
  transform: translateY(-2px);
  box-shadow: var(--ip-shadow-xl);
  border-color: var(--card-accent);
}

.proj-card:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * 5 个 accent modifier（spec §2.3.1）
 * ============================================================ */
.proj-card.accent-glacier {
  --card-accent: var(--ip-primary-500);
  --card-soft: var(--ip-primary-100);
}
.proj-card.accent-aurora {
  --card-accent: #2D8B66;
  --card-soft: #DAEFEE;
}
.proj-card.accent-ember {
  --card-accent: var(--ip-warning-base);
  --card-soft: var(--ip-warning-bg);
}
.proj-card.accent-violet {
  --card-accent: #6B5BBA;
  --card-soft: #E5E0F2;
}
.proj-card.accent-moss {
  --card-accent: #5C8C4F;
  --card-soft: #DEEBD8;
}

/* ============================================================
 * 头部：色块 tile + 名称 + chip + 删除
 * ============================================================ */
.proj-card-head {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  min-width: 0;
}

.proj-icon {
  width: 38px;
  height: 38px;
  border-radius: var(--ip-radius-md);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-family: var(--ip-font-mono);
  font-weight: 600;
  font-size: 16px;
  flex-shrink: 0;
  letter-spacing: -0.01em;
}

.proj-card-name-wrap {
  flex: 1;
  min-width: 0;
}

.proj-card-name {
  margin: 0;
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-tight, 1.2);
  color: var(--ip-color-text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.proj-pin {
  font-size: 11px;
  font-family: var(--ip-font-mono);
  font-weight: 500;
  padding: 2px 8px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-gray-100);
  color: var(--ip-color-text-tertiary);
  flex-shrink: 0;
}

.proj-delete {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    color var(--ip-duration-base) var(--ip-ease-out),
    opacity var(--ip-duration-base) var(--ip-ease-out);
  opacity: 0;
}

.proj-card:hover .proj-delete,
.proj-card:focus-within .proj-delete {
  opacity: 1;
}

.proj-delete:hover {
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
}

/* ============================================================
 * 描述
 * ============================================================ */
.proj-card-desc {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ============================================================
 * stats（3 列 grid）
 * ============================================================ */
.proj-stats {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-3) 0;
  border-top: 1px solid var(--ip-color-border-default);
  border-bottom: 1px solid var(--ip-color-border-default);
}

.stat {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 2px;
}

.proj-stat {
  font-family: var(--ip-font-mono);
  font-size: 18px;
  font-weight: 600;
  color: var(--ip-color-text-primary);
  line-height: 1;
  font-variant-numeric: tabular-nums;
}

.proj-stat-label {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  line-height: 1;
}

/* ============================================================
 * 底部：头像栈 + meta + activity
 * ============================================================ */
.proj-card-foot {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  min-width: 0;
}

.team-meta {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
}

.proj-activity {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-left: auto;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  flex-shrink: 0;
}

.proj-activity-dot {
  width: 6px;
  height: 6px;
  border-radius: var(--ip-radius-full);
  background: var(--card-accent);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--card-accent) 20%, transparent);
  animation: paw-pulse 2.4s var(--ip-ease-out) infinite;
  flex-shrink: 0;
}

.proj-activity-dot--inactive {
  background: var(--ip-gray-300);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--ip-gray-300) 20%, transparent);
  animation: none;
}

.proj-activity-time {
  font-family: var(--ip-font-mono);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
</style>