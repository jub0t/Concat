# SOP: updating the README hero screenshot

The image at the top of `README.md` is `assets/editor-preview.png` — a single
frame that is the **dark theme on the left half** and the **light theme on the
right half**, joined at the vertical center line. Because both screenshots show
the identical window state, the seam falls invisibly through the middle of the
preview pane.

## Steps

1. **Run the app** — `cd desktop && npm run app`. Stage something presentable:
   a clip in the media bin, a couple of tracks on the timeline, the playhead
   somewhere interesting. Avoid personal file names in the media panel.

2. **Take the dark screenshot.** With the app in dark theme, press
   `⌘⇧4`, then `Space`, then click the Concat window (captures just the
   window). Don't move or resize the window after this.

3. **Switch to light theme** (the sun/moon toggle in the title bar) and take
   the second screenshot the same way. **Don't touch anything else** — same
   window size, same panels, same playhead. Any drift shows at the seam.

4. **Build the split:**

   ```sh
   scripts/make-editor-preview.sh ~/Desktop/Screenshot*<time1>*.png ~/Desktop/Screenshot*<time2>*.png
   ```

   First argument is the **dark** screenshot, second is the **light** one.
   Tab-complete or glob the paths — macOS puts a narrow no-break space before
   "AM"/"PM" in screenshot names, so a hand-typed regular space won't match.

   The script verifies both images are the same size, copies them to
   `assets/screenshot-dark.png` / `assets/screenshot-light.png`, and writes
   `assets/editor-preview.png`.

5. **Check the result** — open `assets/editor-preview.png` and look at the
   center seam. If panels are misaligned, retake both screenshots.

6. **Commit** the three changed files in `assets/`. The README references
   `assets/editor-preview.png` by path, so no README edit is needed.
