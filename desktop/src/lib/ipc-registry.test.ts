// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The IPC boundary's two halves against each other.
 *
 * The boundary is JSON, so nothing at compile time checks that a wrapper in
 * `lib/engine.ts` names a command the host actually registered, or that every
 * registered command has a wrapper. This test closes the gap the desktop
 * README used to flag ("nothing checks steps 1 and 3 against each other"),
 * and enforces the standing rule that `lib/engine.ts` is the only file that
 * calls `invoke` at all.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, test } from "vitest";

const root = process.cwd();

function read(path: string): string {
  return readFileSync(join(root, path), "utf8");
}

/** Command names the host registers, from the generate_handler! block. */
function registeredCommands(): Set<string> {
  const source = read("src-tauri/src/lib.rs");
  const block = source.match(/generate_handler!\[([\s\S]*?)\]/);
  expect(block, "generate_handler! block in src-tauri/src/lib.rs").toBeTruthy();
  const names = block![1]
    .split(",")
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0)
    // `editor_api::editor_open` registers as `editor_open`.
    .map((entry) => entry.split("::").pop()!);
  return new Set(names);
}

/** Command names the frontend invokes, from lib/engine.ts. */
function invokedCommands(): Set<string> {
  const source = read("src/lib/engine.ts");
  const names = new Set<string>();
  for (const match of source.matchAll(/invoke(?:<[^>]*>)?\(\s*"([a-z_]+)"/g)) {
    names.add(match[1]);
  }
  return names;
}

function walk(directory: string, out: string[] = []): string[] {
  for (const entry of readdirSync(join(root, directory))) {
    const path = join(directory, entry);
    if (statSync(join(root, path)).isDirectory()) walk(path, out);
    else if (/\.(ts|tsx)$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(path);
  }
  return out;
}

describe("the IPC registry", () => {
  test("every wrapper names a registered command", () => {
    const registered = registeredCommands();
    const unregistered = [...invokedCommands()].filter((name) => !registered.has(name));
    expect(unregistered, "wrappers invoking commands the host never registered").toEqual([]);
  });

  test("every registered command has a wrapper", () => {
    const invoked = invokedCommands();
    const orphaned = [...registeredCommands()].filter((name) => !invoked.has(name));
    expect(orphaned, "registered commands nothing in the UI can call").toEqual([]);
  });

  test("lib/engine.ts is the only file that calls invoke", () => {
    const offenders = walk("src").filter(
      (path) =>
        path !== join("src", "lib", "engine.ts") &&
        /\binvoke\s*[(<]/.test(read(path)),
    );
    expect(offenders, "files calling invoke outside the typed boundary").toEqual([]);
  });
});
