<!-- src/components/ui/CeTrafficChart.vue - 60 秒流量图（手写 SVG，无图表库）。
     线宽 2；只有下载线带 7% 面积填充；每秒更新、不做补间；单一 Y 轴。
     悬停显示十字线 + Tooltip（数值中性色，色点标识系列）；「查看数据表」给读屏/键盘用户同样的数据。 -->
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { formatRate } from "@/utils/format";

const props = withDefaults(
  defineProps<{
    /** 下载速率（字节/秒），最旧在前、最新在后 */
    down: number[];
    up: number[];
    /** 窗口秒数 */
    window?: number;
    height?: number;
  }>(),
  { window: 60, height: 168 },
);
const { t } = useI18n();

const wrap = ref<HTMLElement | null>(null);
const width = ref(640);
let observer: ResizeObserver | null = null;

onMounted(() => {
  if (!wrap.value) return;
  width.value = wrap.value.clientWidth || width.value;
  // 无 ResizeObserver 的环境（测试 / 极旧 WebView）退回初始宽度。
  if (typeof ResizeObserver === "undefined") return;
  observer = new ResizeObserver(() => {
    width.value = wrap.value?.clientWidth || width.value;
  });
  observer.observe(wrap.value);
});
onUnmounted(() => observer?.disconnect());

const PAD_L = 72;
const PAD_R = 8;
const PAD_T = 8;
const PAD_B = 24;
const plotW = computed(() => Math.max(1, width.value - PAD_L - PAD_R));
const plotH = computed(() => props.height - PAD_T - PAD_B);

/** 1-2-5 序列向上取整，保证刻度干净；下限 1 KB/s，空图也有合理刻度。 */
function niceCeil(v: number): number {
  const x = Math.max(v, 1024);
  const pow = Math.pow(10, Math.floor(Math.log10(x)));
  for (const m of [1, 2, 5, 10]) if (x <= m * pow) return m * pow;
  return 10 * pow;
}

const yMax = computed(() => niceCeil(Math.max(0, ...props.down, ...props.up)));
const ticks = computed(() =>
  [0, 1, 2, 3].map((i) => ({
    value: (yMax.value * i) / 3,
    y: PAD_T + plotH.value - (plotH.value * i) / 3,
  })),
);

function xAt(i: number, n: number): number {
  const offset = props.window - n;
  return PAD_L + ((i + offset) / (props.window - 1)) * plotW.value;
}
function yAt(v: number): number {
  return PAD_T + plotH.value - (v / yMax.value) * plotH.value;
}
function line(series: number[]): string {
  return series.map((v, i) => `${xAt(i, series.length).toFixed(1)},${yAt(v).toFixed(1)}`).join(" ");
}

const downLine = computed(() => line(props.down));
const upLine = computed(() => line(props.up));
const downArea = computed(() => {
  const n = props.down.length;
  if (n === 0) return "";
  const base = PAD_T + plotH.value;
  return `${xAt(0, n).toFixed(1)},${base} ${downLine.value} ${xAt(n - 1, n).toFixed(1)},${base}`;
});

const latestDown = computed(() => props.down[props.down.length - 1] ?? 0);
const latestUp = computed(() => props.up[props.up.length - 1] ?? 0);
const ariaLabel = computed(() =>
  t("ui.chart.aria", { down: formatRate(latestDown.value), up: formatRate(latestUp.value) }),
);

// ---- 悬停 ----
const hover = ref<number | null>(null);

function onMove(e: MouseEvent) {
  const n = props.down.length;
  if (n === 0) return;
  const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
  const x = e.clientX - rect.left;
  const offset = props.window - n;
  const idx = Math.round(((x - PAD_L) / plotW.value) * (props.window - 1)) - offset;
  hover.value = idx >= 0 && idx < n ? idx : null;
}

const hoverX = computed(() => (hover.value === null ? 0 : xAt(hover.value, props.down.length)));
const secondsAgo = (i: number) => props.down.length - 1 - i;
const timeText = (i: number) =>
  secondsAgo(i) === 0 ? t("ui.chart.now") : t("ui.chart.seconds_ago", { n: secondsAgo(i) });
const tooltipStyle = computed(() => {
  const flip = hoverX.value > width.value * 0.6;
  return flip
    ? { right: `${width.value - hoverX.value + 12}px`, top: "4px" }
    : { left: `${hoverX.value + 12}px`, top: "4px" };
});

const showTable = ref(false);
const rows = computed(() =>
  props.down.map((d, i) => ({ time: timeText(i), down: d, up: props.up[i] ?? 0 })).reverse(),
);
</script>

<template>
  <div class="ce-chart">
    <div ref="wrap" class="ce-chart__plot">
      <svg
        :width="width"
        :height="height"
        role="img"
        :aria-label="ariaLabel"
        @mousemove="onMove"
        @mouseleave="hover = null"
      >
        <g v-for="tk in ticks" :key="tk.y">
          <line class="ce-chart__grid" :x1="PAD_L" :x2="width - PAD_R" :y1="tk.y" :y2="tk.y" />
          <text class="ce-chart__axis" :x="PAD_L - 8" :y="tk.y + 4" text-anchor="end">
            {{ formatRate(tk.value) }}
          </text>
        </g>
        <text class="ce-chart__axis" :x="PAD_L" :y="height - 6">{{ t("ui.chart.seconds_ago", { n: window }) }}</text>
        <text class="ce-chart__axis" :x="width - PAD_R" :y="height - 6" text-anchor="end">{{ t("ui.chart.now") }}</text>
        <polygon v-if="downArea" class="ce-chart__area" :points="downArea" />
        <polyline v-if="downLine" class="ce-chart__down" :points="downLine" />
        <polyline v-if="upLine" class="ce-chart__up" :points="upLine" />
        <g v-if="hover !== null">
          <line class="ce-chart__cross" :x1="hoverX" :x2="hoverX" :y1="PAD_T" :y2="PAD_T + plotH" />
          <circle class="ce-chart__pt ce-chart__pt--down" :cx="hoverX" :cy="yAt(down[hover])" r="4" />
          <circle class="ce-chart__pt ce-chart__pt--up" :cx="hoverX" :cy="yAt(up[hover] ?? 0)" r="4" />
        </g>
      </svg>
      <div v-if="hover !== null" class="ce-chart__tip" :style="tooltipStyle" role="presentation">
        <span class="ce-chart__tip-time">{{ timeText(hover) }}</span>
        <span class="ce-chart__tip-row">
          <i class="ce-chart__dot ce-chart__dot--down" />{{ t("ui.chart.down") }}
          <b>{{ formatRate(down[hover]) }}</b>
        </span>
        <span class="ce-chart__tip-row">
          <i class="ce-chart__dot ce-chart__dot--up" />{{ t("ui.chart.up") }}
          <b>{{ formatRate(up[hover] ?? 0) }}</b>
        </span>
      </div>
    </div>

    <div class="ce-chart__foot">
      <span class="ce-chart__legend"><i class="ce-chart__dot ce-chart__dot--down" />{{ t("ui.chart.down") }}</span>
      <span class="ce-chart__legend"><i class="ce-chart__dot ce-chart__dot--up" />{{ t("ui.chart.up") }}</span>
      <button
        type="button"
        class="ce-chart__toggle"
        :aria-expanded="showTable ? 'true' : 'false'"
        @click="showTable = !showTable"
      >
        {{ showTable ? t("ui.chart.hide_table") : t("ui.chart.show_table") }}
      </button>
    </div>

    <div v-if="showTable" class="ce-chart__table-wrap">
      <table class="ce-chart__table">
        <thead>
          <tr>
            <th scope="col">{{ t("ui.chart.time") }}</th>
            <th scope="col">{{ t("ui.chart.down") }}</th>
            <th scope="col">{{ t("ui.chart.up") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(r, i) in rows" :key="i">
            <th scope="row">{{ r.time }}</th>
            <td>{{ formatRate(r.down) }}</td>
            <td>{{ formatRate(r.up) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.ce-chart {
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-2);
  min-width: 0;
}

.ce-chart__plot {
  position: relative;
  min-width: 0;
}

.ce-chart__plot svg {
  display: block;
}

.ce-chart__grid {
  stroke: var(--ce-chart-grid);
  stroke-width: 1;
}

.ce-chart__axis {
  fill: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
  font-weight: 400;
  font-variant-numeric: tabular-nums;
}

.ce-chart__down,
.ce-chart__up {
  fill: none;
  stroke-width: 2;
  stroke-linejoin: round;
  stroke-linecap: round;
}

.ce-chart__down {
  stroke: var(--ce-chart-down);
}

.ce-chart__up {
  stroke: var(--ce-chart-up);
}

.ce-chart__area {
  fill: var(--ce-chart-down);
  opacity: 0.07;
}

.ce-chart__cross {
  stroke: var(--ce-text-tertiary);
  stroke-width: 1;
  stroke-dasharray: 3 3;
}

.ce-chart__pt {
  stroke: var(--ce-bg-surface);
  stroke-width: 2;
}

.ce-chart__pt--down {
  fill: var(--ce-chart-down);
}

.ce-chart__pt--up {
  fill: var(--ce-chart-up);
}

.ce-chart__tip {
  position: absolute;
  display: flex;
  flex-direction: column;
  gap: var(--ce-space-1);
  min-width: 160px;
  padding: 10px var(--ce-space-3);
  border-radius: var(--ce-radius-md);
  background: var(--ce-bg-elevated);
  box-shadow: var(--ce-shadow-e2);
  color: var(--ce-text-primary);
  font: var(--ce-type-caption);
  font-variant-numeric: tabular-nums;
  pointer-events: none;
}

.ce-chart__tip-time {
  color: var(--ce-text-tertiary);
}

.ce-chart__tip-row {
  display: flex;
  align-items: center;
  gap: var(--ce-space-2);
}

.ce-chart__tip-row b {
  margin-left: auto;
  font-weight: 600;
}

.ce-chart__dot {
  display: inline-block;
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.ce-chart__dot--down {
  background: var(--ce-chart-down);
}

.ce-chart__dot--up {
  background: var(--ce-chart-up);
}

.ce-chart__foot {
  display: flex;
  align-items: center;
  gap: var(--ce-space-4);
  color: var(--ce-text-secondary);
  font: var(--ce-type-callout);
}

.ce-chart__legend {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.ce-chart__toggle {
  margin-left: auto;
  padding: 0 var(--ce-space-2);
  height: var(--ce-control-h-sm);
  border: 0;
  border-radius: var(--ce-radius-sm);
  background: transparent;
  color: var(--ce-accent-fg);
  font: var(--ce-type-callout);
  font-weight: 500;
  cursor: pointer;
}

.ce-chart__toggle:hover {
  background: var(--ce-accent-soft);
}

.ce-chart__toggle:focus-visible {
  outline: 2px solid var(--ce-accent-bg);
  outline-offset: 2px;
}

.ce-chart__table-wrap {
  max-height: 200px;
  overflow: auto;
  border: 1px solid var(--ce-border);
  border-radius: var(--ce-radius-md);
}

.ce-chart__table {
  width: 100%;
  border-collapse: collapse;
  font: var(--ce-type-callout);
  font-variant-numeric: tabular-nums;
}

.ce-chart__table th,
.ce-chart__table td {
  padding: 6px var(--ce-space-3);
  text-align: left;
  border-bottom: 1px solid var(--ce-divider);
  font-weight: 400;
}

.ce-chart__table thead th {
  position: sticky;
  top: 0;
  background: var(--ce-bg-surface);
  color: var(--ce-text-tertiary);
  font: var(--ce-type-caption);
}
</style>
