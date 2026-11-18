// Keeps the app version the same in every file that carries it, and reads
// release notes out of CHANGELOG.md.
//
//   node scripts/version.mjs                 print the current version
//   node scripts/version.mjs --check         fail if the files disagree
//   node scripts/version.mjs --check v1.2.3  ...or disagree with a git tag
//   node scripts/version.mjs 1.2.3           set a new version everywhere and
//                                            date the Unreleased changelog
//   node scripts/version.mjs --notes 1.2.3   print that version's changelog
//
// package.json is the source of truth. tauri.conf.json points at it, and
// Cargo.toml / Cargo.lock / package-lock.json are rewritten to match.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const file = (p) => path.join(root, p);
const read = (p) => fs.readFileSync(file(p), "utf8");
const write = (p, s) => fs.writeFileSync(file(p), s);
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;
const CRATE = "agent-desk";

const fail = (msg) => {
  console.error(`error: ${msg}`);
  process.exit(1);
};

function versions() {
  const pkg = JSON.parse(read("package.json")).version;
  const lock = JSON.parse(read("package-lock.json"));
  const cargo = read("src-tauri/Cargo.toml").match(/^\[package\][^[]*?^version\s*=\s*"([^"]+)"/ms)?.[1];
  const cargoLock = read("src-tauri/Cargo.lock").match(
    new RegExp(`\\[\\[package\\]\\]\\r?\\nname = "${CRATE}"\\r?\\nversion = "([^"]+)"`),
  )?.[1];
  const tauri = JSON.parse(read("src-tauri/tauri.conf.json")).version;
  return {
    "package.json": pkg,
    "package-lock.json": lock.version,
    "package-lock.json (root package)": lock.packages?.[""]?.version,
    "src-tauri/Cargo.toml": cargo,
    "src-tauri/Cargo.lock": cargoLock,
    // tauri.conf.json may point at package.json instead of repeating it.
    "src-tauri/tauri.conf.json": tauri === "../package.json" ? pkg : tauri,
  };
}

function check(tag) {
  const found = versions();
  const expected = tag ? tag.replace(/^v/, "") : found["package.json"];
  const wrong = Object.entries(found).filter(([, v]) => v !== expected);
  if (wrong.length) {
    for (const [f, v] of wrong) console.error(`  ${f}: ${v ?? "(not found)"} (expected ${expected})`);
    fail(tag ? `version files do not match tag ${tag}.` : "version files disagree.");
  }
  console.log(`version ${expected} is consistent`);
}

function notes(version) {
  const v = version.replace(/^v/, "");
  const text = read("CHANGELOG.md").replace(/\r\n/g, "\n");
  const start = text.search(new RegExp(`^## \\[${v.replace(/\./g, "\\.")}\\]`, "m"));
  if (start < 0) return "";
  const rest = text.slice(start).split("\n").slice(1).join("\n");
  const end = rest.search(/^## /m);
  return (end < 0 ? rest : rest.slice(0, end)).replace(/^\[[^\]]+\]:.*$/gm, "").trim();
}

function bump(next) {
  if (!SEMVER.test(next)) fail(`"${next}" is not a version like 1.2.3 or 1.2.3-beta.1.`);
  const prev = versions()["package.json"];

  const pkg = JSON.parse(read("package.json"));
  pkg.version = next;
  write("package.json", JSON.stringify(pkg, null, 2) + "\n");

  const lock = JSON.parse(read("package-lock.json"));
  lock.version = next;
  if (lock.packages?.[""]) lock.packages[""].version = next;
  write("package-lock.json", JSON.stringify(lock, null, 2) + "\n");

  write(
    "src-tauri/Cargo.toml",
    read("src-tauri/Cargo.toml").replace(/^(\[package\][^[]*?^version\s*=\s*")[^"]+(")/ms, `$1${next}$2`),
  );
  write(
    "src-tauri/Cargo.lock",
    read("src-tauri/Cargo.lock").replace(
      new RegExp(`(\\[\\[package\\]\\]\\r?\\nname = "${CRATE}"\\r?\\nversion = ")[^"]+(")`),
      `$1${next}$2`,
    ),
  );

  const conf = JSON.parse(read("src-tauri/tauri.conf.json"));
  if (conf.version !== "../package.json") {
    conf.version = next;
    write("src-tauri/tauri.conf.json", JSON.stringify(conf, null, 2) + "\n");
  }

  // Date the Unreleased section and open a new empty one above it.
  let log = read("CHANGELOG.md");
  const d = new Date();
  const today = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  if (/^## \[Unreleased\]/m.test(log)) {
    log = log.replace(/^## \[Unreleased\]/m, `## [Unreleased]\n\n## [${next}] - ${today}`);
    const repo = repoUrl();
    log = log.replace(
      /^\[Unreleased\]:.*$/m,
      `[Unreleased]: ${repo}/compare/v${next}...HEAD\n[${next}]: ${repo}/releases/tag/v${next}`,
    );
    write("CHANGELOG.md", log);
  } else {
    console.warn("warning: CHANGELOG.md has no [Unreleased] section; left unchanged.");
  }

  check();
  console.log(`\n${prev} -> ${next}. Review CHANGELOG.md, then:\n`);
  console.log(`  git commit -am "Release v${next}"`);
  console.log(`  git tag v${next}`);
  console.log(`  git push && git push origin v${next}`);
}

export function repoUrl() {
  const repo = JSON.parse(read("package.json")).repository;
  const url = typeof repo === "string" ? repo : repo?.url;
  if (!url) fail("package.json has no repository url.");
  return url.replace(/^git\+/, "").replace(/\.git$/, "");
}

// Only run the command line when executed directly, not when imported.
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [cmd, arg] = process.argv.slice(2);
  if (!cmd) console.log(versions()["package.json"]);
  else if (cmd === "--check") check(arg);
  else if (cmd === "--notes") {
    if (!arg) fail("--notes needs a version.");
    process.stdout.write(notes(arg) + "\n");
  } else bump(cmd.replace(/^v/, ""));
}

export { notes, versions };
