// Verify that the release tag and every version in the repository agree:
// package.json, the workspace (core and CLI), src-tauri/Cargo.toml and the
// newest CHANGELOG heading (which must not say "unreleased" for a tag).
// Usage: node scripts/check-version.mjs [v1.2.3]
import { readFileSync } from "node:fs";

const pkg = JSON.parse(readFileSync("package.json", "utf8")).version;
const tomlVersion = (file) => /^version\s*=\s*"([^"]+)"/m.exec(readFileSync(file, "utf8"))?.[1];
const workspace = tomlVersion("Cargo.toml");
const app = tomlVersion("src-tauri/Cargo.toml");
const heading = /^## (\S+)(.*)$/m.exec(readFileSync("CHANGELOG.md", "utf8"));
// In GitHub Actions the ref is a tag only for releases; a branch name is no version.
const fromEnv = process.env.GITHUB_REF_TYPE === "tag" ? process.env.GITHUB_REF_NAME : undefined;
const tag = (process.argv[2] ?? fromEnv ?? "").replace(/^v/, "");

const problems = [];
if (pkg !== workspace) problems.push(`package.json ${pkg} != Cargo.toml (workspace) ${workspace}`);
if (pkg !== app) problems.push(`package.json ${pkg} != src-tauri/Cargo.toml ${app}`);
if (heading?.[1] !== pkg) problems.push(`CHANGELOG.md starts with ${heading?.[1]}, not ${pkg}`);
if (tag && tag !== pkg) problems.push(`tag ${tag} != package.json ${pkg}`);
if (tag && /unreleased/i.test(heading?.[2] ?? "")) problems.push(`CHANGELOG.md still calls ${pkg} unreleased`);
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log(`version ${pkg} OK`);
