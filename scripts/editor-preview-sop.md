# SOP: updating the README hero image

The image at the top of `README.md` is `assets/editor.png`: the dark-theme
editor window floating on a soft gradient card, built by
`scripts/make-preview.py` from the raw capture in `assets/editor-dark.png`.

## Steps

1. **Run the app** — `cd engine && cargo run --release -p concat`. Stage something
   presentable: clips in the media bin, a few tracks on the timeline, the
   playhead somewhere interesting. Avoid personal file names in the media
   panel. Make the window wide (the composite assumes a landscape window).

2. **Capture the window.** In dark theme, press `⌘⇧4`, then `Space`, then
   click the Concat window. macOS saves the window with a transparent margin
   and its drop shadow; the script trims that away itself.

3. **Build the composite:**

   ```sh
   cp ~/Desktop/Screenshot*<time>*.png assets/editor-dark.png
   scripts/make-preview.py
   ```

   Tab-complete or glob the screenshot path: macOS puts a narrow no-break
   space before "AM"/"PM" in the file name. The script needs Pillow
   (`pip install pillow`). Pass explicit paths to build from somewhere else:
   `scripts/make-preview.py in.png out.png`.

4. **Check the result** — open `assets/editor.png`. The window should sit
   centred with the timeline running off the bottom edge. Backdrop colours,
   corner radii, and the crop height are constants at the top of the script.

5. **Commit** `assets/editor-dark.png` and `assets/editor.png`. The
   README loads `assets/editor.png` through jsDelivr rather than GitHub's
   raw host, which drops requests from some regions. No README edit is
   needed, but jsDelivr caches `@main` for up to a day, so purge it after
   pushing:

   ```sh
   curl https://purge.jsdelivr.net/gh/jub0t/Concat@main/assets/editor.png
   ```
