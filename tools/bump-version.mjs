#!/usr/bin/env node
/**
 * tools/bump-version.mjs — unica fonte di verita' per il versionamento.
 *
 * Allinea la versione nei 3 manifest (package.json, src-tauri/tauri.conf.json,
 * src-tauri/Cargo.toml), ritocca la voce corrispondente in src-tauri/Cargo.lock
 * e apre la sezione nel CHANGELOG.md. Opzionalmente crea commit + tag e pusha.
 *
 * Uso:
 *   node tools/bump-version.mjs --patch [--tag] [--push]
 *   node tools/bump-version.mjs --minor [--tag] [--push]
 *   node tools/bump-version.mjs --major [--tag] [--push]
 *   node tools/bump-version.mjs 1.2.3 [--tag] [--push]
 *   node tools/bump-version.mjs 0.2.0-rc.1 --tag --push   # pre-release
 *   npm run release -- --minor                            # scorciatoia completa
 *
 * Flag:
 *   --tag       commit dei manifest + tag annotato vX.Y.Z
 *   --push      git push del branch e (se creato) del tag
 *   --dry-run   mostra cosa cambierebbe senza scrivere nulla
 */
import { readFileSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const FILES = {
  packageJson: join(ROOT, "package.json"),
  tauriConf: join(ROOT, "src-tauri", "tauri.conf.json"),
  cargoToml: join(ROOT, "src-tauri", "Cargo.toml"),
  cargoLock: join(ROOT, "src-tauri", "Cargo.lock"),
  changelog: join(ROOT, "CHANGELOG.md"),
};
const SEMVER = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/;

function fail(msg) {
  console.error(`bump-version: errore: ${msg}`);
  process.exit(1);
}

function parseArgs(argv) {
  const opts = { bump: null, version: null, tag: false, push: false, dryRun: false };
  for (const a of argv) {
    if (a === "--major" || a === "--minor" || a === "--patch") opts.bump = a.slice(2);
    else if (a === "--tag") opts.tag = true;
    else if (a === "--push") opts.push = true;
    else if (a === "--dry-run") opts.dryRun = true;
    else if (!a.startsWith("--") && !opts.version) opts.version = a.replace(/^v/, "");
    else fail(`argomento non riconosciuto: ${a}`);
  }
  if (opts.version && opts.bump) fail("specifica o una versione esatta o --major/--minor/--patch, non entrambi");
  if (!opts.version && !opts.bump) {
    fail("specifica una versione (es. 0.2.0) oppure --major/--minor/--patch");
  }
  return opts;
}

function bumpSemver(current, kind) {
  const m = current.match(/^(\d+)\.(\d+)\.(\d+)(-.+)?$/);
  if (!m) fail(`versione corrente non semver: ${current}`);
  let [major, minor, patch] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (kind === "major") { major += 1; minor = 0; patch = 0; }
  else if (kind === "minor") { minor += 1; patch = 0; }
  else { patch += 1; }
  return `${major}.${minor}.${patch}`;
}

function updateJson(path, version, dryRun) {
  const raw = readFileSync(path, "utf8");
  const data = JSON.parse(raw);
  const old = data.version;
  data.version = version;
  if (!dryRun) writeFileSync(path, JSON.stringify(data, null, 2) + "\n");
  return old;
}

function updateCargoToml(path, oldVersion, version, dryRun) {
  const raw = readFileSync(path, "utf8");
  const needle = `version = "${oldVersion}"`;
  if (!raw.includes(needle)) fail(`in Cargo.toml non trovo: ${needle}`);
  // Solo la prima occorrenza: quella di [package] (le dipendenze usano "1", "2", ...).
  const next = raw.replace(needle, `version = "${version}"`);
  if (!dryRun) writeFileSync(path, next);
}

function updateCargoLock(path, oldVersion, version, dryRun) {
  let raw;
  try {
    raw = readFileSync(path, "utf8");
  } catch {
    console.warn("bump-version: avviso: Cargo.lock assente, salto (cargo lo rigenera alla build)");
    return false;
  }
  const needle = `name = "dsh-desktop-manager"\nversion = "${oldVersion}"`;
  if (!raw.includes(needle)) {
    console.warn("bump-version: avviso: voce dsh-desktop-manager non trovata nel Cargo.lock (cargo lo riallinea alla build)");
    return false;
  }
  if (!dryRun) {
    writeFileSync(path, raw.replace(needle, `name = "dsh-desktop-manager"\nversion = "${version}"`));
  }
  return true;
}

function updateChangelog(path, version, dryRun) {
  let raw;
  try {
    raw = readFileSync(path, "utf8");
  } catch {
    console.warn("bump-version: avviso: CHANGELOG.md assente, salto");
    return false;
  }
  const today = new Date().toISOString().slice(0, 10);
  const marker = "## [Unreleased]";
  if (!raw.includes(marker)) {
    console.warn("bump-version: avviso: sezione [Unreleased] assente nel CHANGELOG, salto");
    return false;
  }
  if (!dryRun) {
    writeFileSync(path, raw.replace(marker, `${marker}\n\n## [${version}] - ${today}`));
  }
  return true;
}

const opts = parseArgs(process.argv.slice(2));
const pkg = JSON.parse(readFileSync(FILES.packageJson, "utf8"));
const current = pkg.version;
const next = opts.version ?? bumpSemver(current, opts.bump);
if (!SEMVER.test(next)) fail(`versione non valida (atteso x.y.z): ${next}`);
if (next === current) fail(`la versione e' gia' ${current}, nulla da fare`);

console.log(`bump-version: ${current} -> ${next}${opts.dryRun ? " (dry-run)" : ""}`);
updateJson(FILES.packageJson, next, opts.dryRun);
updateJson(FILES.tauriConf, next, opts.dryRun);
updateCargoToml(FILES.cargoToml, current, next, opts.dryRun);
updateCargoLock(FILES.cargoLock, current, next, opts.dryRun);
updateChangelog(FILES.changelog, next, opts.dryRun);
console.log("bump-version: manifest allineati (package.json, tauri.conf.json, Cargo.toml, Cargo.lock, CHANGELOG.md)");

if (opts.dryRun) process.exit(0);

if (opts.tag) {
  const tag = `v${next}`;
  const changed = ["package.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "CHANGELOG.md"];
  execSync(`git add ${changed.join(" ")}`, { cwd: ROOT, stdio: "inherit" });
  execSync(`git commit -m "chore(release): ${tag}"`, { cwd: ROOT, stdio: "inherit" });
  execSync(`git tag -a ${tag} -m "${tag}"`, { cwd: ROOT, stdio: "inherit" });
  console.log(`bump-version: commit + tag ${tag} creati`);
  if (opts.push) {
    execSync("git push", { cwd: ROOT, stdio: "inherit" });
    execSync(`git push origin ${tag}`, { cwd: ROOT, stdio: "inherit" });
    console.log(`bump-version: branch + tag ${tag} pushati (la Action Release parte dal tag)`);
  }
} else if (opts.push) {
  execSync("git push", { cwd: ROOT, stdio: "inherit" });
  console.log("bump-version: branch pushato (nessun tag creato: usa --tag per versionare una release)");
}
