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
  let disposed = false;
  let failures = 0;
  const { fail } = useNotify();

  function stop() {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
  }

  function start() {
    if (disposed || document.hidden || timer !== undefined || inFlight) return;
    const base = Math.max(500, intervalFor());
    const delay = Math.min(60_000, base * 2 ** Math.min(failures, 5));
    timer = window.setTimeout(() => {
      timer = undefined;
      void tick();
    }, delay);
  }

  async function tick() {
    if (disposed || document.hidden || inFlight) return;
    stop();
    inFlight = true;
    try {
      await task();
      if (!disposed) failures = 0;
    } catch (e) {
      if (!disposed) {
        failures++;
        // Persistent outages should not produce a toast every polling interval.
        if (failures === 1 || failures === 5) fail(e, "轮询失败：");
      }
    } finally {
      inFlight = false;
      if (!disposed) start();
    }
  }

  function onVisibilityChange() {
    if (document.hidden) stop();
    else void tick();
  }

  onMounted(() => {
    disposed = false;
    void tick();
    document.addEventListener("visibilitychange", onVisibilityChange);
  });
  onUnmounted(() => {
    disposed = true;
    stop();
    document.removeEventListener("visibilitychange", onVisibilityChange);
  });

  return { tick, stop, start };
}
