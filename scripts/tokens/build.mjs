// scripts/tokens/build.mjs - zero-dependency design token generator.
// Reads design/tokens.json and writes:
//   apps/windows/src/styles/tokens.css   (:root = global + light, html[data-theme="dark"] = dark)
//   apps/android/.../ui/theme/Tokens.kt  (colors + radii for Compose)
// Run from apps/windows: `npm run tokens`. Output is deterministic (LF, stable order),
// so CI can verify it with `git diff --exit-code`.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const tokens = JSON.parse(readFileSync(resolve(root, "design", "tokens.json"), "utf8"));
const cssOut = resolve(root, "apps", "windows", "src", "styles", "tokens.css");
const ktOut = resolve(
  root,
  "apps/android/app/src/main/java/com/clashedge/android/ui/theme/Tokens.kt",
);

for (const group of ["global", "light", "dark"]) {
  if (!tokens[group] || typeof tokens[group] !== "object") {
    throw new Error(`design/tokens.json: missing group "${group}"`);
  }
}
const lightKeys = Object.keys(tokens.light);
const darkKeys = Object.keys(tokens.dark);
const missing = lightKeys.filter((k) => !(k in tokens.dark)).concat(darkKeys.filter((k) => !(k in tokens.light)));
if (missing.length > 0) {
  throw new Error(`design/tokens.json: light/dark key mismatch: ${missing.join(", ")}`);
}

const decls = (obj, indent = "  ") =>
  Object.entries(obj)
    .map(([k, v]) => `${indent}--ce-${k}: ${v};`)
    .join("\n");

const css = `/* AUTO-GENERATED — do not edit.
 * Source: design/tokens.json  ·  Generator: scripts/tokens/build.mjs  ·  Run: npm run tokens */

:root {
  color-scheme: light;
${decls(tokens.global)}

${decls(tokens.light)}
}

html[data-theme="dark"] {
  color-scheme: dark;
${decls(tokens.dark)}
}
`;

const camel = (k) => k.replace(/-([a-z0-9])/g, (_, c) => c.toUpperCase());
const hexRe = /^#([0-9A-Fa-f]{6})$/;
const ktColors = (obj) =>
  Object.entries(obj)
    .filter(([, v]) => hexRe.test(v))
    .map(([k, v]) => `        val ${camel(k)} = Color(0xFF${v.slice(1).toUpperCase()})`)
    .join("\n");
const ktRadii = Object.entries(tokens.global)
  .filter(([k, v]) => k.startsWith("radius-") && /^\d+px$/.test(v))
  .map(([k, v]) => `    val ${camel(k)}: Dp = ${parseInt(v, 10)}.dp`)
  .join("\n");

const kt = `// AUTO-GENERATED — do not edit.
// Source: design/tokens.json  ·  Generator: scripts/tokens/build.mjs  ·  Run: npm run tokens (apps/windows)
package com.clashedge.android.ui.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

object CeTokens {
${ktRadii}

    object Light {
${ktColors(tokens.light)}
    }

    object Dark {
${ktColors(tokens.dark)}
    }
}
`;

mkdirSync(dirname(cssOut), { recursive: true });
mkdirSync(dirname(ktOut), { recursive: true });
writeFileSync(cssOut, css, "utf8");
writeFileSync(ktOut, kt, "utf8");
console.log(`tokens: wrote ${cssOut}`);
console.log(`tokens: wrote ${ktOut}`);
