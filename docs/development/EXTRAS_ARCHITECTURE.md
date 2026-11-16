# Extras: architecture and comparison

This is the reconnaissance behind Concat's Extras system: what Concat already
had, what Drift's Extras does, and what was built. Drift (GPL-3.0) was read as a
reference for behaviour and design only. No Drift code, assets or text were
copied. Everything here is reimplemented on Concat's own crates.

## What Concat already had

Concat is a Rust workspace (`src/crates/*`, edition 2024, Rust 1.93.1). The
window is Slint (`concat`). The engine crates have no UI.

| Concern | Where it lives | What it does |
|---|---|---|
| App folders | `concat-host::dirs::AppDirs` | `config` (`%APPDATA%\app.concat.editor`) for settings and small state, `data` (`%LOCALAPPDATA%\app.concat.editor`) for models and caches. A `portable` folder next to the exe holds both. |
| Network | `concat-host::net::get` | One way out. It uses rustls with the shipped and system roots (`Trust::System`), or the shipped roots only (`Trust::Shipped`) for manifests that vouch for downloads. It also classifies failures into causes a person can act on. |
| Downloads | `concat-host::models::{download, verify}` | Resumable `.part` downloads with Range requests, a cancel flag, progress callbacks, and a SHA-256 check before anything is called installed. Downloads with no digest are refused. |
| Effects, filters, transitions, audio effects | `concat-effects` | Declarative packages: a folder with `effect.toml` (TOML), plus a WGSL shader for pictures or an FFmpeg chain template for sound. Built-ins are compiled in. User packages load from `config/effects/<id>/`, are GPU-trialled against a timeout, and are hot-reloaded by a 2 s folder stamp watch (`studio/speech.rs`). Dangerous FFmpeg filters (`movie`, `frei0r`, `sendcmd`…) are refused at load. |
| Fonts | `concat/src/presets.rs` | `config/fonts/` (flat), offered by the font picker via `installed_fonts`. |
| Title presets | `concat/src/presets.rs` | `config/text-presets/*.toml` or `*/preset.toml`, merged over built-ins by id. |
| Stickers | `concat` Stickers page | Built-in vector shapes drawn in code. There are no image stickers and no image asset browser. |
| Sounds | `concat-host::sounds` | The user's library (`data/audio/`), plus Concat's sounds listed by a remote manifest and fetched on first use. |
| AI models | `concat-vision::models`, `concat-speech::{transcribe,tts}` | Compiled-in tables (file, upstream, size, SHA-256, licence). A model's identity is compiled in on purpose. They download from Settings → Transcriber / Speech, with mirror and source preferences. |
| AI runtimes | `ort` (ONNX Runtime 1.28), whisper.cpp, sherpa-onnx | Linked into the build: DirectML on Windows, CoreML on Apple, NNAPI on Android. Nothing native is loaded at run time. |
| Missing effects | `concat-effects` | An applied id the catalogue does not know is skipped at render time. The document keeps it untouched. |
| UI pattern | `concat/src/panes/*` | Elm-style: one `Msg` enum per pane, workers via `host::spawn`, results through `on_ui`, and a `publish` that pushes plain structs to Slint. |
| Localisation | `concat/locales/*.json` | `I18n.t("key")` in Slint and `t`/`tf` in Rust. `scripts/locales.py --check` keeps 26 locales in step. |

There was no package manager. Each downloadable thing (models, sounds,
updates) had its own table and flow. There was no catalogue of optional
content, no install database, and no dependency handling.

## What Drift's Extras does (read-only study)

Verified in source, not assumed from docs:

* **Entry point**: an Extras button in the editor header (`EditorHeaderExtrasButton.qml`).
  It pulses when essentials or updates are pending, and never opens on its own.
  The store opens in its own window (`ExtrasWindow.qml` → `ExtrasStore.qml`).
* **Store layout**: search, a section sidebar (Discover, Installed, Updates,
  then catalogue categories, with engines and runtimes last), a grid of cards,
  a detail page, multi-select, and one install bar.
* **Catalogue**: a remote *index* (what exists, at which version, download
  links) and a separate *catalog* (store metadata such as categories, shelves,
  `requires`, `recommends` and images). Both are fetched through a Worker with a
  client token, and cached for six hours so an offline launch stays quiet.
* **Package format**: `.driftpkg` is a custom container: magic, version, JSON
  manifest, one zstd payload, a SHA-256 over everything, and an Ed25519
  signature against a compiled-in key. The file table carries per-file SHA-256s.
  `.driftfx` is the same layout without a signature, for user packages.
* **Install**: one streaming pass into `<dest>.partial`. Each file is hashed as
  it lands, and the staging dir is promoted only once the signature verifies.
  Paths with `\`, `:`, `.`, `..`, or absolute forms are refused.
* **Registry**: `addons/installed.json`, rewritten atomically. A pack declares
  `provides: [{kind, root}]`, and catalogues ask `addonRootsForKind(kind)`
  for their search roots. Safe mode reads the registry as empty.
* **Dependencies**: `requires` names an addon id or a *kind*, and
  `prefer[platform]` picks a provider. A plan puts dependencies first; a failed
  or cancelled dependency drops its dependents.
* **Unsigned files**: install only after an explicit confirmation.
* **AI**: ONNX Runtime and execution providers ship *as addons with native
  code* (platform-tagged), plus an acceleration chooser and a restart notice
  when the runtime changes.
* **Missing content**: when a project loads, the addons it references that are
  not installed are reported: "Open Extras to install it."

## Comparison

| Area | Concat (before) | Drift | Concat Extras (built) |
|---|---|---|---|
| UI entry point | Settings gear in the title bar | Header Extras button | **Extras** button beside the Settings gear. It works on the launch screen and in the editor, and opens a sheet like Settings. |
| Package catalogue | Per-feature tables (models, sounds) | Remote index plus store catalog, token-gated | Signed `index.json` per repository: Ed25519 over the exact bytes, verified against the repository's configured key, fetched with `Trust::Shipped`, and cached. The official repository plus optional ones in `config/extras/repositories.toml`. |
| Package format | Folders (effects), raw files (models) | `.driftpkg` custom container plus zstd | `.concatpkg` = gzip'd tar with `extra.toml` at the root. The manifest lists every file with its size and SHA-256. Standard tools can open it, and nothing is executable. |
| Download manager | `models::download` (resume, cancel, digest) | Qt network, a queue, one extraction at a time | **Reused** `concat_host::models::download` and `net::get`. Extras adds a queue, a progress model and cancellation per item. |
| Installation manager | `.part` then rename (models) | `.partial` then promote after the signature | Verify the archive SHA-256 against the signed index, then extract to `staging/` with path, type and size checks and per-file SHA-256. A kind-specific vet follows (effects must load). Then an atomic rename into `packages/<id>/`, and `installed.json` is written last. |
| Dependency manager | none | id or kind requirements, platform preference | id requirements with semver ranges, topological order, cycle and conflict detection, a missing-dependency report, and a reverse-dependency check on uninstall. |
| Effects / filters / audio effects | `config/effects/` hot-reload | addon roots per kind | `concat-effects` loads from several roots: user effects first, then each enabled extra's `effects` root. Same GPU trial and same watch. |
| Transitions | effect packages with `kind = "transition"` | separate kind | Same `effects` kind. A transition *is* an effect package in Concat. The category is display only. |
| Fonts | `config/fonts/` | `fonts` kind | `installed_fonts` also walks each enabled extra's `fonts` root. |
| Title presets | `config/text-presets/` | effect-templates / looks | `presets::user` also reads each enabled extra's `title-presets` root. |
| Stickers | built-in vector shapes | `stickers` kind (images) | A new **Pictures** shelf beside Shapes on the Stickers page (PNG, JPEG, WebP). A press copies the picture into the project and places it over the picture at the playhead. |
| Sounds | library plus remote sound list | `audio` packs | A **From Extras** shelf on the Audio page, with the same preview, add, drag and copy-into-project path as the user's own sounds. |
| AI tools / models | Settings rows, compiled-in tables | ONNX models as addons | The **AI models** page in Extras shows the existing transcriber and voice models with the same rows and callbacks as Settings. Their identity stays compiled in, so there is no parallel model system. |
| AI engines | linked into the build | native-code addons | Shown as built-in facts on the AI page. **Native code is never installed by Extras** (security model). |
| Security | digests, shipped roots, filter denylist | signature plus digest, confirmation for unsigned | Signed index, archive digest, per-file digests, manifest field validation, no symlinks or hardlinks or devices, no `..`/absolute/drive/`\` paths, size and count limits, extension allowlists per kind, HTTPS only (or `file:` for a local repo), and confirmation for unverified side-loaded files. |
| Uninstallation | delete model file | delete dir and forget | Refused while an enabled extra depends on it, unless forced. Warns when the open project uses something it provides. The folder is moved aside, then deleted, then forgotten. **Disable/enable** without deleting. |
| Missing content | id skipped at render | "Open Extras to install it" | On project open, applied effect ids that no catalogue knows are matched against the index's `provides_ids`. A toast names the extra and points to Extras. The id stays in the document untouched. |

## Shape of the implementation

```
concat-extras  (new crate, no network, no UI)
  manifest.rs   extra.toml: fields, validation, kinds, categories
  index.rs      the repository index, its Ed25519 signature, compatibility
  resolve.rs    install plans: dependencies, versions, cycles, conflicts
  archive.rs    .concatpkg reading/writing with every safety check
  registry.rs   installed.json, roots per kind, enable/disable, provided ids
  install.rs    the pipeline: verify → stage → vet → commit, and uninstall
  pack.rs       authoring: pack a folder, write an index, sign it, keys

concat-host::extras   paths under AppDirs.data, repositories, fetch & cache
                      the index (net::get), download (models::download),
                      vetting per kind (effects through concat-effects)
concat-effects        Catalogue::install_with_roots / package_stamp over roots
concat (window)       panes/extras.rs + ui/dialogs/extras.slint, title-bar
                      button, effect/font/preset roots, missing-extra notice
concat-cli            `concat extras pack|index|sign|keygen|verify`
```

The engine crate holds everything that decides whether something is safe and
installed. The host only moves bytes, and the window only shows state. This
matches the split between Drift's AddonRegistry and AddonManager, and Concat's
own split between engine and host.
