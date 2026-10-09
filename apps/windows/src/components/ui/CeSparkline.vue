<!-- src/components/ui/CeSparkline.vue - 迷你流量曲线（首页）。
     下载 + 上传两条线共用同一纵轴；最新点在最右。纯装饰性图形，读屏用 label 概括。 -->
<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{ down: number[]; up: number[]; label: string; window?: number }>(),
  { window: 60 },
);

const W = 300;
const H = 56;
const PAD = 3;

const max = computed(() => Math.max(1, ...props.down, ...props.up));

function points(series: number[]): string {
  const n = series.length;
  if (n === 0) return "";
  const offset = props.window - n;
  return series
    .map((v, i) => {
      const x = ((i + offset) / (props.window - 1)) * W;
      const y = H - PAD - (v / max.value) * (H - PAD * 2);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

const downPoints = computed(() => points(props.down));
const upPoints = computed(() => points(props.up));
const areaPoints = computed(() => {
  const n = props.down.length;
  if (n === 0) return "";
  const x0 = ((props.window - n) / (props.window - 1)) * W;
  return `${x0.toFixed(1)},${H} ${downPoints.value} ${W},${H}`;
});
</script>

<template>
  <svg
    class="ce-spark"
    :viewBox="`0 0 ${W} ${H}`"
    preserveAspectRatio="none"
    role="img"
    :aria-label="label"
  >
    <polygon v-if="areaPoints" class="ce-spark__area" :points="areaPoints" />
    <polyline v-if="downPoints" class="ce-spark__down" :points="downPoints" />
    <polyline v-if="upPoints" class="ce-spark__up" :points="upPoints" />
  </svg>
</template>

<style scoped>
.ce-spark {
  display: block;
  width: 100%;
  height: 56px;
}

.ce-spark__down,
.ce-spark__up {
  fill: none;
  stroke-width: 2;
  stroke-linejoin: round;
  stroke-linecap: round;
  vector-effect: non-scaling-stroke;
}

.ce-spark__down {
  stroke: var(--ce-chart-down);
}

.ce-spark__up {
  stroke: var(--ce-chart-up);
}

.ce-spark__area {
  fill: var(--ce-chart-down);
  opacity: 0.07;
}
</style>
