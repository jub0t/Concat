# Extras: implementation

How the Extras system is built, file by file. For why it is shaped this way,
see [EXTRAS_ARCHITECTURE.md](EXTRAS_ARCHITECTURE.md). For the format, see
[EXTRAS_PACKAGE_FORMAT.md](EXTRAS_PACKAGE_FORMAT.md). For the threat model, see
[EXTRAS_SECURITY.md](EXTRAS_SECURITY.md).

## Layers

| Layer | Crate / file | Owns |
|---|---|---|
| Engine | `src/crates/concat-extras` (new) | Everything that decides: the format, the signed index, plans, the install database and the install pipeline. No network and no UI. |
| Host | `src/crates/concat-host/src/extras.rs` (new) | Paths under `AppDirs`, repositories, fetching and caching the index, downloads, per-kind vetting, the free-space check. |
| Effects | `src/crates/concat-effects/src/catalogue.rs` | `Catalogue::install_from` / `assemble` over several roots, and `packages_stamp`. |
| Window | `src/crates/concat/src/panes/extras.rs` (new), `ui/dialogs/extras.slint` (new) | The sheet, the install queue, progress, confirmations, and reloading what reads extras. |
| CLI | `src/crates/concat-cli/src/extras.rs` (new) | `concat-cli extras keygen / pack / repo / verify / inspect`. |

### `concat-extras`

* `manifest.rs`: `extra.toml` (`Manifest`, `Info`, `Provide`,
  `Requirement`, `FileEntry`), `Kind` (the extension points), `Category`
  (shelves), and `validate()`. That covers ids, versions, text limits, roots
  that cannot overlap, per-kind file types, `safe_path`, case collisions and
  size limits. `compatible()` checks `min_app` and `platforms`.
* `index.rs`: `Index` / `Entry` / `Download`. `verify_and_parse` checks the
  Ed25519 signature (ring) before parsing. `resolve_url` resolves archive URLs
  relative to the index. `provider_of(effect id)` drives the missing-extra
  notice.
* `resolve.rs`: `plan(requested, available, installed)` returns `Steps`
  (Install / Upgrade / Reinstall / Enable, dependencies first) plus `Missing`
  (unavailable, wrong version, a conflict with an installed dependent).
  Cycles are an error. `dependents()` is used for remove and switch-off.
* `archive.rs`: `read_manifest`, `extract` (all the checks, with a
  decompression budget), `write` (deterministic), and `sha256_file`.
* `registry.rs`: `installed.json` (`Registry`, `Record`, `Origin`), saved
  atomically. `roots(kind)` returns the folders of enabled extras. `stamp()`
  is used by the package watch.
* `install.rs`: `install()` runs verify, read the manifest, compatibility,
  downgrade refusal, dependency check, extract to `staging/`, vet, then
  commit (old version to `trash/`, rename, write registry, roll back on
  failure). `uninstall()`, `set_enabled()` and `sweep()` cover the rest.
* `pack.rs`: `pack()` (the folder plus a generated file list), `index_entry()`,
  `write_index()`, `keygen()`, `sign()`, `public_key()` and
  `build_repository()`.

### `concat-host::extras`

* `dir(dirs)` is `<data>/extras`, added to `dirs::DATA_FOLDERS` so the
  layout migrations carry it. Portable builds keep it in the portable folder.
* `repositories(dirs)` returns the official repository (when this build has
  `OFFICIAL_KEY`) plus `<config>/extras/repositories.toml`.
* `load_index` / `load_all` fetch `index.json` and `.sig` (HTTPS via
  `net::get` + `Trust::Shipped`, or `file:`), verify them, and cache them for
  six hours. When a repository can't be reached they fall back to the verified
  cache, marked stale. `cached_indexes()` reads the cache only, with no
  network.
* `download()` reuses `models::download` (resume, Range, cancel). It stops a
  server that sends more than the index says, and keeps the finished archive
  under `downloads/`.
* `vet()` checks effects (load and compile, reserved author, id clashes),
  fonts (must parse) and images (magic bytes), and returns the provided effect
  ids.
* `install_entry()` / `install_file()` / `inspect_file()` drive the engine's
  `install()` with the right `Expected` and `Origin`. `check_room()` checks
  free space using `GetDiskFreeSpaceExW` or `statvfs`.

### The window

* Entry point: an **Extras** button (new `Glyph.extras`) beside the Settings
  gear in the title bar, on both the launch screen and the editor.
  `app.slint` holds `ExtrasDialog` (a `Sheet`, sized like Settings) and the
  `extras-*` callbacks. `lib.rs` routes them to `Msg::Extras`.
* `panes/extras.rs` (`ExtrasPane`) works like every other pane: a `Msg`
  enum, `update`, `data()` / `cards()`. Workers run through `host::spawn`,
  with progress through `on_ui`, throttled to one report per percent and per
  phase.
* Pages: Discover, Installed, Updates, Downloads, seven category shelves,
  and AI models. AI models shows the existing transcriber and voice
  `ModelRow`s (exported from `settings.slint`) with the existing `model-*`
  callbacks, so models keep one source of truth.
* One install at a time from a queue. A failed or cancelled step drops the
  queued steps that need it, with a reason. Downloads over 100 MB, files no
  repository vouches for, and removals go through the shared confirm sheet
  (`Confirm::ExtrasDownload / ExtrasUnverified / ExtrasRemove`).
* After any change, `reload()` rebuilds the effects catalogue
  (`reload_packages`, which GPU-trials new shaders), redraws the effect
  cards, and re-reads `installed_fonts` and `text_presets`.

### Resource integration

| Kind | Where it appears | How |
|---|---|---|
| effects (filters, effects, transitions, audio effects) | The Effects/Filters/Transitions/Audio shelves and inspector, export and preview | `concat_host::extras::effect_roots` (`config/effects` first, then `roots(Effects)`) → `Catalogue::install_from`. The window, the API, MCP and the CLI all read this one list. The package watch stamps every root plus `installed.json`, so installing, removing or switching an extra off reloads within 2 s. `reload_packages` then drops the monitor's cached plan and frames, so the change shows on the next frame. |
| fonts | Font picker and titles | `presets::installed_fonts` also walks `roots(Fonts)`. A title set in one registers it on the project like any installed font. |
| title presets | Text page and caption styles | `presets::user` also reads `roots(TitlePresets)`. The user's own presets win on id. |
| stickers | Stickers page → **Pictures** shelf (beside Shapes) | `extra_files(Stickers, png/jpg/jpeg/webp)` → `Studio::extra_stickers`. Cards are drawn once and cached. A press copies the picture into the project's `stickers/` folder, imports it, and places it at the playhead on the first free lane over the picture (`Studio::place_sticker`). |
| sounds | Audio page → **From Extras** shelf (beside My sounds) | `extra_files(Sounds, …)` with lengths probed on a worker → `SoundsPane::extras`. Cards are `extra-<id>/<path>` and go through the same preview, **+**, drag and placement code as the user's own sounds. The file is copied into the project's `audio/` folder (`Studio::adopt_file`). |
| AI models | Extras → AI models | The existing Settings rows and download machinery. |

### Headless tools and the API

`Api::load_installed_packages()` loads the same roots into the catalogue.
`concat-cli api`, `concat-cli serve` (once, as its one `Api` is made at
start), `concat-cli preview` and the headless MCP server call it. They read
what is installed when they start: an extra installed or switched while one
of them runs is seen when it is started again. `Api::new` does not, because the
window also builds an `Api` for its automation server over a catalogue it has
already GPU-trialled. The API adds a read-only **`extras.list`** method (id,
name, version, author, category, enabled, verified, kinds, provided ids). The
effects an enabled extra brings are also in `catalogue.list`. Installing and
removing stay in the Extras sheet, where a person sees what is being trusted.

### Accessibility

- The shared `Button` (`primitives/modal.slint`) and `PageTab`
  (`dialogs/settings.slint`) now have an accessible role, name, state and
  default action, a keyboard focus scope (reached by Tab, pressed with Space
  or Return), and the existing focus ring for keyboard focus only.
- A mouse click does not take keyboard focus, so the editor's shortcuts stay
  with the window after a dialog button is clicked, as before.
- The fix covers every dialog's buttons and Settings' tabs, not only Extras'.
- The Extras rows and the picture-sticker cards have the same treatment.

### Projects and missing extras

When a project opens, `panes::extras::missing_extras` finds the effect and
transition ids the catalogue lacks that an installed-but-switched-off extra,
or a cached signed index, provides. It shows one notice ("This project uses
effects from X … Open Extras to add it") instead of the generic
retired-effects notice. The links in the document are never rewritten or
replaced. Removing an extra names what the open project uses from it before
asking.

## Adding a new kind

1. Add a variant to `concat_extras::Kind` (`key`, `extensions`, `ALL`) and
   to the `kind_label` text.
2. Add content checks in `concat_host::extras::vet`.
3. Read `concat_host::extras::roots(dirs, Kind::New)` where that content is
   consumed, and reload it in `panes::extras::reload`.
4. Document it in EXTRAS_PACKAGE_FORMAT.md, and test it.

## Limitations

* **Picture stickers** are placed by a click, at the playhead. Dragging one
  onto a lane, as shapes can be, is not wired yet. SVG and GIF are not
  sticker types: nothing in Concat rasterises SVG, and a GIF would import as
  a clip of its frames.
* **Headless rendering**: the API's `preview.frame` draws an extra's effects
  (measured), but it doesn't draw transition clips for any transition,
  built-in or extra. That is existing behaviour of the headless preview. The
  window draws them. `concat-cli render` takes a media file, not a project,
  so it doesn't apply.
* **Headless shader trial**: the CLI, API and MCP compile every package's
  shader but cannot trial-run it on a GPU against a timeout as the window
  does. A shader that loads but hangs a GPU would be caught in the window,
  not headless.
* A title preset's card is drawn in the fallback face, not in the font the
  preset brings. Applying the preset uses the right font.
* The **official repository** has no key in this build, so it offers
  nothing until the maintainers publish a signed index (see EXTRAS_SECURITY.md).
* **AI engines** aren't installable. They are linked into the build by design.
* A font from an extra that a project registered keeps its path. Removing
  the extra makes that face unavailable to the project, just like deleting an
  imported font from `config/fonts` would.
* **Accessibility**: a few controls Extras uses still have no accessible name
  or action: the sheet's close button (the shared `Sheet`), the Audio page's
  row **+** (`RowButton`) and the shelves' category rows (`CategoryRow`). The
  title presets' cards aren't exposed either. They work with the mouse.
* The sheet lists extras as rows, not a grid, and has no preview images yet.
  `icon.png` and `preview.png` are allowed in packages for a later version.
