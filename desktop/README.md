<p align="center">
  <img src="../assets/concat_logo_512.png" alt="Concat logo" width="160" />
</p>

# Concat Desktop

The editor front end. Tauri v2 + React 19 + Tailwind v4, talking to the Rust
engine in [`../engine`](../engine).

## Run it

```sh
npm install
npm run app          # tauri dev - builds the Rust host and opens the window
npm run app:build    # production bundle (msi/nsis on Windows)
npm run typecheck    # tsc --noEmit
npm test             # vitest - the engine-mirror and IPC-registry suites
```

`npm run dev` alone serves the UI in a browser, which is useful for pure layout
work - but every `invoke` fails there, because there is no Rust host behind it.

## Layout

```
src/
  App.tsx              editor shell: panel grid, window state, keyboard map
  components/
    TitleBar.tsx       custom title bar - the window is undecorated
    StartScreen.tsx    launch screen: new project, recents, template gallery
    Icon.tsx           the whole icon set, plus the standard IconButton
    Panel.tsx          Panel / Bar / Divider / Empty - all shared chrome
    Menu.tsx           dropdown menus (title bar)
    ContextMenu.tsx    right-click menus (portal, edge-clamped)
    Resizer.tsx        draggable panel dividers
    MediaBin.tsx       the library: media, text, transitions, effects, templates
    Preview.tsx        program monitor: live approximation + the engine's true still
    TimelinePanel.tsx  toolbar, track headers, and the timeline canvas
    RightPanel.tsx     the selection's tab strip, hosting the panels below
    Inspector.tsx      properties of the selection (read-only)
    AdjustPanel.tsx    volume, fades, speed, transform
    FiltersPanel.tsx   the clip's audio filter chain
    EffectsPanel.tsx   the clip's video effect chain and incoming transition
    TextPanel.tsx      title styling and custom fonts
    ExportDialog.tsx   render settings, progress, title rasterisation
    TtsDialog.tsx      text to speech: narration in, a clip at the playhead out
    SettingsDialog.tsx transcriber and speech model setup
    SaveTemplateDialog.tsx  names and saves the open project as a template
    TemplateThumb.tsx  a template's poster, shared by gallery and library
    ConfirmDialog.tsx  the are-you-sure sheet for destructive actions
    ErrorNotice.tsx    inline error strip
    controls.tsx       sliders and fields shared by the panels
  lib/
    engine.ts          the ONLY file that calls invoke(). Typed IPC boundary.
    editor.ts          type mirrors of the engine model, selectors, gesture echo
    transport.ts       the playback clock (follows the engine's position events)
    effects.ts         video effect & transition catalogue -> ffmpeg chains + live previews
    filters.ts         audio filter catalogue -> ffmpeg chains
    text.ts            title styles and bundled fonts
    assets.ts          filmstrips and waveforms, cached in the project folder
    rasterize.ts       titles -> full-frame PNGs at export time
    theme.ts           light / dark, and the canvas palette reads
    time.ts            display formatting. No time arithmetic lives in the UI.
    settings.ts        localStorage preferences
  styles.css           Tailwind entry point, tokens, and the `surface` utility
src-tauri/
  src/lib.rs           Tauri commands: engine types in, JSON out. Nothing else.
  src/editor_api.rs    the engine-owned editing session (open / apply / undo / save)
  src/export.rs        transition resolution, timeline build, render, preview_frame
  src/playback.rs      audio decode, mix, and the transport clock
  src/projects.rs      project folders on disk and the recents list
  src/templates.rs     template bundles: save, list, instantiate, delete
  src/transcribe.rs    whisper-cli discovery, models, transcription
  src/tts.rs           Kokoro text to speech: model downloads, synthesis
```

## Rules that keep this fast

- **`lib/engine.ts` is the only file that calls `invoke`.** One place to fix
  when a command changes, and the compiler finds every call site.
- **The timeline is not DOM.** One canvas, one rAF loop. See decision 0002.
- **No time arithmetic in TypeScript.** The engine works in exact rational
  seconds; JS numbers are `f64` and drift. `lib/time.ts` formats, nothing more.
- **The engine owns the edit.** Every mutation is an `editor_apply` command
  into `wolfcut-project` (engine decision 0007); the UI renders the state that
  comes back and holds only a transient gesture echo while a drag is in
  flight. Undo, serialisation and the operations themselves live in Rust -
  do not reintroduce model logic in TypeScript.

## Keyboard

| Key | Action |
|---|---|
| `Space` | Play / pause |
| `←` `→` | Step one frame (`Shift` for ten) |
| `Home` `End` | Jump to start / end |
| `V` `C` | Select tool / razor tool |
| `S` / `Ctrl`+`B` | Split at playhead |
| `M` | Merge selected clips |
| `N` | Toggle snapping |
| `Del` | Delete selection |
| `Ctrl`+`S` | Save |
| `Ctrl`+`E` | Export |
| `Ctrl`+wheel | Zoom the timeline about the pointer |
| wheel | Pan the timeline |

## Adding a command

1. Write the function in `src-tauri/src/lib.rs`, `#[tauri::command]`, returning
   `Result<T, String>`.
2. Register it in `generate_handler![..]`.
3. Add the matching type and wrapper to `src/lib/engine.ts`.

The IPC boundary is JSON, so the compiler cannot check steps 1 and 3 against
each other - `src/lib/ipc-registry.test.ts` does: `npm test` fails on a
registered command with no wrapper, a wrapper naming an unregistered command,
or an `invoke` call anywhere outside `lib/engine.ts`.

## Not decided yet

- **No state library.** React state is enough while the engine owns the model.
  If it stops being enough, the answer is a store outside React (so timeline
  updates do not re-render the tree), not more `useState`.
