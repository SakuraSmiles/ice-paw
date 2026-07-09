// 计数器 Store：演示 Pinia 的 state / actions / getters
// 使用 Composition API 风格的 setup store（推荐写法，类型推导更友好）
import { computed, ref } from "vue";
import { defineStore } from "pinia";

/**
 * 计数器 store
 * - state: count（数值）、history（操作历史）
 * - actions: increment / decrement / reset（自动追加历史记录）
 * - getters: doubleCount（2 倍）、historyCount（历史长度）
 */
export const useCounterStore = defineStore("counter", () => {
  // ===== state（使用 ref 声明） =====
  const count = ref(0);
  const history = ref<string[]>([]);

  // ===== actions（普通函数即可，无需 mutation 概念） =====

  /** 自增 +1，并追加一条历史记录 */
  function increment() {
    count.value += 1;
    history.value.push(`increment → ${count.value}`);
  }

  /** 自减 -1，并追加一条历史记录 */
  function decrement() {
    count.value -= 1;
    history.value.push(`decrement → ${count.value}`);
  }

  /** 重置计数为 0，并追加一条历史记录 */
  function reset() {
    count.value = 0;
    history.value.push("reset → 0");
  }

  // ===== getters（使用 computed 声明） =====

  /** count 的 2 倍 */
  const doubleCount = computed(() => count.value * 2);

  /** 历史记录条数 */
  const historyCount = computed(() => history.value.length);

  // 必须从 setup store 中返回 state / getters / actions
  return {
    count,
    history,
    doubleCount,
    historyCount,
    increment,
    decrement,
    reset,
  };
});
