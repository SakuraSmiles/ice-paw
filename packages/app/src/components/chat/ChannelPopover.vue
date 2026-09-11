<!--
  ChannelPopover — 频道成员与统筹者治理浮层（C5 指定档的用户出口）

  数据源 = chat store 的 channelView（selectConversation 已拉取；浮层打开与
  每次治理动作后 refreshChannelView 权威回正——成员/统筹位不是瞬态计数，
  无收件箱那种「本地算术可漂移」问题，但打开刷新是同一纪律）。

  动作三件（全部调后端命令，前端零直接写）：
  - 任命 setCoordinator(convId, agentId)：role 写统筹者 + 频道投影更新
  - 罢免 setCoordinator(convId, null)：统筹位空缺，下次广播触发自选举
    （投影暂回落最早成员——非统筹承诺，诚实文案）
  - 让系统补选 reelect(convId)：绕过全员弃权熔断的推荐出口（指定档故障降级）
  任命/罢免可逆（可再任命），非不可恢复删除——单步执行，不走两步武装。

  Props: convId（频道会话 id）
  Emits: close（请求关闭浮层）
-->
<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { Shield, Users } from "@lucide/vue";
import { bridge } from "../../api/bridge";
import { useChatStore } from "../../stores/chat";
import EntityAvatar from "../common/EntityAvatar.vue";

const props = defineProps<{ convId: string }>();
// close 契约留给父层（ChatHeader 外点/Esc 关闭）；浮层自身不主动 emit
defineEmits<{ close: [] }>();

const chat = useChatStore();
const actingId = ref<string | null>(null);
const reelecting = ref(false);
const errorText = ref<string | null>(null);

const view = computed(() => chat.channelView);

/** 成员角色词表（project_agents.role；统筹者角色与统筹位绑定） */
const ROLE_LABELS: Record<string, string> = {
  coordinator: "统筹者",
  lead: "负责人",
  member: "成员",
};

// 打开浮层 = 权威刷新（会话切换已拉过一次；此处防治理后侧栏等旁路变化）
watch(
  () => props.convId,
  () => { if (props.convId) void chat.refreshChannelView(); },
  { immediate: true },
);

async function appoint(agentId: string) {
  actingId.value = agentId;
  errorText.value = null;
  try {
    await bridge.channels.setCoordinator(props.convId, agentId);
    await chat.refreshChannelView();
  } catch (e) {
    errorText.value = e instanceof Error ? e.message : String(e);
  } finally {
    actingId.value = null;
  }
}

async function remove(agentId: string) {
  actingId.value = agentId;
  errorText.value = null;
  try {
    await bridge.channels.setCoordinator(props.convId, null);
    await chat.refreshChannelView();
  } catch (e) {
    errorText.value = e instanceof Error ? e.message : String(e);
  } finally {
    actingId.value = null;
  }
}

async function reelect() {
  reelecting.value = true;
  errorText.value = null;
  try {
    await bridge.channels.reelect(props.convId);
    await chat.refreshChannelView();
  } catch (e) {
    errorText.value = e instanceof Error ? e.message : String(e);
  } finally {
    reelecting.value = false;
  }
}
</script>

<template>
  <div class="channel-popover" @click.stop>
    <div class="channel-head">
      <span class="channel-title">频道成员</span>
      <span class="channel-count">{{ view?.members.length ?? 0 }} 名</span>
    </div>

    <div v-if="errorText" class="channel-error">{{ errorText }}</div>

    <div v-if="!view" class="channel-hint">加载中…</div>
    <ul v-else-if="view.members.length === 0" class="channel-empty">
      项目还没有成员——先在项目设置里添加成员，再开启频道协作
    </ul>
    <ul v-else class="channel-list">
      <li v-for="m in view.members" :key="m.agent_id" class="channel-member">
        <EntityAvatar :name="m.name" size="sm" />
        <div class="channel-member-info">
          <span class="channel-member-name">{{ m.name }}</span>
          <span class="channel-member-role">
            <Shield
              v-if="view.coordinator_agent_id === m.agent_id"
              :size="12"
              aria-hidden="true"
            />
            {{ ROLE_LABELS[m.role] ?? m.role }}
          </span>
        </div>
        <button
          v-if="view.coordinator_agent_id !== m.agent_id"
          class="channel-act channel-act-appoint"
          :disabled="actingId === m.agent_id"
          title="指定档：由你手动指定统筹者——接管无 @ 消息的回合编排"
          @click="appoint(m.agent_id)"
        >设为统筹者</button>
        <button
          v-else-if="view.coordinator_appointed"
          class="channel-act"
          :disabled="actingId === m.agent_id"
          title="罢免后统筹位空缺，下次广播触发全员自选举"
          @click="remove(m.agent_id)"
        >罢免</button>
      </li>
    </ul>

    <div class="channel-footer">
      <p class="channel-footer-hint">
        统筹者接管无 @ 消息的回合编排；指定档故障不自动换帅——可让系统补选。
      </p>
      <button
        class="channel-reelect"
        :disabled="reelecting"
        title="立即发起一次全员自选举（绕过全员弃权熔断）"
        @click="reelect"
      >
        <Users :size="13" aria-hidden="true" />
        {{ reelecting ? "补选中…" : "让系统补选" }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.channel-popover {
  position: absolute;
  top: calc(100% + 6px);
  /* ⑰：锚点迁左侧子标题后左对齐下挂（右对齐会向左溢出窗口） */
  left: 0;
  width: 320px;
  max-height: 420px;
  display: flex;
  flex-direction: column;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  z-index: var(--ip-z-popover);
  padding: var(--ip-spacing-3);
  gap: var(--ip-spacing-2);
}
.channel-head { display: flex; align-items: baseline; justify-content: space-between; }
.channel-title { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.channel-count { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.channel-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); padding: var(--ip-spacing-2) 0; }
.channel-error {
  font-size: var(--ip-text-caption-size); color: var(--ip-danger-text);
  background: var(--ip-danger-bg); border-radius: var(--ip-radius-md);
  padding: 6px 10px; line-height: 1.5;
}
.channel-empty {
  margin: 0; padding: var(--ip-spacing-4) var(--ip-spacing-2);
  text-align: center; font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary); list-style: none; line-height: 1.6;
}

.channel-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; overflow-y: auto; }
.channel-member { display: flex; align-items: center; gap: var(--ip-spacing-2); padding: 6px var(--ip-spacing-2); border-radius: var(--ip-radius-md); }
.channel-member:hover { background: var(--ip-color-bg-secondary); }
.channel-member-info { flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 6px; }
.channel-member-name { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.channel-member-role { display: inline-flex; align-items: center; gap: 3px; flex-shrink: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); }
/* Shield 图标进文本流：显式 inline-block（base.css svg display:block reset 陷阱） */
.channel-member-role svg { display: inline-block; color: var(--ip-color-primary-tint-text, var(--ip-primary-600)); }

.channel-act {
  flex-shrink: 0; padding: 3px 10px;
  border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-sm);
  background: transparent; color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-caption-size); font-family: inherit; cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.channel-act:hover:not(:disabled) { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }
.channel-act:disabled { opacity: 0.5; cursor: not-allowed; }
.channel-act-appoint { color: var(--ip-primary-600); border-color: var(--ip-primary-soft-border, rgba(var(--ip-primary-500-rgb), 0.35)); }
.channel-act-appoint:hover:not(:disabled) { background: var(--ip-primary-soft-bg); color: var(--ip-primary-600); }

.channel-footer { display: flex; flex-direction: column; gap: var(--ip-spacing-2); padding-top: var(--ip-spacing-2); border-top: 1px solid var(--ip-color-border-default); }
.channel-footer-hint { margin: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); line-height: 1.5; }
.channel-reelect {
  align-self: flex-start;
  display: inline-flex; align-items: center; gap: 5px;
  padding: 4px 12px; border: none; border-radius: var(--ip-radius-sm);
  background: transparent; color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-caption-size); font-family: inherit; cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.channel-reelect:hover:not(:disabled) { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }
.channel-reelect:disabled { opacity: 0.5; cursor: not-allowed; }
.channel-reelect svg { display: inline-block; }
</style>
