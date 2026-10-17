// Verify that the release tag, package.json and src-tauri/Cargo.toml agree.
// Usage: node scripts/check-version.mjs [v1.2.3]
import { readFileSync } from "node:fs";

const pkg = JSON.parse(readFileSync("package.json", "utf8")).version;
const cargo = /^version\s*=\s*"([^"]+)"/m.exec(readFileSync("src-tauri/Cargo.toml", "utf8"))?.[1];
const tag = (process.argv[2] ?? process.env.GITHUB_REF_NAME ?? "").replace(/^v/, "");

const problems = [];
if (pkg !== cargo) problems.push(`package.json ${pkg} != src-tauri/Cargo.toml ${cargo}`);
if (tag && tag !== pkg) problems.push(`tag ${tag} != package.json ${pkg}`);
if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log(`version ${pkg} OK`);
