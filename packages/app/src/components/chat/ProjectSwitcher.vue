<script setup lang="ts">
// ProjectSwitcher.vue — 侧栏顶部的「项目空间胶囊」
// 左侧 chip（主题色圆点 + 当前空间名，点击开向下切换菜单）+ 右侧动作钮
// （快速新建 / 管理）。开关态（isOpen）与快速新建表单态内部自持；
// select / create / manage 通过 emit 上交 Sidebar 处理。
import { ref, computed, nextTick } from "vue";
import type { Project } from "../../types";

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
}>();

const isOpen = ref(false);
const isScoped = computed(() => props.scopeProjectId !== null);

/** chip 圆点 = 当前项目主题色（散落 / 无主题色时回落 CSS 默认灰） */
const scopeDotStyle = computed(() => {
  if (!props.scopeProjectId) return {};
  const p = props.projects.find((x) => x.id === props.scopeProjectId);
  return p?.theme_color ? { backgroundColor: p.theme_color } : {};
});

function onSelect(id: string | null) {
  isOpen.value = false;
  emit("select", id);
}

function onManage() {
  isOpen.value = false;
  emit("manage");
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
      <!-- 左侧：当前空间 chip（点开切换菜单） -->
      <button
        class="proj-chip"
        :class="{ 'switcher-open': isOpen, scoped: isScoped }"
        :title="isScoped ? `当前项目空间：${currentProjectName}` : '未选择项目（散落会话）'"
        @click="isOpen = !isOpen"
      >
        <span class="item-dot" :class="{ muted: !isScoped }" :style="scopeDotStyle" />
        <span class="switcher-name">{{ currentProjectName }}</span>
        <svg class="switcher-caret" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      <!-- 右侧：动作钮（快速新建 / 管理）——对集合的操作外置，不藏进菜单 -->
      <div class="capsule-actions">
        <button class="capsule-btn" title="快速新建项目" @click="startCreate">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
          </svg>
        </button>
        <button class="capsule-btn" title="管理项目" @click="onManage">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="4" y1="21" x2="4" y2="14" /><line x1="4" y1="10" x2="4" y2="3" /><line x1="12" y1="21" x2="12" y2="12" /><line x1="12" y1="8" x2="12" y2="3" /><line x1="20" y1="21" x2="20" y2="16" /><line x1="20" y1="12" x2="20" y2="3" /><line x1="1" y1="14" x2="7" y2="14" /><line x1="9" y1="8" x2="15" y2="8" /><line x1="17" y1="16" x2="23" y2="16" />
          </svg>
        </button>
      </div>
    </template>
  </div>

  <!-- 切换菜单（向下弹出，锚 .project-capsule） -->
  <div v-if="isOpen" class="switcher-overlay" @click="isOpen = false" />
  <Transition name="switcher-pop">
    <nav v-if="isOpen" class="switcher-menu">
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
            <span class="item-mark"><span class="item-dot" :style="p.theme_color ? { backgroundColor: p.theme_color } : {}" /></span>
            <span class="item-name">{{ p.name }}</span>
            <svg v-if="scopeProjectId === p.id" class="item-check" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          </button>
        </template>
      </div>
    </nav>
  </Transition>
</template>

<style scoped>
/* ===== 项目空间胶囊：单行「左内容 + 右按钮」，多根组件（capsule + overlay + Transition/nav）
   自持全部样式——父级 scoped CSS 命不中多根子组件的元素 ===== */
.project-capsule {
  position: relative; /* 切换菜单向下弹出的定位锚 */
  display: flex;
  align-items: stretch;
  gap: 4px;
}

/* 左侧 chip：当前空间名 + 圆点 + caret */
.proj-chip {
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
.proj-chip:hover {
  background-color: var(--color-sidebar-item-hover);
  color: var(--ip-color-text-primary);
}
/* 展开态 = 选中底色（与会话选中项同款），表明控件处于打开 */
.proj-chip.switcher-open {
  background-color: var(--color-sidebar-item-active);
}
/* 选中某项目时，名称提亮（轻微强调当前空间，区别于散落会话） */
.proj-chip.scoped .switcher-name { color: var(--ip-color-text-primary); }

.switcher-name {
  flex: 1;
  min-width: 0;
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.switcher-caret {
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.proj-chip.switcher-open .switcher-caret {
  transform: rotate(180deg);
  color: var(--ip-primary-600);
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

/* ===== 切换菜单（向下弹出，锚 .project-capsule） ===== */
.switcher-overlay {
  position: fixed;
  inset: 0;
  z-index: 50;
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

/* 前导标记列：固定 16px 宽，让圆点 / 文字左对齐 */
.item-mark {
  width: 16px;
  height: 16px;
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

/* 弹出动画（从上方缩放进入） */
.switcher-pop-enter-active,
.switcher-pop-leave-active {
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out);
  transform-origin: top left;
}
.switcher-pop-enter-from,
.switcher-pop-leave-to {
  opacity: 0;
  transform: translateY(-6px) scaleY(0.96);
}
</style>
