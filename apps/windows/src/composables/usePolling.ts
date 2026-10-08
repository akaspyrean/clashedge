// src/composables/usePolling.ts - 可见性感知的自适应轮询。
//
// - 窗口隐藏（最小化到托盘）时暂停，恢复可见时立即拉一次；
// - 上一轮请求未返回时跳过本轮（避免慢请求与定时器堆叠、乱序覆盖）；
// - 每轮结束后按 `intervalFor()` 重新计算下一轮间隔（连接数大时放慢）。

import { onMounted, onUnmounted } from "vue";

export function usePolling(task: () => Promise<void>, intervalFor: () => number) {
  let timer: number | undefined;
  let inFlight = false;

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
    } catch {
      // 轮询失败静默：下一轮重试。
    } finally {
      inFlight = false;
      stop();
      if (!document.hidden) start(); // 按最新数据量重算间隔；窗口已隐藏则不再续订
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
