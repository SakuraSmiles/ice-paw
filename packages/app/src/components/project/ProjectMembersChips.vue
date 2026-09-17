<script setup lang="ts">
// ProjectMembersChips.vue — 项目成员 chips（已选 × 移除 / 候选 + 添加）共享组件。
// 双入口复用：ProjectList 展开区 + 项目详情页设置 tab。agent 名单解析内部
// 走 agent store。值走 v-model（草稿语义，与 ProjectBasicForm 同构）——点击
// chip 只改草稿并 emit update:memberIds，持久化上交父级显式保存（编辑契约
// U0-7：摘除「点击即持久化」旧路径，本组件无状态无中间态，两入口行为一致）。
import { computed } from "vue";
import { useAgentStore } from "../../stores/agent";

const props = defineProps<{ memberIds: string[] }>();
const emit = defineEmits<{ "update:memberIds": [ids: string[]] }>();

const agent = useAgentStore();
const memberSet = computed(() => new Set(props.memberIds));
/** 未入项目的 agent = 候选（+ 前缀） */
const candidates = computed(() => agent.list.filter((a) => !memberSet.value.has(a.id)));

function add(id: string) {
  emit("update:memberIds", [...props.memberIds, id]);
}
function remove(id: string) {
  emit("update:memberIds", props.memberIds.filter((m) => m !== id));
}
</script>

<template>
  <div class="field">
    <label class="field-label">成员</label>
    <div v-if="memberIds.length === 0 && candidates.length === 0" class="members-empty">暂无可用智能体</div>
    <div v-else class="member-chips">
      <button
        v-for="m in memberIds"
        :key="'m-' + m"
        type="button"
        class="member-chip selected"
        :title="`移除 ${agent.getById(m)?.name ?? ''}`"
        @click="remove(m)"
      >× {{ agent.getById(m)?.name ?? '未知' }}</button>
      <button
        v-for="a in candidates"
        :key="'a-' + a.id"
        type="button"
        class="member-chip"
        :title="`添加 ${a.name}`"
        @click="add(a.id)"
      >+ {{ a.name }}</button>
    </div>
  </div>
</template>

<style scoped>
/* 样式自持（从 ProjectList 编辑区原样搬入），不依赖父级 scoped CSS */
.field { display: flex; flex-direction: column; gap: var(--ip-spacing-1_5); }
.field-label {
  font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  display: flex; align-items: center; gap: var(--ip-spacing-1_5);
}

.members-empty { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.member-chips { display: flex; flex-wrap: wrap; gap: var(--ip-spacing-1_5); }
.member-chip {
  height: 28px; padding: 0 var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-full); cursor: pointer;
  font-family: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.member-chip:hover { border-color: var(--ip-primary-300); }
.member-chip.selected {
  color: var(--ip-color-primary-tint-text); background-color: var(--ip-color-primary-tint-bg);
  border-color: var(--ip-primary-400);
}
</style>
