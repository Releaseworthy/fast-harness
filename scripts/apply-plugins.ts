// Turns a plugin list into dependencies of crates/fast-harness-plugins
// (PLAN.md D52, docs/plugins.md). Cargo cannot add dependencies from a
// build script, so this writes them between the marker lines of
// crates/fast-harness-plugins/Cargo.toml; build.rs then generates
// register_all from plugins.toml and fails when the two lists differ.
//
//   bun scripts/apply-plugins.ts [--from FILE] [--base DIR]
//
// --from: a TOML (`[[plugin]]` name, git or path, rev) or JSON
//   (`[{"name","git","rev"}]` or `{"plugin":[...]}`) list; it is written
//   to plugins.toml in the canonical TOML form. Without it plugins.toml
//   is read as it is.
// --base: the folder a relative `path` resolves against (default: the
//   workspace root); the workflow passes the caller repository.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { isAbsolute, join, resolve } from "node:path";

type Plugin = { name: string; git?: string; path?: string; rev?: string; api?: string | number };

const root = resolve(import.meta.dir, "..");
const crateToml = join(root, "crates", "fast-harness-plugins", "Cargo.toml");
const BEGIN = "# BEGIN PLUGINS";
const END = "# END PLUGINS";
// fast_harness_core::ext::API_VERSION; crates/fast-harness-plugins/src/tests.rs checks the copy.
const API_VERSION = 1;

function arg(name: string): string | undefined {
  const at = process.argv.indexOf(name);
  return at >= 0 ? process.argv[at + 1] : undefined;
}

function parseToml(text: string): Plugin[] {
  const plugins: Plugin[] = [];
  text.split(/\r?\n/).forEach((raw, index) => {
    const line = raw.replace(/#[^"]*$/, "").trim();
    if (!line) return;
    if (line === "[[plugin]]") return void plugins.push({ name: "" });
    // A quoted string, or a bare whole number (`api = 1`).
    const match = /^([a-z]+)\s*=\s*(?:"([^"\\]*)"|([0-9]+))$/.exec(line);
    const last = plugins[plugins.length - 1];
    if (!match || !last) throw new Error(`plugins line ${index + 1}: cannot read \`${raw}\``);
    const [, key, quoted, bare] = match;
    const value = quoted ?? bare;
    if (!["name", "git", "path", "rev", "api"].includes(key)) {
      throw new Error(`plugins line ${index + 1}: unknown key \`${key}\` (name, git, path, rev, api)`);
    }
    if (bare !== undefined && key !== "api") {
      throw new Error(`plugins line ${index + 1}: \`${key}\` needs a plain quoted string`);
    }
    (last as Record<string, string>)[key] = value;
  });
  return plugins;
}

function parseList(text: string): Plugin[] {
  const trimmed = text.trim();
  if (!trimmed.startsWith("[{") && !trimmed.startsWith("{") && trimmed !== "[]") {
    return parseToml(text);
  }
  const value = JSON.parse(trimmed);
  const list = Array.isArray(value) ? value : (value.plugin ?? value.plugins ?? []);
  return list.map((p: Plugin) => ({ name: p.name, git: p.git, path: p.path, rev: p.rev, api: p.api }));
}

function check(plugins: Plugin[]): void {
  const seen = new Set<string>();
  for (const p of plugins) {
    if (!/^[A-Za-z0-9_-]+$/.test(p.name ?? "")) throw new Error(`bad plugin name \`${p.name}\``);
    const id = p.name.replaceAll("-", "_");
    if (seen.has(id)) throw new Error(`plugin \`${p.name}\` is listed twice`);
    seen.add(id);
    if (!!p.git === !!p.path) throw new Error(`plugin \`${p.name}\`: set exactly one of git or path`);
    if (p.git && !p.rev) throw new Error(`plugin \`${p.name}\`: a git plugin needs rev (pinned builds)`);
    // Absent reads as 1, like crates/fast-harness-plugins/src/gen.rs.
    const api = p.api === undefined ? 1 : Number(p.api);
    if (!Number.isInteger(api) || String(p.api ?? 1).trim() === "") {
      throw new Error(`plugin \`${p.name}\`: \`api\` needs a whole number, got \`${p.api}\``);
    }
    if (api !== API_VERSION) {
      throw new Error(
        `plugin \`${p.name}\` targets plugin API ${api}, this Fast Harness supports API ${API_VERSION} (fast_harness_core::ext::API_VERSION)`,
      );
    }
  }
}

const q = (s: string) => JSON.stringify(s);

function toToml(plugins: Plugin[]): string {
  const blocks = plugins.map((p) => {
    const lines = ["[[plugin]]", `name = ${q(p.name)}`];
    if (p.git) lines.push(`git = ${q(p.git)}`);
    if (p.path) lines.push(`path = ${q(p.path)}`);
    if (p.rev) lines.push(`rev = ${q(p.rev)}`);
    if (p.api !== undefined) lines.push(`api = ${Number(p.api)}`);
    return lines.join("\n");
  });
  return `# Native plugins of this build (docs/plugins.md); written by scripts/apply-plugins.ts.\n${blocks.join("\n\n")}\n`;
}

function depLine(p: Plugin): string {
  if (p.git) return `${p.name} = { git = ${q(p.git)}, rev = ${q(p.rev!)} }`;
  return `${p.name} = { path = ${q(p.path!.replaceAll("\\", "/"))} }`;
}

/** Replaces the lines between two marker lines of `file` with `body`. */
function between(file: string, begin: string, end: string, body: string): void {
  const text = readFileSync(file, "utf8");
  const start = text.indexOf(begin);
  const stop = text.indexOf(end);
  if (start < 0 || stop < start) throw new Error(`${file} lacks the markers ${begin} / ${end}`);
  writeFileSync(file, `${text.slice(0, start + begin.length)}\n${body}${text.slice(stop)}`);
}

function main(): void {
  const from = arg("--from");
  const base = resolve(arg("--base") ?? root);
  const listPath = join(root, "plugins.toml");
  const text = from ? readFileSync(from, "utf8") : existsSync(listPath) ? readFileSync(listPath, "utf8") : "";
  const plugins = parseList(text);
  check(plugins);
  for (const p of plugins) {
    if (p.path && !isAbsolute(p.path)) p.path = resolve(base, p.path);
    // build.rs reads plain strings without escapes: forward slashes only.
    if (p.path) p.path = p.path.replaceAll("\\", "/");
  }
  if (from) writeFileSync(listPath, toToml(plugins));
  const deps = plugins.map(depLine).map((line) => `${line}\n`).join("");
  between(crateToml, BEGIN, END, deps);
  // A plugin depends on fast-harness-core by git; the patch makes that the
  // core of this checkout, so the plugin's `Ext` is the binaries' `Ext`.
  const patch = plugins.length
    ? `[patch."https://github.com/Releaseworthy/fast-harness"]\nfast-harness-core = { path = "crates/fast-harness-core" }\n`
    : "";
  between(join(root, "Cargo.toml"), "# BEGIN PLUGIN PATCH", "# END PLUGIN PATCH", patch);
  console.log(`plugins: ${plugins.map((p) => p.name).join(", ") || "(none)"}`);
}

main();
