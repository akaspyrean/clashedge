<!-- src/views/SettingsView.vue - 设置：常规 / 代理 / TUN / 高级 / 关于
     本文件只是外壳：加载配置、按容器宽度切换标签布局，每个标签页是独立组件
     （components/settings/*），各自管理状态与动作。
     配置降级（config.yaml 损坏）的提示与确认在应用顶部横幅（DegradedBanner），不在本页。 -->
<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import AboutTab from "@/components/settings/AboutTab.vue";
import AdvancedTab from "@/components/settings/AdvancedTab.vue";
import GeneralTab from "@/components/settings/GeneralTab.vue";
import ProxyTab from "@/components/settings/ProxyTab.vue";
import TunTab from "@/components/settings/TunTab.vue";
import { useAppStore } from "@/stores/app";
import { useConfigStore } from "@/stores/config";

const appStore = useAppStore();
const configStore = useConfigStore();

// 响应式 tabs：设置页容器宽 < 700px 时切为顶部布局，避免左置标签挤压内容。
const pageEl = ref<HTMLElement | null>(null);
const compactTabs = ref(false);
let resizeObserver: ResizeObserver | undefined;

onMounted(async () => {
  if (!configStore.config) await configStore.load();
  await appStore.loadAutostart();

  resizeObserver = new ResizeObserver((entries) => {
    compactTabs.value = entries[0].contentRect.width < 700;
  });
  if (pageEl.value) resizeObserver.observe(pageEl.value);
});

onUnmounted(() => {
  resizeObserver?.disconnect();
  resizeObserver = undefined;
});
</script>

<template>
  <div v-if="configStore.config" ref="pageEl" class="page">
    <h2 class="page-title">{{ $t("settings.title") }}</h2>

    <el-tabs :tab-position="compactTabs ? 'top' : 'left'" class="settings-tabs">
      <el-tab-pane :label="$t('settings.tabs.general')"><GeneralTab /></el-tab-pane>
      <el-tab-pane :label="$t('settings.tabs.proxy')"><ProxyTab /></el-tab-pane>
      <el-tab-pane :label="$t('settings.tabs.tun')"><TunTab /></el-tab-pane>
      <el-tab-pane :label="$t('settings.tabs.advanced')"><AdvancedTab /></el-tab-pane>
      <el-tab-pane :label="$t('settings.tabs.about')"><AboutTab /></el-tab-pane>
    </el-tabs>
  </div>
</template>
