// scripts/i18n/build.mjs - zero-dependency Android strings generator.
// Reads the `android:` section of resources/i18n/{en-US,zh-CN}.yaml and writes:
//   apps/android/.../res/values/strings.xml     (en-US, default locale)
//   apps/android/.../res/values-zh/strings.xml  (zh-CN)
// Run from apps/windows: `npm run i18n`. Output is deterministic (LF, stable
// source order), so CI can verify it with `git diff --exit-code`.
//
// Scope: this is NOT a general YAML parser on purpose — it only accepts the
// controlled subset used by our i18n resources (2-space indentation, quoted
// scalar values, `#` comments, nested maps). A full YAML dependency would
// violate the zero-dependency npm-scripts rule (see tokens/build.mjs).
// The desktop (vue-i18n + Rust loader) keeps consuming the full yaml; this
// section only feeds Android, and dotted keys map mechanically to resource
// names (a.b -> a_b).

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const sources = {
  "values/strings.xml": {
    yaml: resolve(root, "apps/windows/src-tauri/resources/i18n/en-US.yaml"),
    header: "Source: apps/windows/src-tauri/resources/i18n/en-US.yaml (android:)",
  },
  "values-zh/strings.xml": {
    yaml: resolve(root, "apps/windows/src-tauri/resources/i18n/zh-CN.yaml"),
    header: "Source: apps/windows/src-tauri/resources/i18n/zh-CN.yaml (android:)",
  },
};
const androidDir = resolve(
  root,
  "apps/android/app/src/main/res",
);

// --- controlled-subset YAML ----------------------------------------------

// Pull out exactly one top-level section (`android:` up to the next top-level
// key), so no other section can ever affect the generated files.
function extractSection(text, section) {
  const start = text.match(new RegExp(`^${section}:\\s*(?:#.*)?$`, "m"));
  if (!start) {
    throw new Error(`i18n: missing "${section}:" section`);
  }
  const rest = text.slice(start.index + start[0].length);
  const end = rest.match(/^(?!\s)[A-Za-z_][A-Za-z0-9_]*:/m);
  return end ? rest.slice(0, end.index) : rest;
}

// Parse a nested map (2-space steps). Values must be quoted scalars (like the
// rest of the file); bare scalars are rejected to force consistency.
// The synthetic root frame is anchored one step before the indent of the
// first content line, so returning to that level (sibling keys) can never
// "pop" the root container, while deeper frames are popped until the parent
// for the current indent is on top.
function parseMap(lines) {
  const firstContent = lines.findIndex(
    (l) => l.replace(/\r$/, "").trim() !== "" && !l.trim().startsWith("#"),
  );
  if (firstContent < 0) {
    throw new Error("i18n: no content lines");
  }
  const rootIndent = lines[firstContent].match(/^ */)[0].length;
  const result = {};
  const stack = [{ indent: rootIndent - 2, value: result }];
  for (const raw of lines) {
    const line = raw.replace(/\r$/, "");
    if (!line.trim() || line.trim().startsWith("#")) {
      continue;
    }
    const indent = line.match(/^ */)[0].length;
    const m = line.trim().match(/^("[^"]*"|'[^']*'|[^\s:]+):\s*(.*)$/);
    if (!m) {
      throw new Error(`i18n: unparseable line: ${JSON.stringify(line)}`);
    }
    const [, key, after] = m;
    if (indent % 2 !== 0) {
      throw new Error(`i18n: odd indentation (2-space expected): ${JSON.stringify(line)}`);
    }
    while (stack.length > 1 && indent <= stack[stack.length - 1].indent) {
      stack.pop();
    }
    const parent = stack[stack.length - 1].value;
    if (indent !== stack[stack.length - 1].indent + 2) {
      throw new Error(`i18n: indent jump (2-space expected): ${JSON.stringify(line)}`);
    }
    let value;
    if (after === "") {
      value = {};
      parent[key] = value;
    } else {
      const vm = after.match(/^"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$/);
      if (!vm) {
        throw new Error(`i18n: value must be a double-quoted scalar: ${JSON.stringify(after)}`);
      }
      value = vm[1].replace(/\\(.)/g, "$1");
      parent[key] = value;
    }
    stack.push({ indent, value });
  }
  return result;
}

// Depth-first pre-order flatten; dotted keys map to resource-safe names.
function flatten(obj) {
  const out = [];
  const walk = (node, prefix) => {
    for (const [key, value] of Object.entries(node)) {
      const name = prefix ? `${prefix}.${key}` : key;
      if (typeof value === "string") {
        const resName = name.replace(/\./g, "_");
        if (!/^[A-Za-z][A-Za-z0-9_]*$/.test(resName)) {
          throw new Error(`i18n: invalid resource name "${resName}"`);
        }
        out.push([resName, value]);
      } else if (typeof value === "object") {
        if (name.split(".").length > 2) {
          throw new Error(`i18n: nesting deeper than one group: "${name}"`);
        }
        if (Object.keys(value).length === 0) {
          throw new Error(`i18n: empty group "${name}"`);
        }
        walk(value, name);
      }
    }
  };
  walk(obj, "");
  return out;
}

function parityCheck(en, zh) {
  const enKeys = en.map(([k]) => k).sort();
  const zhKeys = zh.map(([k]) => k).sort();
  const missing = [...enKeys, ...zhKeys].filter(
    (k, i, all) => all.indexOf(k) === i && !(enKeys.includes(k) && zhKeys.includes(k)),
  );
  if (missing.length > 0) {
    throw new Error(`i18n: en/zh key mismatch: ${missing.join(", ")}`);
  }
}

// Android XML text-node escapes: quotes need backslash escapes, XML needs
// character escapes; backslash itself first so order is stable.
function xmlEscape(value) {
  return value
    .replace(/\\/g, "\\\\")
    .replace(/'/g, "\\'")
    .replace(/"/g, '\\"')
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\t/g, "\\t")
    .replace(/\n/g, "\\n");
}

function xmlFor(entries, header) {
  const body = entries
    .map(([name, value]) => `    <string name="${name}">${xmlEscape(value)}</string>`)
    .join("\n");
  return `<?xml version="1.0" encoding="utf-8"?>
<!-- AUTO-GENERATED — do not edit.
     ${header}  ·  Generator: scripts/i18n/build.mjs  ·  Run: npm run i18n (apps/windows) -->
<resources>
${body}
</resources>
`;
}

// Build both languages first so en/zh parity is enforced before anything
// is written (a single missing key must fail the run, not partially update).
const keys = Object.keys(sources);
const built = keys.map((rel) => {
  const { yaml, header } = sources[rel];
  const text = readFileSync(yaml, "utf8");
  const map = parseMap(extractSection(text, "android").split("\n"));
  const entries = flatten(map);
  if (entries.length === 0) {
    throw new Error(`i18n: "android:" section is empty in ${yaml}`);
  }
  return { rel, header, entries };
});
const [first, second] = built.map((b) => b.entries);
parityCheck(first, second);

for (const { rel, header, entries } of built) {
  const out = xmlFor(entries, header);
  const target = resolve(androidDir, rel);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, out, "utf8");
  console.log(`i18n: wrote ${target} (${entries.length} strings)`);
}
