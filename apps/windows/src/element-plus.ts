// src/element-plus.ts - Element Plus 按需注册
//
// 之前 main.ts 里 `app.use(ElementPlus)` + 引入整份 dist/index.css + 全部图标全局注册，
// 打进包里的是整个组件库。托盘常驻应用的 WebView 内存 / 启动时间都与此相关（审计 C3）。
// 现在只注册 UI 实际用到的组件，并只引入这些组件（及其依赖）的样式。
//
// 新增使用 `<el-xxx>` 的组件时：在 COMPONENTS 里补上组件，并在下面补一行对应的
// `element-plus/es/components/xxx/style/css`（缺样式表现为"组件无样式"，很容易发现）。

import type { App, Plugin } from "vue";
import {
  ElAlert,
  ElAside,
  ElButton,
  ElCard,
  ElCheckbox,
  ElCollapse,
  ElCollapseItem,
  ElConfigProvider,
  ElContainer,
  ElDescriptions,
  ElDescriptionsItem,
  ElDialog,
  ElDropdown,
  ElDropdownItem,
  ElDropdownMenu,
  ElEmpty,
  ElForm,
  ElFormItem,
  ElIcon,
  ElInput,
  ElInputNumber,
  ElMain,
  ElMenu,
  ElMenuItem,
  ElOption,
  ElRadioButton,
  ElRadioGroup,
  ElSelect,
  ElSwitch,
  ElTable,
  ElTableColumn,
  ElTabPane,
  ElTabs,
  ElTag,
} from "element-plus";

// 样式：base 含全部 CSS 变量；其余为各组件（及其依赖，如 popper/overlay）样式。
import "element-plus/es/components/base/style/css";
import "element-plus/es/components/alert/style/css";
import "element-plus/es/components/aside/style/css";
import "element-plus/es/components/button/style/css";
import "element-plus/es/components/card/style/css";
import "element-plus/es/components/checkbox/style/css";
import "element-plus/es/components/collapse/style/css";
import "element-plus/es/components/collapse-item/style/css";
import "element-plus/es/components/container/style/css";
import "element-plus/es/components/descriptions/style/css";
import "element-plus/es/components/descriptions-item/style/css";
import "element-plus/es/components/dialog/style/css";
import "element-plus/es/components/dropdown/style/css";
import "element-plus/es/components/dropdown-item/style/css";
import "element-plus/es/components/dropdown-menu/style/css";
import "element-plus/es/components/empty/style/css";
import "element-plus/es/components/form/style/css";
import "element-plus/es/components/form-item/style/css";
import "element-plus/es/components/icon/style/css";
import "element-plus/es/components/input/style/css";
import "element-plus/es/components/input-number/style/css";
import "element-plus/es/components/main/style/css";
import "element-plus/es/components/menu/style/css";
import "element-plus/es/components/menu-item/style/css";
import "element-plus/es/components/option/style/css";
import "element-plus/es/components/radio-button/style/css";
import "element-plus/es/components/radio-group/style/css";
import "element-plus/es/components/select/style/css";
import "element-plus/es/components/switch/style/css";
import "element-plus/es/components/table/style/css";
import "element-plus/es/components/table-column/style/css";
import "element-plus/es/components/tab-pane/style/css";
import "element-plus/es/components/tabs/style/css";
import "element-plus/es/components/tag/style/css";
// 服务式组件（ElMessage / ElMessageBox / ElNotification）的样式
import "element-plus/es/components/message/style/css";
import "element-plus/es/components/message-box/style/css";
import "element-plus/es/components/notification/style/css";

const COMPONENTS: Plugin[] = [
  ElAlert,
  ElAside,
  ElButton,
  ElCard,
  ElCheckbox,
  ElCollapse,
  ElCollapseItem,
  ElConfigProvider,
  ElContainer,
  ElDescriptions,
  ElDescriptionsItem,
  ElDialog,
  ElDropdown,
  ElDropdownItem,
  ElDropdownMenu,
  ElEmpty,
  ElForm,
  ElFormItem,
  ElIcon,
  ElInput,
  ElInputNumber,
  ElMain,
  ElMenu,
  ElMenuItem,
  ElOption,
  ElRadioButton,
  ElRadioGroup,
  ElSelect,
  ElSwitch,
  ElTable,
  ElTableColumn,
  ElTabPane,
  ElTabs,
  ElTag,
];

export function installElementPlus(app: App): void {
  for (const c of COMPONENTS) app.use(c);
}
