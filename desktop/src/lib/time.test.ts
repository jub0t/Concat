// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * Time as the reader sees it.
 *
 * These helpers only format - the arithmetic lives in the engine - but they
 * are on screen constantly: the monitor's timecode, the bin's durations, the
 * recents list. A wrong digit here misreports the edit everywhere at once.
 */
import { afterEach, describe, expect, test, vi } from "vitest";

import { getLocale } from "./i18n";
import { relativeTime, shortDuration, timecode } from "./time";

describe("timecode", () => {
  test("zero is all zeros", () => {
    expect(timecode(0, 30)).toBe("00:00:00:00");
  });

  test("frames count within the second at the given rate", () => {
    expect(timecode(1.5, 30)).toBe("00:00:01:15");
    expect(timecode(1.5, 24)).toBe("00:00:01:12");
    expect(timecode(2.96, 25)).toBe("00:00:02:24"); // the second's last frame
  });

  test("hours, minutes and seconds carry like a clock", () => {
    expect(timecode(59, 30)).toBe("00:00:59:00");
    expect(timecode(60, 30)).toBe("00:01:00:00");
    expect(timecode(3599, 30)).toBe("00:59:59:00");
    expect(timecode(3600, 30)).toBe("01:00:00:00");
    expect(timecode(3661.5, 30)).toBe("01:01:01:15");
  });

  test("the frame counter never reaches the rate", () => {
    // 0.999s at 30fps is frame 29, not a rollover into :30.
    expect(timecode(0.999, 30)).toBe("00:00:00:29");
    expect(timecode(1, 30)).toBe("00:00:01:00");
  });

  test("negative time clamps to zero rather than going surreal", () => {
    expect(timecode(-5, 30)).toBe("00:00:00:00");
  });

  test("fractional rates keep frames and seconds in one clock", () => {
    // Non-drop-frame: both fields derive from the frame count. 100s at
    // 29.97 is frame 2997 exactly - second 99, frame 27 - and the frame
    // field must roll to :00 exactly when its own seconds increment, never
    // disagree with them (the old wall-clock seconds read 01:40 here).
    expect(timecode(100, 29.97)).toBe("00:01:39:27");
    expect(timecode(100.101, 29.97)).toBe("00:01:40:00");
  });
});

describe("shortDuration", () => {
  test("unknown duration reads as placeholder dashes", () => {
    expect(shortDuration(null)).toBe("--:--");
  });

  test("whole seconds, floor not round", () => {
    expect(shortDuration(0)).toBe("0:00");
    expect(shortDuration(59.9)).toBe("0:59");
    expect(shortDuration(61)).toBe("1:01");
  });

  test("minutes keep counting past an hour - no third field appears", () => {
    expect(shortDuration(3600)).toBe("60:00");
    expect(shortDuration(3725)).toBe("62:05");
  });

  test("negative clamps to zero", () => {
    expect(shortDuration(-3)).toBe("0:00");
  });
});

describe("relativeTime", () => {
  const NOW = new Date("2026-08-28T12:00:00Z").getTime();
  const ago = (seconds: number) => relativeTime(NOW - seconds * 1000);

  afterEach(() => {
    vi.useRealTimers();
  });

  const freeze = () => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  };

  test("under ninety seconds is just now", () => {
    freeze();
    expect(ago(0)).toBe("just now");
    expect(ago(89)).toBe("just now");
  });

  test("minutes until the hour", () => {
    freeze();
    expect(ago(90)).toBe("2 min ago"); // 1.5 min rounds up
    expect(ago(10 * 60)).toBe("10 min ago");
    expect(ago(3599)).toBe("60 min ago");
  });

  test("hours until a day, singular reads as singular", () => {
    freeze();
    expect(ago(3600)).toBe("1 hour ago");
    expect(ago(2 * 3600)).toBe("2 hours ago");
  });

  test("days until a week, one day ago reads as yesterday", () => {
    freeze();
    expect(ago(86_400)).toBe("yesterday");
    expect(ago(3 * 86_400)).toBe("3 days ago");
  });

  test("a week or more becomes a real date in the reader's locale", () => {
    freeze();
    const timestamp = NOW - 30 * 86_400 * 1000;
    expect(relativeTime(timestamp)).toBe(
      new Date(timestamp).toLocaleDateString(getLocale(), {
        day: "numeric",
        month: "short",
        year: "numeric",
      }),
    );
  });

  test("a timestamp from the future clamps to just now", () => {
    // Clock skew between machines must not render "-3 min ago".
    freeze();
    expect(relativeTime(NOW + 60_000)).toBe("just now");
  });
});
