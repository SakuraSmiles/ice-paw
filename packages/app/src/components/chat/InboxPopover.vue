<!--
  InboxPopover — MA-3 收件箱浮层（pending 来件的用户出口）

  条目按语境分两视图（2026-09-10 测试反馈）：
  - **队列视图**（accept 政策 / is_reply 回投件）：来件本会自动消费，撞上
    生成中只是排队常态——条目标「排队中 · 会话空闲后自动处理」，主操作降级
    为次要「立即处理」（不摘除——自动消费配额尽时手动放行是唯一通道，详见
    title 说明）；
  - **审批视图**（hold）：真审批手势——「批准并消费」主色钮。
  拒绝 = 不可恢复处置（settled refused 永久出队）→ 两步确认（按钮武装态）。

  数据：打开与每次处置后都走 list_inbox 权威刷新（本地 badge 计数只是气味，
  会随处置回正——refreshInboxCount）。批准时会话忙 → 后端 Err，来件留队，
  横幅显示错误文案（语境分叉的三段式来自后端）。

  Props: convId（目标会话 id）
  Emits: close（请求关闭浮层）
-->
<script setup lang="ts">
import { ref, watch } from "vue";
import { ArrowLeftRight } from "@lucide/vue";
import { bridge } from "../../api/bridge";
import { refreshInboxCount } from "../../composables/useInbox";
import { timeAgo } from "../../utils/time";
import type { InboxItem } from "../../types";
import EntityAvatar from "../common/EntityAvatar.vue";

const props = defineProps<{ convId: string }>();
// close 契约留给父层（ChatHeader 外点/Esc 关闭）；浮层自身不主动 emit
defineEmits<{ close: [] }>();

const loading = ref(false);
const items = ref<InboxItem[]>([]);
const policy = ref("accept");
const errorText = ref<string | null>(null);
/** 处置中条目（禁重复点击；批准是即时动作，拒绝两步确认见 armedRefuseId） */
const actingId = ref<string | null>(null);
/** 武装态的拒绝按钮（两步确认：一次点击武装成 danger 确认键，二次执行，
 *  再点其它区域解除——编辑交互契约的按钮武装态载体） */
const armedRefuseId = ref<string | null>(null);

/** 收件政策三态（conversations.inbox_policy 词表；文案即语义，无 jargon）。
 *  2026-09-10 默认改 accept（「在 A 发起、切到 B 批准」反人类）；「（默认）」
 *  标注跟随词表，勿随会话实际值漂移。
 *  ⚠️ hint 需恒单行（micro 11px × 340px 浮层内容宽 ≈ 28 字上限）——segmented
 *  切档时 hint 逐行换会跳高度，超长文案收字数而非允许折行 */
const POLICY_OPTIONS: { value: string; label: string; hint: string }[] = [
  { value: "accept", label: "自动接收（默认）", hint: "来件排队，会话空闲时自动消费" },
  { value: "hold", label: "需批准", hint: "来件扣住，你批准后才消费；对方的回信除外" },
  { value: "refuse", label: "拒收", hint: "投递方工具立即报错，不再接收" },
];

async function load() {
  loading.value = true;
  errorText.value = null;
  armedRefuseId.value = null;
  try {
    const view = await bridge.inbox.list(props.convId);
    items.value = view.items;
    policy.value = view.policy;
  } catch (e) {
    errorText.value = e instanceof Error ? e.message : String(e);
    items.value = [];
  } finally {
    loading.value = false;
  }
}

watch(() => props.convId, () => { if (props.convId) void load(); }, { immediate: true });

async function approve(item: InboxItem) {
  actingId.value = item.message_id;
  errorText.value = null;
  try {
    await bridge.inbox.respond(props.convId, item.message_id, true);
    // 消费回合已发起（settled + turn 启动）；本会话聊天区会自行流出事件
    await load();
    void refreshInboxCount(props.convId);
  } catch (e) {
    // 会话忙：来件留队零丢失，显示后端三段式文案
    errorText.value = e instanceof Error ? e.message : String(e);
  } finally {
    actingId.value = null;
  }
}

async function refuse(item: InboxItem) {
  if (armedRefuseId.value !== item.message_id) {
    armedRefuseId.value = item.message_id; // 第一次点击：武装
    return;
  }
  armedRefuseId.value = null;
  actingId.value = item.message_id;
  errorText.value = null;
  try {
    await bridge.inbox.respond(props.convId, item.message_id, false);
    await load();
    void refreshInboxCount(props.convId);
  } catch (e) {
    errorText.value = e instanceof Error ? e.message : String(e);
  } finally {
    actingId.value = null;
  }
}

async function switchPolicy(next: string) {
  if (next === policy.value) return;
  const prev = policy.value;
  policy.value = next; // 乐观切（segmented 即时反馈）
  try {
    await bridge.inbox.setPolicy(props.convId, next);
  } catch (e) {
    policy.value = prev; // 失败回滚
    errorText.value = e instanceof Error ? e.message : String(e);
  }
}

function deliveredAgo(item: InboxItem): string {
  return timeAgo(new Date(item.delivered_at_unix * 1000).toISOString());
}

/** 该来件是否走自动消费语境（accept 政策，或 is_reply 回投件——hold 下也免扣
 *  自动消费）：队列视图呈现。政策 segmented 乐观切，条目呈现随 policy live 更新。 */
function isAutoItem(item: InboxItem): boolean {
  return policy.value === "accept" || item.is_reply === true;
}
</script>

<template>
  <div class="inbox-popover" @click.stop>
    <div class="inbox-head">
      <span class="inbox-title">收件箱</span>
      <span v-if="items.length" class="inbox-count">
        {{ policy === "accept" ? `${items.length} 条排队中` : `${items.length} 条待处理` }}
      </span>
    </div>

    <div v-if="errorText" class="inbox-error">{{ errorText }}</div>

    <div v-if="loading" class="inbox-hint">加载中…</div>
    <template v-else-if="items.length === 0">
      <div class="inbox-empty">
        <p>暂无待处理来件</p>
        <p class="inbox-empty-sub">其他会话 agent 投递来的消息会出现在这里</p>
      </div>
    </template>
    <ul v-else class="inbox-list">
      <li v-for="item in items" :key="item.message_id" class="inbox-item">
        <div class="inbox-item-head">
          <EntityAvatar :name="item.source_agent_name" size="xs" />
          <span class="inbox-item-source" :title="`源会话：${item.source_conversation_title}`">
            {{ item.source_agent_name }} · 来自「{{ item.source_conversation_title }}」
          </span>
          <span class="inbox-item-time">{{ deliveredAgo(item) }}</span>
        </div>
        <p class="inbox-item-content">{{ item.content }}</p>
        <p v-if="isAutoItem(item)" class="inbox-item-status">排队中 · 会话空闲后自动处理</p>
        <div class="inbox-item-actions">
          <!-- 队列视图：主操作降级为次要「立即处理」（放行通道保留——自动消费
               配额尽时留队的来件靠它放行，by=user-approval 不占配额） -->
          <button
            v-if="isAutoItem(item)"
            class="inbox-act"
            :disabled="actingId === item.message_id"
            title="通常无需手动处理；仅在自动消费停摆（如配额用尽）时手动放行"
            @click="approve(item)"
          >立即处理</button>
          <button
            v-else
            class="inbox-act inbox-act-approve"
            :disabled="actingId === item.message_id"
            @click="approve(item)"
          >批准并消费</button>
          <button
            class="inbox-act"
            :class="{ 'inbox-act-armed': armedRefuseId === item.message_id }"
            :disabled="actingId === item.message_id"
            @click="refuse(item)"
            @blur="armedRefuseId === item.message_id && (armedRefuseId = null)"
          >{{ armedRefuseId === item.message_id ? "确认拒绝？" : "拒绝" }}</button>
          <span v-if="item.is_reply" class="inbox-reply-flag" title="这是你此前 expect_reply 投递的回投——对方对你上一条消息的回复">
            <ArrowLeftRight :size="12" aria-hidden="true" />
            回复
          </span>
          <span v-else-if="item.expect_reply" class="inbox-reply-flag" title="批准后消费回合完成时，回复会自动投回源会话">
            <ArrowLeftRight :size="12" aria-hidden="true" />
            期待回复
          </span>
        </div>
      </li>
    </ul>

    <div class="inbox-policy">
      <span class="inbox-policy-label">收件政策</span>
      <div class="inbox-policy-seg" role="radiogroup" aria-label="收件政策">
        <button
          v-for="opt in POLICY_OPTIONS"
          :key="opt.value"
          role="radio"
          :aria-checked="policy === opt.value"
          :class="['inbox-policy-btn', { active: policy === opt.value }]"
          :title="opt.hint"
          @click="switchPolicy(opt.value)"
        >{{ opt.label }}</button>
      </div>
      <p class="inbox-policy-hint">{{ POLICY_OPTIONS.find((o) => o.value === policy)?.hint }}</p>
    </div>
  </div>
</template>

<style scoped>
.inbox-popover {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  width: 340px;
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
.inbox-head { display: flex; align-items: baseline; justify-content: space-between; }
.inbox-title { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.inbox-count { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.inbox-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); padding: var(--ip-spacing-2) 0; }
.inbox-error {
  font-size: var(--ip-text-caption-size); color: var(--ip-danger-text);
  background: var(--ip-danger-bg); border-radius: var(--ip-radius-md);
  padding: 6px 10px; line-height: 1.5;
}
.inbox-empty { padding: var(--ip-spacing-4) var(--ip-spacing-2); text-align: center; }
.inbox-empty p { margin: 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); }
.inbox-empty-sub { margin-top: 4px; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.inbox-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--ip-spacing-2); overflow-y: auto; }
.inbox-item { display: flex; flex-direction: column; gap: 6px; padding: var(--ip-spacing-3); background: var(--ip-color-bg-secondary); border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-md); }
.inbox-item-head { display: flex; align-items: center; gap: 6px; min-width: 0; }
.inbox-item-source { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; font-size: var(--ip-text-caption-size); font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-primary); }
.inbox-item-time { flex-shrink: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); }
.inbox-item-content {
  margin: 0; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); line-height: 1.5;
  display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden;
  white-space: pre-wrap; word-break: break-word;
}
.inbox-item-status { margin: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); }
.inbox-item-actions { display: flex; align-items: center; gap: 6px; }
.inbox-act {
  padding: 3px 10px; border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-sm);
  background: transparent; color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-caption-size); font-family: inherit; cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.inbox-act:hover:not(:disabled) { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-primary); }
.inbox-act:disabled { opacity: 0.5; cursor: not-allowed; }
.inbox-act-approve { color: var(--ip-primary-600); border-color: var(--ip-primary-soft-border, rgba(var(--ip-primary-500-rgb), 0.35)); }
.inbox-act-approve:hover:not(:disabled) { background: var(--ip-primary-soft-bg); color: var(--ip-primary-600); }
/* 拒绝两步确认：武装态转 danger 确认键（第一击武装、第二击执行、blur 解除） */
.inbox-act-armed { color: #fff; background: var(--ip-danger-base); border-color: var(--ip-danger-base); }
.inbox-act-armed:hover:not(:disabled) { color: #fff; background: var(--ip-danger-base); opacity: 0.9; }
.inbox-reply-flag { display: inline-flex; align-items: center; gap: 3px; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); }
.inbox-reply-flag svg { display: inline-block; }

.inbox-policy { display: flex; flex-direction: column; gap: 6px; padding-top: var(--ip-spacing-2); border-top: 1px solid var(--ip-color-border-default); }
.inbox-policy-label { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.inbox-policy-seg { display: inline-flex; padding: 2px; background: var(--ip-color-bg-tertiary); border-radius: var(--ip-radius-md); gap: 2px; }
.inbox-policy-btn {
  /* 内容自适应最小宽 + 余量均分（非等分三份——「自动接收（默认）」最长，
     等分档容不下会被挤成两行）；nowrap 是排版契约，恒单行 */
  flex: 1 0 auto; white-space: nowrap;
  padding: 4px 10px; border: none; border-radius: var(--ip-radius-sm);
  background: transparent; color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-caption-size); font-family: inherit; cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.inbox-policy-btn.active { background: var(--ip-color-bg-elevated); color: var(--ip-color-text-primary); font-weight: var(--ip-font-weight-medium); box-shadow: var(--ip-shadow-sm); }
.inbox-policy-hint { margin: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); line-height: 1.4; }
</style>
