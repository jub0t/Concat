// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * Font identity from a file path.
 *
 * `familyForPath` names the CSS family a custom font registers under, and that
 * name is written into the project's font list and referenced by text clips.
 * Two files that end up with one name would silently render as each other -
 * exactly the collision the uniquing exists to prevent.
 */
import { describe, expect, test } from "vitest";

import { familyForPath } from "./text";

describe("familyForPath", () => {
  test("the file's own name, without its extension", () => {
    expect(familyForPath("/fonts/Inter-Bold.otf", [])).toBe("Inter-Bold");
    expect(familyForPath("/fonts/Cabinet Grotesk.TTF", [])).toBe("Cabinet Grotesk");
  });

  test("windows paths split on backslashes too", () => {
    expect(familyForPath("C:\\Fonts\\Regular.otf", [])).toBe("Regular");
  });

  test("a taken name gets a counter, and the counter keeps counting", () => {
    expect(familyForPath("/a/Regular.otf", ["Regular"])).toBe("Regular 2");
    expect(familyForPath("/b/Regular.otf", ["Regular", "Regular 2"])).toBe("Regular 3");
  });

  test("punctuation CSS would choke on is stripped", () => {
    expect(familyForPath("/f/Font™.otf", [])).toBe("Font");
    // Inner dots go with the punctuation; only the last extension was an
    // extension.
    expect(familyForPath("/f/My.Font.v2.otf", [])).toBe("MyFontv2");
  });

  test("a name with nothing usable left falls back, and still uniques", () => {
    expect(familyForPath("/f/★.otf", [])).toBe("Custom font");
    expect(familyForPath("/f/★.otf", ["Custom font"])).toBe("Custom font 2");
  });
});
