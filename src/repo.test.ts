/// <reference types="node" />
import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

/** Files in the repository: tracked plus new ones that are not ignored. */
function repoFiles(): string[] {
  try {
    return execSync("git ls-files --cached --others --exclude-standard", {
      encoding: "utf8",
    })
      .split("\n")
      .filter(Boolean);
  } catch {
    return [];
  }
}

const TEXT = /\.(rs|ts|js|mjs|svelte|html|css|md|json|toml|ya?ml|svg|sh|rules|plist)$/;

describe("repository", () => {
  // Long dashes read as machine written; use a comma, a semicolon, a colon or two sentences.
  it("has no em or en dashes", () => {
    const offenders: string[] = [];
    // The notices quote third-party license texts verbatim.
    const exempt = (f: string) => f.endsWith("package-lock.json") || f === "THIRD-PARTY-NOTICES.md";
    for (const file of repoFiles().filter((f) => TEXT.test(f) && !exempt(f))) {
      let text: string;
      try {
        text = readFileSync(file, "utf8");
      } catch {
        continue; // deleted but not yet staged
      }
      text.split("\n").forEach((line, i) => {
        if (/[\u2013\u2014]/.test(line)) offenders.push(`${file}:${i + 1}: ${line.trim()}`);
      });
    }
    expect(offenders).toEqual([]);
  });
});
