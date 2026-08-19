<script setup lang="ts">
// ProjectSwitcher.vue — 侧栏顶部的「项目空间胶囊」
// 左侧名称区（主题色圆点 + 当前空间名，scoped 点击直达项目详情页）+ 右侧
// 动作钮（⇄ 切换空间开菜单 / 快速新建 / 管理）——显示/切换/管理三动作三入口。
// 开关态（isOpen）与快速新建表单态内部自持；select / create / manage / open
// 通过 emit 上交 Sidebar 处理。
import { ref, computed, nextTick } from "vue";
import type { Project } from "../../types";
import EntityAvatar from "../common/EntityAvatar.vue";

const props = defineProps<{
  currentProjectName: string;
  /** 当前选中的项目空间（null = 散落会话）；用于高亮菜单中的当前项 */
  scopeProjectId: string | null;
  projects: Project[];
}>();

const emit = defineEmits<{
  select: [id: string | null];
  create: [name: string];
  manage: [];
  /** 项目名点击 → 打开项目详情页（散落态按钮 disabled，本 emit 不触发） */
  open: [id: string];
}>();

const isOpen = ref(false);
const isScoped = computed(() => props.scopeProjectId !== null);

/** 当前项目对象（散落态 null）——胶囊头像取数 */
const currentProject = computed(() =>
  props.scopeProjectId ? (props.projects.find((x) => x.id === props.scopeProjectId) ?? null) : null,
);

function onSelect(id: string | null) {
  isOpen.value = false;
  emit("select", id);
}

function onManage() {
  isOpen.value = false;
  emit("manage");
}

/** 名称区点击 → 项目详情页（散落态按钮 disabled，防御性兜底） */
function onOpenDetail() {
  if (!props.scopeProjectId) return;
  emit("open", props.scopeProjectId);
}

// ---- 快速新建（UX #1）：+ 钮把胶囊整行原地替换为迷你表单，纯名字创建 ——
// 完整字段（图标/工作区/主题色）留给项目列表页
const creating = ref(false);
const newName = ref("");
const nameInput = ref<HTMLInputElement | null>(null);

async function startCreate() {
  isOpen.value = false;
  creating.value = true;
  newName.value = "";
  await nextTick();
  nameInput.value?.focus();
}
function cancelCreate() {
  creating.value = false;
  newName.value = "";
}
function confirmCreate() {
  const name = newName.value.trim();
  if (!name) return;
  creating.value = false;
  newName.value = "";
  emit("create", name);
}
</script>

<template>
  <!-- 项目空间胶囊（position:relative 是切换菜单向下弹出的锚点） -->
  <div class="project-capsule">
    <!-- 快速新建：整行原地替换为迷你表单（纯名字，Enter 确认 / Esc 取消） -->
    <div v-if="creating" class="switcher-create">
      <input
        ref="nameInput"
        v-model="newName"
        class="create-input"
        type="text"
        placeholder="项目名称"
        maxlength="60"
        @keydown.enter.prevent="confirmCreate"
        @keydown.esc.prevent="cancelCreate"
      />
      <div class="create-actions">
        <button class="create-btn" :disabled="!newName.trim()" title="创建" @click="confirmCreate">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12" /></svg>
        </button>
        <button class="create-btn" title="取消" @click="cancelCreate">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
        </button>
      </div>
    </div>

    <template v-else>
      <!-- 左侧：当前空间名纯展示——scoped 点击直达项目详情页（轨迹/台账/
           设置），散落态置灰不可点。「看项目」与「切空间」是两个动作，不混一个入口 -->
      <button
        class="proj-name"
        :class="{ scoped: isScoped }"
        :disabled="!isScoped"
        :title="isScoped ? `${currentProjectName}——点击查看项目详情` : '散落会话：不属于任何项目的会话'"
        @click="onOpenDetail"
      >
        <EntityAvatar
          v-if="currentProject"
          class="scope-avatar"
          :name="currentProject.name"
          :image="currentProject.avatar"
          :accent="currentProject.theme_color"
          size="sm"
        />
        <span v-else class="item-dot muted" />
        <span class="switcher-name">{{ currentProjectName }}</span>
      </button>

      <!-- 右侧：动作钮（⇄ 切换空间 / 快速新建 / 管理）——对集合的操作外置，不藏进菜单 -->
      <div class="capsule-actions">
        <button
          class="capsule-btn"
          :class="{ 'switcher-open': isOpen }"
          title="切换项目空间"
          @click="isOpen = !isOpen"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="17 1 21 5 17 9" />
            <path d="M3 11V9a4 4 0 0 1 4-4h14" />
            <polyline points="7 23 3 19 7 15" />
            <path d="M21 13v2a4 4 0 0 1-4 4H3" />
          </svg>
        </button>
        <button class="capsule-btn" title="快速新建项目" @click="startCreate">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
          </svg>
        </button>
        <button class="capsule-btn" title="管理项目" @click="onManage">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" />
            <line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" />
          </svg>
        </button>
      </div>
    </template>

    <!-- 切换菜单（向下弹出）与遮罩必须在 .project-capsule 内部：
         菜单 absolute 的锚是 capsule（position:relative），放组件根级兄弟位置时
         会锚到 .sidebar 上（真机事故：top:calc(100%+6px) 参照整个侧边栏高度，
         菜单弹到视口外——「点了没反应」的真正根因）。开合用 class 驱动（open）
         + CSS transition，不走 <Transition>+v-if 插拔（首开不挂载，见前 commit） -->
    <div class="switcher-overlay" :class="{ open: isOpen }" @click="isOpen = false" />
    <nav class="switcher-menu" :class="{ open: isOpen }" :aria-hidden="!isOpen || undefined">
      <div class="switcher-list">
        <button class="switcher-item" :class="{ active: !isScoped }" @click="onSelect(null)">
          <span class="item-mark"><span class="item-dot muted" /></span>
          <span class="item-name">散落会话</span>
          <svg v-if="!isScoped" class="item-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="20 6 9 17 4 12" />
          </svg>
        </button>
        <template v-if="projects.length">
          <div class="switcher-sep" />
          <button
            v-for="p in projects"
            :key="p.id"
            class="switcher-item"
            :class="{ active: scopeProjectId === p.id }"
            @click="onSelect(p.id)"
          >
            <span class="item-mark">
              <EntityAvatar
                :name="p.name"
                :image="p.avatar"
                :accent="p.theme_color"
                size="sm"
              />
            </span>
            <span class="item-name">{{ p.name }}</span>
            <svg v-if="scopeProjectId === p.id" class="item-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          </button>
        </template>
      </div>
    </nav>
  </div>
</template>

<style scoped>
/* ===== 项目空间胶囊：单行「左内容 + 右按钮」，单根组件（菜单/遮罩在 capsule 内，
   absolute 锚定 capsule）。样式自持——不依赖父级 scoped CSS ===== */
.project-capsule {
  position: relative; /* 切换菜单向下弹出的定位锚 */
  display: flex;
  align-items: stretch;
  gap: 4px;
}

/* 左侧名称区：当前空间名 + 圆点（纯展示，scoped 可点去详情页） */
.proj-name {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  border: none;
  background: none;
  cursor: pointer;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  border-radius: var(--ip-radius-md);
  line-height: 1;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.proj-name:hover {
  background-color: var(--color-sidebar-item-hover);
  color: var(--ip-color-text-primary);
}
/* 选中某项目时，名称提亮（轻微强调当前空间，区别于散落会话） */
.proj-name.scoped .switcher-name { color: var(--ip-color-text-primary); }
/* 散落态：置灰不可点（无 hover 反馈，名称按钮 disabled） */
.proj-name:disabled {
  cursor: default;
  color: var(--ip-color-text-tertiary);
}
.proj-name:disabled:hover {
  background: none;
  color: var(--ip-color-text-tertiary);
}

.switcher-name {
  flex: 1;
  min-width: 0;
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-dot {
  width: 8px;
  height: 8px;
  flex-shrink: 0;
  border-radius: 50%;
  background-color: var(--ip-primary-500);
}
.item-dot.muted {
  background-color: var(--ip-color-text-tertiary);
}
/* 胶囊当前项目头像（sm=20px，EntityAvatar 三级链） */
.scope-avatar { flex-shrink: 0; }

/* 右侧动作钮 */
.capsule-actions {
  display: flex;
  gap: 2px;
  flex-shrink: 0;
}
.capsule-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  border: none;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-tertiary);
  background: none;
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.capsule-btn:hover {
  background-color: var(--color-sidebar-item-hover);
  color: var(--ip-primary-600);
}
/* ⇄ 钮展开态 = 选中底色（与会话选中项同款），表明菜单处于打开 */
.capsule-btn.switcher-open {
  background-color: var(--color-sidebar-item-active);
  color: var(--ip-primary-600);
}

/* ===== 快速新建内联表单（UX #1）：替换胶囊整行 ===== */
.switcher-create {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
  padding: 5px 8px;
  border: 1px solid var(--ip-primary-400);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-primary);
}
.create-input {
  flex: 1;
  min-width: 0;
  border: none;
  background: none;
  outline: none;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  padding: 0;
}
.create-input::placeholder { color: var(--ip-color-text-tertiary); }
.create-actions { display: flex; gap: 2px; flex-shrink: 0; }
.create-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  border-radius: var(--ip-radius-sm);
  background: none;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.create-btn:hover { background-color: var(--color-sidebar-item-hover); color: var(--ip-primary-600); }
.create-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.create-btn:disabled:hover { background: none; color: var(--ip-color-text-tertiary); }

/* ===== 切换菜单（向下弹出，锚 .project-capsule）。
   开合 = .open class 切换 visibility/opacity/transform（常驻 DOM，不插拔）：
   overlay 关态必须完全不可交互，菜单关态藏出无障碍树 ===== */
.switcher-overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
}
.switcher-overlay.open {
  opacity: 1;
  visibility: visible;
  pointer-events: auto;
}

.switcher-menu {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  right: 0;
  z-index: 51;
  padding: 0;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  max-height: min(320px, 48vh);
  overflow-y: auto;
  /* 关态：藏出视口 + 无障碍树 + 不可交互（与 .open 间 CSS 过渡） */
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  transform: translateY(-6px) scaleY(0.96);
  transform-origin: top left;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out),
    visibility var(--ip-duration-fast);
}
.switcher-menu.open {
  opacity: 1;
  visibility: visible;
  pointer-events: auto;
  transform: none;
}

.switcher-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 10px;
  border-radius: var(--ip-radius-md);
  border: none;
  background: none;
  cursor: pointer;
  font-family: inherit;
  text-align: left;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.switcher-item:hover { background-color: var(--color-sidebar-item-hover); }
.switcher-item.active { background-color: var(--color-sidebar-item-active); }

/* 前导标记列：固定 20px 宽（EntityAvatar sm / 圆点），让标记与文字左对齐 */
.item-mark {
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.item-mark .item-dot {
  width: 8px;
  height: 8px;
}

.switcher-list {
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.item-name {
  flex: 1;
  min-width: 0;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.switcher-item.active .item-name { font-weight: var(--ip-font-weight-medium); }

.item-check { flex-shrink: 0; color: var(--ip-primary-600); }

.switcher-sep {
  height: 1px;
  background-color: var(--ip-color-border-default);
  margin: 4px 2px;
}
</style>
