// src/composables/usePolling.ts - 可见性感知的自适应轮询。
//
// - 窗口隐藏（最小化到托盘）时暂停，恢复可见时立即拉一次；
// - 上一轮请求未返回时跳过本轮（避免慢请求与定时器堆叠、乱序覆盖）；
// - 每轮结束后按 `intervalFor()` 重新计算下一轮间隔（连接数大时放慢）。
// - 错误通过 useNotify 上报（不再静默），连续失败会弹出 toast。

import { onMounted, onUnmounted } from "vue";
import { useNotify } from "./useNotify";

export function usePolling(task: () => Promise<void>, intervalFor: () => number) {
  let timer: number | undefined;
  let inFlight = false;
  const { fail } = useNotify();

  function stop() {
    if (timer !== undefined) {
      window.clearInterval(timer);
      timer = undefined;
    }
  }

  function start() {
    if (timer !== undefined) return;
    timer = window.setInterval(() => void tick(), intervalFor());
  }

  async function tick() {
    if (inFlight) return;
    inFlight = true;
    try {
      await task();
    } catch (e) {
      fail(e, "轮询失败：");
    } finally {
      inFlight = false;
      stop();
      if (!document.hidden) start();
    }
  }

  function onVisibilityChange() {
    if (document.hidden) {
      stop();
    } else {
      void tick();
    }
  }

  onMounted(() => {
    void tick();
    document.addEventListener("visibilitychange", onVisibilityChange);
  });
  onUnmounted(() => {
    stop();
    document.removeEventListener("visibilitychange", onVisibilityChange);
  });

  return { tick, stop, start };
}
