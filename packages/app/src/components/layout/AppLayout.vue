<!-- AppLayout — 应用主布局：左侧 Sidebar + 右侧 router-view -->
<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted } from "vue";
import { useRoute } from "vue-router";
import Sidebar from "../chat/Sidebar.vue";
import AuthNoticeStack from "../chat/AuthNoticeStack.vue";
import TitleBar from "./TitleBar.vue";

/** 启动入场编排（感官升级批 2）：触发信号 = main.ts finish() 派发的
 * icepaw:splash-out——2026-10-29 无感衔接演进：入场信号前移到编舞**中段**
 * （t=550 SPLASH_HANDOFF_MS，非节点移除后），「加载元素退场」与「应用骨架
 * 入场」交叠成 crossfade——侧栏 t=700 / 消息区 t=850 起跑，落在环场淡净（800）
 * 前后，无亮色重叠、无纯底色空窗。（⚠️ 挂 boot-ready 会在不透明骨架背后
 * 提前播完，淡出时 stagger 已结束；勿回退到 t=0 同帧起跑——2026-10-28 实机
 * 否决：首页与 0~350ms 满亮度环组重叠。）兜底两道：splash 节点已不在（退场后再挂载，如
 * HMR 重挂 → 直接入场）/ 15.5s 超时（main.ts 真实兜底 = 14s 强制退场 + 950ms
 * 节点移除 ≈ 14.95s；须晚于它——提前入场会被不透明 splash 盖住且点击穿透成
 * 盲交互窗，批次 Z/B3 修正了按旧值 10s+360ms 算出的 11.5s 漂移），
 * 上报链断裂时 app 不得永远隐身。reduced-motion 下全局 transition 归零
 * （tokens §12），入场瞬时完成。 */
const entered = ref(false);
let enterTimer = 0;
function markEntered() {
  entered.value = true;
}
onMounted(() => {
  if (!document.getElementById("boot-splash")) {
    markEntered();
    return;
  }
  window.addEventListener("icepaw:splash-out", markEntered, { once: true });
  enterTimer = window.setTimeout(markEntered, 15500);
});
onUnmounted(() => {
  window.removeEventListener("icepaw:splash-out", markEntered);
  window.clearTimeout(enterTimer);
});

const route = useRoute();

/** 页级过渡（感官升级批 1）：方向感知——按路由嵌套深度（route.matched
 * 长度）判定前进/后退：跨层跳转（会话→项目列表→项目详情）走侧向滑动
 * 表达层级方向，同层跳转（设置二级页、项目内 tab）保持淡入。
 * matched 长度天然等于路由表嵌套层级，无需手写 depth 映射。
 * watch pre-flush 在重渲染前更新过渡名，Transition 以新名接管进出对。 */
const routeTransition = ref("route-fade");
let prevDepth = route.matched.length;
watch(
  () => route.matched.length,
  (depth) => {
    const delta = depth - prevDepth;
    prevDepth = depth;
    routeTransition.value =
      delta > 0 ? "route-slide-forward" : delta < 0 ? "route-slide-back" : "route-fade";
  },
);
</script>

<template>
  <div class="app-shell" :class="{ 'is-entered': entered }">
    <!-- 侧栏顶到顶部（2026-10-29 拍板）：全高悬浮浮板列 + 右侧内容列
         （TitleBar 只罩内容区，不再整宽压顶——Arc/Linear 形态）。侧栏
         header 的空白区兼任拖拽区（见 Sidebar），窗口拖拽面不缩水 -->
    <Sidebar />
    <div class="app-right">
      <!-- 自绘窗口栏（感官升级批 3 · L3）：decorations:false 后承接拖拽/三键 -->
      <TitleBar />
      <main class="app-main">
        <router-view v-slot="{ Component }">
          <!-- 页级过渡（UX #7 + 批 1 方向感知）：out-in 顺序切换（无叠影）；
               KeepAlive 缓存与滚动位置不受影响 -->
          <Transition :name="routeTransition" mode="out-in">
            <keep-alive>
              <component :is="Component" />
            </keep-alive>
          </Transition>
        </router-view>
      </main>
    </div>
  </div>

  <!-- 后台会话授权通知栈（#10 后台分支）：全局挂载，任何页面都可见可跳转 -->
  <AuthNoticeStack />
</template>

<style scoped>
/* 横向壳（2026-10-29 侧栏顶到顶部拍板）：全高侧栏列 + 右侧内容列
   （TitleBar 36px + app-main）。侧栏由本行 stretch 撑满全高。 */
.app-shell {
  display: flex;
  height: 100vh;
  width: 100vw;
  overflow: hidden;
}

/* ---- 启动入场 · 渐进建设（形态 A）：TitleBar 与内容区解耦——
   TitleBar **不等退场编舞**（三键立刻可见可用确定项，2026-10-27）：mount 即
   渲染即显示，自带 300ms 入场淡入；splash 已让出顶部 36px 条带，主线程解冻
   （~0.65s）≈ mount（~1.4s）时刻起可拖可点。内容区两拍在 splash-out 信号
   （t=550 SPLASH_HANDOFF_MS，编舞中段）后起跑：侧栏 t=700（自左滑入）→
   消息区 t=850（自下浮现），与环场淡净（800）交叠成无感衔接。
   Sidebar 是子组件根（.sidebar 继承父 scope id 可选中）。 ---- */
.titlebar {
  animation: titlebar-in 300ms var(--ip-ease-out, ease) both;
}
@keyframes titlebar-in {
  from { opacity: 0; transform: translateY(-4px); }
  to { opacity: 1; transform: translateY(0); }
}
.app-shell:not(.is-entered) .sidebar {
  opacity: 0;
  transform: translateX(-14px);
}
.app-shell:not(.is-entered) .app-main {
  opacity: 0;
  transform: translateY(6px);
}
.sidebar,
.app-main {
  transition:
    opacity 350ms var(--ip-ease-out, ease),
    transform 350ms var(--ip-ease-out, ease);
}
.sidebar {
  transition-delay: 150ms;
}
.app-main {
  transition-delay: 300ms;
}
.app-right {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.app-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  /* 内容区随窗口基底（2026-10-28 基底归一）：原 bg-secondary「亮纸面」是旧分区
     模式遗留——悬浮面板模型下内容平铺在基底上（L1 沉降层），白纸/灰亮纸与
     基底的色差就是「割裂感」本体（实测浅 #FFFFFF vs 基底 #FAFBFC、暗
     #1C222B vs #14181F 差 8 阶更明显）。回滚 = revert 本行 */
  background-color: var(--ip-color-bg-base);
}
</style>
