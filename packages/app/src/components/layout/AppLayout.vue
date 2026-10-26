<!-- AppLayout — 应用主布局：左侧 Sidebar + 右侧 router-view -->
<script setup lang="ts">
import { ref, watch } from "vue";
import { useRoute } from "vue-router";
import Sidebar from "../chat/Sidebar.vue";
import AuthNoticeStack from "../chat/AuthNoticeStack.vue";
import TitleBar from "./TitleBar.vue";

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
  <div class="app-shell">
    <!-- 自绘窗口栏（感官升级批 3 · L3）：decorations:false 后承接拖拽/三键，
         全宽压顶与侧栏同底色成「框」——见 TitleBar 组件头取舍注释 -->
    <TitleBar />
    <div class="app-layout">
      <Sidebar />
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
/* 纵向壳：TitleBar（36px 固定）+ 原横向布局（占余高）。100vh 归属从
   .app-layout 上移到壳层，内层改 flex:1（min-height:0 保滚动语义）。 */
.app-shell {
  display: flex;
  flex-direction: column;
  height: 100vh;
  width: 100vw;
  overflow: hidden;
}

.app-layout {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.app-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  background-color: var(--ip-color-bg-secondary);
}
</style>
