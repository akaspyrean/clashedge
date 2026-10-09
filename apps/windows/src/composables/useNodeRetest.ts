// src/composables/useNodeRetest.ts - 节点格上的「重新测速」：按节点名维护在途集合，
// 同一节点连点不重复发起；首页「快捷切换」与代理页共用。
import { ref } from "vue";
import { useProxyStore } from "@/stores/proxy";

export function useNodeRetest() {
  const proxyStore = useProxyStore();
  const testing = ref(new Set<string>());

  async function retest(name: string) {
    if (testing.value.has(name)) return;
    testing.value = new Set(testing.value).add(name);
    try {
      await proxyStore.testNode(name);
    } finally {
      const next = new Set(testing.value);
      next.delete(name);
      testing.value = next;
    }
  }

  return { testing, retest };
}
