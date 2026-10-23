---
name: study-editors
description: Launch, drive, screenshot and learn from the established video editors installed on this Mac - CapCut and DaVinci Resolve - to benchmark a Concat feature, settle how something should look or behave, or study an effect before re-creating it clean-room. Use whenever a decision about Concat's UI, UX, editing behaviour, effects, export or audio would be better made by seeing how CapCut or Resolve does it, even if the user doesn't say "open CapCut".
---

# Studying CapCut and DaVinci Resolve

Concat is benchmarked against CapCut (the product goal) and, for pro behaviour, DaVinci Resolve. Both are installed here. Use them to answer "how should this work?" with evidence instead of guessing. Then write it down and build Concat's own version.

## What is installed

| App | Where | Notes |
|---|---|---|
| CapCut 9.4.0 | `/Applications/CapCut.app` (bundle `com.lemon.lvoverseas`) | Drafts: `~/Movies/CapCut/User Data/Projects/com.lveditor.draft/<draft>/` (`draft_info.json`, `Timelines/`, `Resources/`). Some files are encrypted (`crypto_key_store.dat`). |
| DaVinci Resolve 21.0.0 (free) | `/Applications/DaVinci Resolve/DaVinci Resolve.app` | Manual: `/Applications/DaVinci Resolve/DaVinci Resolve Manual.pdf` (read with `Read` + `pages`; no pdftotext). Scripting API: `/Library/Application Support/Blackmagic Design/DaVinci Resolve/Developer/Scripting/README.txt`. |

Re-check versions with `defaults read <app>/Contents/Info.plist CFBundleShortVersionString` before quoting them.

## Hard rules

1. **Clean room.** Study how they look and behave, then build Concat's own: our code, our own or CC0 assets, our own names. Never copy, convert, decrypt, bundle or load at runtime anything from either install (shaders, PNG sequences, Lua, models, LUTs, fonts, sounds, encrypted files), and never name a Concat effect after a CapCut title. Reading a draft's JSON to learn its structure is fine; shipping a converter that loads CapCut's own assets is not.
2. **Never capture the whole screen.** The user is working on the same Mac, and a full-screen grab captures their private windows. Capture only the editor's window, with `scripts/winshot.sh` below.
3. **Never touch the user's real projects.** Work in a new scratch project or a copied draft. Don't sign in, buy, upgrade, upload media to CapCut's cloud, or accept any account or telemetry prompt. If the app demands a login for a feature, note that as a finding and stop there.
4. **Leave it as you found it.** Quit the apps you launched when done (`osascript -e 'quit app "CapCut"'`). Delete scratch drafts and projects you created.

## Launch and see

```bash
open -a CapCut                       # or: open -a "DaVinci Resolve" (slow: ~30-60 s to the project manager)
.claude/skills/study-editors/scripts/winshot.sh CapCut /path/in/scratchpad/capcut.png
```

`winshot.sh <owner-substring> <out.png> [max-px]` finds the app's largest on-screen window by owner name and captures only it (`screencapture -l`), downscaled to 1600 px by default. Then `Read` the PNG. It works when the window is behind others. If it says "no window", the app is still launching or minimised: wait and retry.

## Drive it

In order of preference:

1. **Computer use**, if this session has it (load the `anthropic-skills:computer-use` skill first). It is the only way to click through panels reliably.
2. **Menus and keys via System Events** (needs Accessibility permission for the terminal; if denied, say so and fall back):
   ```bash
   osascript -e 'tell application "CapCut" to activate' \
     -e 'tell application "System Events" to keystroke "b" using command down'
   ```
   `activate` steals focus from the user, so warn them first and keep it short.
3. **Resolve's scripting API** for timeline/project facts: Python with `RESOLVE_SCRIPT_API`, `RESOLVE_SCRIPT_LIB` and `PYTHONPATH` set as the README says, `import DaVinciResolveScript as dvr; resolve = dvr.scriptapp("Resolve")`. The free edition may refuse external scripting: if `scriptapp` returns `None`, use Workspace › Console inside Resolve, or the UI.
4. **Ask the user** for the two or three clicks you can't make, saying exactly what to do and what to look at.

## Learn

- Read first, click second. CapCut's draft JSON shows its data model (tracks, segments, keyframes, transitions, material types). The Resolve manual explains intent and edge cases that clicking won't reveal.
- Measure, don't describe: timings (scrub latency, export speed on the same file), sizes (hit targets, panel widths in the window capture), defaults (bitrate, fades, easing), and what happens at edges (undo, overlap, trim past media end).
- Compare against Concat on the same input. The `run-concat` skill launches Concat. Capture both windows at the same step.

## Write it down

Put durable findings in `reports/<Topic> - CapCut and Resolve versus Concat.md` (see `reports/CapCut effects versus Concat effects.md` for the shape). Cover what each app does, the evidence (window captures in the scratchpad, draft fields, manual pages), what Concat does today (file:line), and what Concat should do, which can be better than either. One-off answers go straight to the user instead.
