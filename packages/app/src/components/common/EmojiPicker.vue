<!--
  EmojiPicker — 头像 emoji 选择器（策展平铺网格，无搜索无分类——保持小）

  Props: 无（受控弹出由调用方负责；组件只管选）
  Emits: select: [emoji: string]（选定）/ clear: []（清除选择回兜底）
-->
<script setup lang="ts">
/** 策展 emoji 集（~144 个，8 列 × 18 行滚动）。身份语境优先：动物/物件/符号。 */
const EMOJIS: ReadonlyArray<readonly string[]> = [
  ["🦊", "🐺", "🐶", "🐱", "🐯", "🦁", "🐻", "🐼"],
  ["🐨", "🐰", "🐸", "🐷", "🐮", "🐔", "🐧", "🦉"],
  ["🦅", "🦇", "🐴", "🦄", "🐝", "🦋", "🐌", "🐞"],
  ["🐢", "🐍", "🐙", "🦑", "🦀", "🐠", "🐬", "🐳"],
  ["🦈", "🐊", "🦕", "🦖", "🐾", "🦴", "🌲", "🌳"],
  ["🌵", "🍀", "🌸", "🌻", "🌙", "⭐", "🌟", "✨"],
  ["☄️", "🔥", "💧", "🌊", "❄️", "🧊", "⚡", "🌈"],
  ["☀️", "☁️", "🎈", "🎉", "🎯", "🧩", "🎲", "🎸"],
  ["🎺", "🎻", "🥁", "🎤", "🎧", "📜", "📚", "✏️"],
  ["🖊️", "🖌️", "📐", "📏", "🔖", "🔗", "📎", "🗂️"],
  ["📁", "📂", "🗃️", "🗄️", "🗒️", "🗓️", "⏰", "⏳"],
  ["🔒", "🔑", "🔨", "🛠️", "⚙️", "🧰", "🧪", "🔬"],
  ["🔭", "💻", "🖥️", "⌨️", "🖱️", "💾", "💿", "📀"],
  ["📱", "📷", "🎥", "📞", "📡", "🔋", "💡", "🔦"],
  ["🌍", "🧭", "🗺️", "🏠", "🏢", "🏰", "🚀", "🛸"],
  ["🚗", "✈️", "⛵", "🚲", "🏁", "🏆", "🥇", "🎖️"],
  ["💎", "👑", "🎪", "🎭", "🎨", "🪄", "🧿", "🛡️"],
  ["⚔️", "🏹", "🧙", "🦸", "👻", "🤖", "👽", "🎃"],
];

const flat = EMOJIS.flat();

const emit = defineEmits<{ select: [emoji: string]; clear: [] }>();
</script>

<template>
  <div class="emoji-picker" role="listbox" aria-label="选择 emoji 头像">
    <div class="emoji-grid">
      <button
        v-for="e in flat"
        :key="e"
        type="button"
        class="emoji-cell"
        role="option"
        :aria-label="e"
        @click="emit('select', e)"
      >
        {{ e }}
      </button>
    </div>
    <button type="button" class="emoji-clear" @click="emit('clear')">不使用 emoji（按名字生成）</button>
  </div>
</template>

<style scoped>
.emoji-picker {
  width: 272px; /* 8 列 × 28px + 间距 —— 显式定宽防内容自适应 wrap（popover 踩坑教训） */
  padding: 8px;
}

.emoji-grid {
  display: grid;
  grid-template-columns: repeat(8, 1fr);
  gap: 2px;
  max-height: 224px;
  overflow-y: auto;
}

.emoji-cell {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 28px;
  font-size: 17px;
  line-height: 1;
  border: 0;
  border-radius: var(--ip-radius-sm, 4px);
  background: transparent;
  cursor: pointer;
  padding: 0;
}

.emoji-cell:hover {
  background: var(--ip-color-bg-tertiary);
}

.emoji-clear {
  width: 100%;
  margin-top: 6px;
  padding: 6px 0;
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-secondary);
  background: transparent;
  border: 0;
  border-top: 1px solid var(--ip-color-border-default);
  border-radius: 0;
  cursor: pointer;
}

.emoji-clear:hover {
  color: var(--ip-color-text-primary);
}
</style>
