#!/usr/bin/env node
// Writes THIRD-PARTY-NOTICES.md, which the Manual shows from its About chapter
// and the CLI archives carry: every Rust crate built into the app or
// tower-bridge-cli for the shipped platforms, every npm
// package in the frontend bundle, and the fonts, icons and map data, each with
// its license text.
//
//   npm run notices               regenerate after a dependency change
//   npm run notices -- --check    fail when the file is out of date (CI)
//
// Where a component offers a choice of licenses, the notices name the one the
// app uses (MIT first, see PREFERENCE). License texts are copied verbatim from
// the packages, so the file is exempt from the repository's dash rule.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "THIRD-PARTY-NOTICES.md");
const STATIC = join(ROOT, "scripts/notices");
const PLATFORMS = [
  "x86_64-pc-windows-msvc",
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
];

/** The license used when a package offers several, best first. */
const PREFERENCE = [
  "MIT",
  "ISC",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "Zlib",
  "Apache-2.0",
  "Unicode-3.0",
  "BSL-1.0",
  "MPL-2.0",
  "0BSD",
  "MIT-0",
  "Unlicense",
  "CC0-1.0",
];

/** What a crate carries beyond its own license files. */
const EXTRA = {
  hidapi: {
    files: ["etc/hidapi/LICENSE-bsd.txt"],
    note: "Contains the HIDAPI C library, used under its BSD-style license.",
  },
  "libsqlite3-sys": { note: "Contains SQLite, which is in the public domain." },
};

const MIT_TEXT = `Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;

// ---------------------------------------------------------------------------
// License expressions

/** Parses an SPDX expression (also the old `MIT/Apache-2.0` form) into a tree. */
function parse(expression) {
  const tokens = expression
    .replace(/\s*\/\s*/g, " OR ")
    .replace(/([()])/g, " $1 ")
    .split(/\s+/)
    .filter(Boolean);
  let i = 0;
  const factor = () => {
    if (tokens[i] === "(") {
      i++;
      const node = or();
      i++; // ")"
      return node;
    }
    let id = tokens[i++];
    if (tokens[i] === "WITH") {
      id += ` WITH ${tokens[i + 1]}`;
      i += 2;
    }
    return { id };
  };
  const and = () => {
    const all = [factor()];
    while (tokens[i] === "AND") (i++, all.push(factor()));
    return all.length === 1 ? all[0] : { and: all };
  };
  const or = () => {
    const any = [and()];
    while (tokens[i] === "OR") (i++, any.push(and()));
    return any.length === 1 ? any[0] : { or: any };
  };
  return or();
}

const rank = (id) => (PREFERENCE.includes(id) ? PREFERENCE.indexOf(id) : PREFERENCE.length);

/** The licenses to comply with: every part of an AND, the best choice of an OR. */
function choose(node) {
  if (node.id) return [node.id];
  if (node.and) return [...new Set(node.and.flatMap(choose))];
  const options = node.or.map(choose);
  const cost = (ids) => ids.reduce((sum, id) => sum + rank(id) + 1, 0);
  return options.reduce((best, o) => (cost(o) < cost(best) ? o : best));
}

/** A pattern for a phrase, whatever the line breaks in the text. */
const phrase = (text) => new RegExp(text.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&").replace(/ /g, "\\s+"), "i");

const WORDING = [
  ["Apache-2.0", [phrase("Apache License"), /TERMS AND CONDITIONS/]],
  ["MIT", [phrase("Permission is hereby granted, free of charge")]],
  ["ISC", [/Permission\s+to\s+use,\s+copy,\s+modify,\s+and(\/or)?\s+distribute\s+this\s+software\s+for\s+any/i]],
  [
    "BSD-3-Clause",
    [
      phrase("Redistribution and use in source and binary forms"),
      /Neither\s+the\s+name|names\s+of\s+(its|the)\s+contributors|name\s+of\s+the\s+copyright\s+holder/i,
    ],
  ],
  ["Zlib", [/provided\s+['\u2018]as-is['\u2019],\s+without\s+any\s+express\s+or\s+implied/i]],
  ["MPL-2.0", [phrase("Mozilla Public License Version 2.0")]],
  ["BSL-1.0", [phrase("Boost Software License")]],
  ["Unlicense", [phrase("free and unencumbered software released into the public domain")]],
];

/** The licenses a text contains, from its wording (a file may combine several). */
function kindsOf(text) {
  // The Unicode license repeats the MIT wording; it is one license, not two.
  if (/UNICODE LICENSE V3|UNICODE, INC\. LICENSE AGREEMENT/i.test(text)) return ["Unicode-3.0"];
  const kinds = WORDING.filter(([, tests]) => tests.every((t) => t.test(text))).map(([id]) => id);
  if (kinds.includes("BSD-3-Clause")) return kinds;
  if (phrase("Redistribution and use in source and binary forms").test(text)) kinds.push("BSD-2-Clause");
  return kinds;
}

const LICENSE_FILE = /^(licen[cs]e|copying|copyright|notice)/i;

function licenseFiles(dir) {
  return readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f) && statSync(join(dir, f)).isFile())
    .sort()
    .map((f) => ({ name: f, text: readFileSync(join(dir, f), "utf8").replace(/\r\n/g, "\n").trim() }));
}

/**
 * The texts that go into the notices for a package: for each license used,
 * its file in the package (named after it if there are several); NOTICE files
 * always. A license without a file gets the standard MIT text with the
 * authors, or a pointer to the standard text below.
 */
function texts(name, dir, licenses, authors, extra) {
  const files = licenseFiles(dir);
  const picked = new Set();
  const missing = [];
  for (const id of licenses) {
    // A file of this license alone first, then one named after it, then a combined one.
    const matching = files
      .filter((f) => kindsOf(f.text).includes(id))
      .sort((a, b) => kindsOf(a.text).length - kindsOf(b.text).length);
    const named = matching.filter((f) => f.name.toUpperCase().includes(id.split("-")[0].toUpperCase()));
    const file = matching.find((f) => kindsOf(f.text).length === 1 && named.includes(f)) ?? matching[0];
    if (file) picked.add(file);
    else missing.push(id);
  }
  // A combined file the wording test cannot place (several licenses in one).
  if (missing.length) for (const f of files) if (!kindsOf(f.text).length && !/^notice/i.test(f.name)) picked.add(f);
  for (const f of files) if (/^notice/i.test(f.name)) picked.add(f);
  const out = [...picked].map((f) => f.text);
  for (const rel of extra?.files ?? []) out.push(readFileSync(join(dir, rel), "utf8").replace(/\r\n/g, "\n").trim());
  // Filled in by `standard()` once every package's texts are known.
  if (missing.length && !out.length)
    for (const id of missing) out.push({ standard: id, holder: authors || `the ${name} authors` });
  return out;
}

/** Licenses whose text names the copyright holder. */
const HOLDER = ["MIT", "ISC", "BSD-2-Clause", "BSD-3-Clause", "Zlib"];

/** A license text without the copyright lines at its top. */
const body = (text) => text.replace(/^(\s*(copyright|\(c\)|all rights reserved)[^\n]*\n)+\s*/i, "");

/**
 * A package without a license file gets the standard text: with its authors
 * as the holder where the license names one (taken from another package's
 * file of that license), otherwise a pointer to "Standard license texts".
 * Returns the licenses that need the pointer.
 */
function standard(packages) {
  const alone = new Map();
  for (const t of packages.flatMap((p) => p.texts))
    if (typeof t === "string" && kindsOf(t).length === 1 && !alone.has(kindsOf(t)[0])) alone.set(kindsOf(t)[0], t);
  const referenced = new Map();
  for (const p of packages)
    p.texts = p.texts.map((t) => {
      if (typeof t === "string") return t;
      if (t.standard === "MIT") return `Copyright (c) ${t.holder}\n\n${MIT_TEXT}`;
      if (HOLDER.includes(t.standard) && alone.has(t.standard))
        return `Copyright (c) ${t.holder}\n\n${body(alone.get(t.standard))}`;
      if (!alone.has(t.standard)) throw new Error(`${p.name}: no ${t.standard} text to refer to`);
      referenced.set(t.standard, alone.get(t.standard));
      return `The package carries no license file; the standard ${t.standard} text applies (see "Standard license texts").`;
    });
  return referenced;
}

// ---------------------------------------------------------------------------
// Rust crates

function crates() {
  const meta = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked", ...PLATFORMS.flatMap((p) => ["--filter-platform", p])],
      { cwd: ROOT, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
    ),
  );
  const packages = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  // The desktop app and tower-bridge-cli, whose archives carry this file too.
  const roots = meta.packages.filter((p) => ["tower-bridge", "tower-bridge-cli"].includes(p.name) && p.source === null);
  // Normal dependencies only: build and dev dependencies do not end up in the app.
  const seen = new Set();
  const stack = roots.map((r) => r.id);
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const d of nodes.get(id).deps) if (d.dep_kinds.some((k) => k.kind === null)) stack.push(d.pkg);
  }
  return [...seen]
    .map((id) => packages.get(id))
    .filter((p) => p.source !== null)
    .map((p) => {
      const dir = dirname(p.manifest_path);
      const expression = p.license ?? "see the license text";
      const licenses = p.license ? choose(parse(p.license)) : [];
      const extra = EXTRA[p.name];
      const own = p.license_file ? [readFileSync(join(dir, p.license_file), "utf8").trim()] : [];
      return {
        name: p.name,
        version: p.version,
        url: p.repository ?? p.homepage ?? `https://crates.io/crates/${p.name}`,
        expression,
        licenses,
        note: extra?.note,
        texts: [...own, ...texts(p.name, dir, licenses, p.authors.join(", "), extra)],
      };
    });
}

// ---------------------------------------------------------------------------
// npm packages

/** Packages whose code is in the frontend bundle, from the bundle's module ids. */
async function bundled() {
  const { build } = await import("vite");
  const output = await build({ root: ROOT, logLevel: "silent", build: { write: false } });
  const ids = [output]
    .flat()
    .flatMap((o) => o.output)
    .flatMap((chunk) => chunk.moduleIds ?? []);
  const names = new Set();
  for (const id of ids) {
    const m = /\/node_modules\/((?:@[^/]+\/)?[^/]+)\//.exec(id);
    if (m) names.add(m[1]);
    else if (id.startsWith("\0vite/")) names.add("vite"); // the preload helper
  }
  return [...names].map((name) => {
    const dir = join(ROOT, "node_modules", name);
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const expression = pkg.license ?? "see the license text";
    const licenses = pkg.license ? choose(parse(pkg.license)) : [];
    const repo = typeof pkg.repository === "string" ? pkg.repository : pkg.repository?.url;
    const author = typeof pkg.author === "string" ? pkg.author : (pkg.author?.name ?? "");
    return {
      name,
      version: pkg.version,
      url: (repo ?? `https://www.npmjs.com/package/${name}`).replace(/^git\+/, "").replace(/\.git$/, ""),
      expression,
      licenses,
      texts: texts(name, dir, licenses, author),
    };
  });
}

// ---------------------------------------------------------------------------
// Fonts, icons, map data

const read = (path) => readFileSync(join(ROOT, path), "utf8").replace(/\r\n/g, "\n").trim();

function assets() {
  return [
    {
      name: "Inter",
      what: "the app's text font",
      url: "https://github.com/rsms/inter",
      expression: "OFL-1.1",
      texts: [read("src/assets/fonts/OFL-Inter.txt")],
    },
    {
      name: "Roboto Mono",
      what: "the font of addresses and code",
      url: "https://github.com/googlefonts/robotomono",
      expression: "OFL-1.1",
      texts: [read("src/assets/fonts/OFL-RobotoMono.txt")],
    },
    {
      name: "Barlow Condensed Bold",
      what: "the digits of the tray icon on Windows",
      url: "https://github.com/jpt/barlow",
      expression: "OFL-1.1",
      texts: [read("src-tauri/assets/fonts/OFL.txt")],
    },
    {
      name: "Lucide",
      what: "icons",
      url: "https://lucide.dev",
      expression: "ISC, partly MIT (Feather)",
      texts: [read("scripts/notices/lucide.txt")],
    },
    {
      name: "world-atlas 2.0.2",
      what: "the country outlines of the GPS map, from Natural Earth",
      url: "https://github.com/topojson/world-atlas",
      expression: "ISC",
      texts: [read("scripts/notices/world-atlas.txt")],
    },
    {
      name: "Libraries in the Linux AppImage",
      what: "WebKitGTK, GTK, GLib and the libraries they need, bundled from Ubuntu 22.04",
      url: "https://packages.ubuntu.com/jammy/",
      expression: "LGPL-2.1-or-later and others",
      texts: [
        "The Linux AppImage contains shared libraries of Ubuntu 22.04 (jammy), among them WebKitGTK\n(libwebkit2gtk-4.1-0), GTK (libgtk-3-0), GLib (libglib2.0-0), libsoup and librsvg. Each is used\nunder its own license, most under the GNU Lesser General Public License 2.1 or later. Their\nlicense texts and source code are in the Ubuntu packages of the same names, at\nhttps://packages.ubuntu.com/jammy/ and https://launchpad.net/ubuntu/jammy.",
      ],
    },
    {
      name: "Natural Earth 5.1.2",
      what: "the lakes, rivers and towns of the GPS map",
      url: "https://www.naturalearthdata.com",
      expression: "Public domain",
      texts: [
        "Made with Natural Earth. Free vector and raster map data @ naturalearthdata.com.\nAll Natural Earth data is in the public domain.",
      ],
    },
    {
      name: "Wikidata",
      what: "the Czech and Slovak town names of the GPS map",
      url: "https://www.wikidata.org",
      expression: "CC0-1.0",
      texts: [
        "The structured data of Wikidata is available under the Creative Commons CC0 1.0 Universal Public Domain Dedication.",
      ],
    },
  ];
}

// ---------------------------------------------------------------------------
// Output

/** By code points: `localeCompare` follows the locale (Czech sorts "ch" after "h"), so the file would differ per machine. */
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/** A fence longer than any run of backticks in the text. */
function fenced(text) {
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  const fence = "`".repeat(longest + 1);
  return `${fence}text\n${text}\n${fence}`;
}

/** Package metadata is third-party text too: it must not turn into HTML. */
const esc = (text) => String(text).replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);

const label = (p) => esc(p.licenses.length ? p.licenses.join(" AND ") : p.expression);

/** Packages with the same texts share one block, so each text appears once. */
function grouped(list) {
  const groups = new Map();
  for (const p of list.sort((a, b) => cmp(a.name, b.name) || cmp(a.version, b.version))) {
    const key = p.texts.map((t) => t.replace(/\s+/g, " ")).join("\n--\n");
    if (!groups.has(key)) groups.set(key, { license: label(p), texts: p.texts, members: [] });
    groups.get(key).members.push(p);
  }
  return [...groups.values()]
    .sort(
      (a, b) =>
        rank(a.license) - rank(b.license) || cmp(a.license, b.license) || cmp(a.members[0].name, b.members[0].name),
    )
    .map((g) => {
      const names = esc(g.members.map((p) => `${p.name} ${p.version}`).join(", "));
      const notes = g.members
        .filter((p) => p.note)
        .map((p) => `${p.name}: ${p.note}\n\n`)
        .join("");
      return `<details>\n<summary>${g.license}: ${names}</summary>\n\n${notes}${g.texts.map(fenced).join("\n\n")}\n\n</details>`;
    })
    .join("\n\n");
}

function table(list) {
  return list
    .sort((a, b) => cmp(a.name, b.name) || cmp(a.version, b.version))
    .map((p) => {
      const used = p.licenses.length && p.licenses.join(" AND ") !== p.expression ? `, used under ${label(p)}` : "";
      // The address as text: the source of every component is named (MPL-2.0 asks for it).
      return `- ${esc(p.name)} ${esc(p.version)}: ${esc(p.expression)}${used}, ${esc(p.url)}`;
    })
    .join("\n");
}

/** One copy of each standard text that a package without a license file refers to. */
function standardTexts(referenced) {
  if (!referenced.size) return "";
  const out = [...referenced]
    .sort(([a], [b]) => cmp(a, b))
    .map(([id, text]) => `<details>\n<summary>${id}</summary>\n\n${fenced(text)}\n\n</details>`);
  return `\n\n## Standard license texts\n\n${out.join("\n\n")}`;
}

const rust = crates();
const npm = await bundled();
const referenced = standard([...rust, ...npm]);
const doc = `# Third-party notices

TOWER Bridge and tower-bridge-cli contain the following software and data by others, each used under the license named beside it. Where a component offers a choice of licenses, the one used is named. The license texts follow each list.

This file is generated by \`npm run notices\`; do not edit it by hand.

## Fonts, icons and map data

${assets()
  .map((a) => `- ${a.name}, ${a.what}: ${a.expression}, ${a.url}`)
  .join("\n")}

${assets()
  .map(
    (a) =>
      `<details>\n<summary>${a.name}: ${a.expression}</summary>\n\n${a.texts.map(fenced).join("\n\n")}\n\n</details>`,
  )
  .join("\n\n")}

## JavaScript packages (${npm.length})

${table(npm)}

${grouped(npm)}

## Rust crates (${rust.length})

${table(rust)}

${grouped(rust)}${standardTexts(referenced)}
`;

if (process.argv.includes("--check")) {
  const current = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
  if (current !== doc) {
    console.error("THIRD-PARTY-NOTICES.md is out of date; run `npm run notices`.");
    process.exit(1);
  }
  console.log("THIRD-PARTY-NOTICES.md is up to date.");
} else {
  writeFileSync(OUT, doc);
  console.log(`Wrote ${OUT}: ${rust.length} crates, ${npm.length} npm packages, ${(doc.length / 1024).toFixed(0)} KB.`);
}
