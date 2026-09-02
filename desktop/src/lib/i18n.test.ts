// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The message catalogs against each other.
 *
 * en.json is the source of truth; every other locale must define exactly its
 * key set with matching placeholders. The catalogs are read from disk, not
 * imported, so a locale file someone dropped in but forgot to register still
 * gets caught. Same enforcement style as ipc-registry.test.ts: conventions
 * the type system cannot see, held by a test that names the offenders.
 */
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, test } from "vitest";

import { LOCALES } from "./i18n";

const dir = join(process.cwd(), "src", "locales");

function catalog(file: string): Record<string, string> {
  return JSON.parse(readFileSync(join(dir, file), "utf8")) as Record<string, string>;
}

/** The {name} placeholders of a message, as a sorted list. */
function placeholders(message: string): string[] {
  return (message.match(/\{\w+\}/g) ?? []).sort();
}

const files = readdirSync(dir).filter((entry) => entry.endsWith(".json"));
const en = catalog("en.json");
const enKeys = Object.keys(en);

describe("the message catalogs", () => {
  test("every locale file on disk is registered in LOCALES", () => {
    const registered = new Set<string>(LOCALES.map((entry) => entry.id));
    const unregistered = files.filter((file) => !registered.has(file.replace(/\.json$/, "")));
    expect(unregistered, "locale files LOCALES never registers").toEqual([]);
    const fileless = [...registered].filter((id) => !files.includes(`${id}.json`));
    expect(fileless, "registered locales with no file on disk").toEqual([]);
  });

  test("plural keys come in complete one/other pairs", () => {
    const widowed = enKeys.filter((key) => {
      if (key.endsWith(".one")) return !(key.replace(/\.one$/, ".other") in en);
      if (key.endsWith(".other")) return !(key.replace(/\.other$/, ".one") in en);
      return false;
    });
    expect(widowed, "plural keys missing their sibling form").toEqual([]);
  });

  test("en.json keys are sorted", () => {
    expect(enKeys, "keys out of alphabetical order").toEqual([...enKeys].sort());
  });

  for (const file of files) {
    const messages = catalog(file);

    describe(file, () => {
      test("defines every key en.json defines", () => {
        const missing = enKeys.filter((key) => !(key in messages));
        expect(missing, "keys en.json has that this locale lacks").toEqual([]);
      });

      test("defines no key en.json does not", () => {
        const stale = Object.keys(messages).filter((key) => !(key in en));
        expect(stale, "keys this locale has that en.json dropped").toEqual([]);
      });

      test("placeholders match en.json exactly", () => {
        const mismatched = enKeys.filter(
          (key) =>
            key in messages &&
            placeholders(messages[key]).join(" ") !== placeholders(en[key]).join(" "),
        );
        expect(mismatched, "keys whose {placeholders} differ from en.json").toEqual([]);
      });

      test("no value is empty", () => {
        const empty = Object.keys(messages).filter((key) => messages[key].trim() === "");
        expect(empty, "an empty string is a translation hole, not a translation").toEqual([]);
      });
    });
  }
});
