// tauri dev 构建在 target/debug/sidecar/ 下查找内核（get_mihomo_path 候选 1/2），
// 但 cargo 不会自动放这个文件，cargo clean 也会清掉它。
// 本脚本幂等地从 build/assets/staging 补齐（缺失只警告，绝不阻塞 dev 启动）。
import { copyFileSync, mkdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const staging = join(repoRoot, "build", "assets", "staging");
const sidecar = join(repoRoot, "apps", "windows", "src-tauri", "target", "debug", "sidecar");

const files = [
  [join(staging, "mihomo", "clash-edge-core.exe"), join(sidecar, "clash-edge-core.exe")],
  [join(staging, "wintun", "wintun.dll"), join(sidecar, "wintun.dll")],
];

let copied = 0;
for (const [src, dst] of files) {
  try {
    const s = statSync(src);
    try {
      if (statSync(dst).size === s.size) continue; // 已就位（文件被执行中也不碰）
    } catch {
      // dst 不存在 → 继续复制
    }
    mkdirSync(sidecar, { recursive: true });
    copyFileSync(src, dst);
    copied += 1;
  } catch (e) {
    console.warn(`[stage-dev-sidecar] skip ${src}: ${e.message}`);
  }
}
if (copied > 0) console.log(`[stage-dev-sidecar] staged ${copied} file(s) -> ${sidecar}`);
