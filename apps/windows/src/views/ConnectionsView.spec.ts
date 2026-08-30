import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

// Mock @tauri-apps/api/core 的 invoke，ConnectionsView 通过 connectionsApi.list
// 拉连接数据。测试控制返回内容以驱动 v-if/v-else 三态渲染。
const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import ElementPlus from "element-plus";
import ConnectionsView from "@/views/ConnectionsView.vue";

function makeConn(id: string) {
  return {
    id,
    host: "h" + id,
    network: "TCP",
    type: "test",
    rule: "DIRECT",
    upload: 0,
    download: 0,
    start: Date.now(),
    chains: [],
  };
}

function mockList(connections: ReturnType<typeof makeConn>[], total?: number) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_connections") {
      // P2：后端已裁剪，返回 total（真实总数，可大于 connections.length）
      return Promise.resolve({
        download_total: 0,
        upload_total: 0,
        total: total ?? connections.length,
        truncated: (total ?? connections.length) > connections.length,
        connections,
      });
    }
    return Promise.resolve(undefined);
  });
}

/** 组件模板用 $t('connections.xxx') 渲染文案。vue-i18n 完整初始化需拉
 *  后端消息表，对单组件测试过重；mocks.$t 直接回退 key 足够断言渲染分支。 */
const globalMocks = {
  $t: (k: string) => k,
};

const mountOptions = {
  global: {
    plugins: [ElementPlus],
    mocks: globalMocks,
    stubs: {
      // el-empty 内部用 teleport+transition，stub 掉避免 jsdom 警告
      "el-empty": { template: '<div class="stub-empty" />' },
    },
  },
};

describe("ConnectionsView (v-if/v-else 三态)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("有连接（<500）→ 表格显示，不显示 empty，不显示截断提示", async () => {
    mockList([makeConn("1"), makeConn("2")]);
    const wrapper = mount(ConnectionsView, mountOptions);
    await flushPromises();
    wrapper.unmount();

    const table = wrapper.find(".connections-table");
    expect(table.exists()).toBe(true);

    // 回归点：有连接时 el-empty 不应出现
    const empty = wrapper.find(".stub-empty");
    expect(empty.exists()).toBe(false);

    const notice = wrapper.find(".truncated-notice");
    expect(notice.exists()).toBe(false);
  });

  it("无连接 → empty 显示，表格不显示，截断提示不显示", async () => {
    mockList([]);
    const wrapper = mount(ConnectionsView, mountOptions);
    await flushPromises();
    wrapper.unmount();

    const table = wrapper.find(".connections-table");
    expect(table.exists()).toBe(false);

    const empty = wrapper.find(".stub-empty");
    expect(empty.exists()).toBe(true);

    const notice = wrapper.find(".truncated-notice");
    expect(notice.exists()).toBe(false);
  });

  it("连接 > 500 → 表格显示 + 截断提示显示，empty 不显示", async () => {
    const conns = Array.from({ length: 600 }, (_, i) => makeConn(String(i)));
    mockList(conns);
    const wrapper = mount(ConnectionsView, mountOptions);
    await flushPromises();
    wrapper.unmount();

    const table = wrapper.find(".connections-table");
    expect(table.exists()).toBe(true);

    const empty = wrapper.find(".stub-empty");
    expect(empty.exists()).toBe(false);

    const notice = wrapper.find(".truncated-notice");
    expect(notice.exists()).toBe(true);
  });
});

