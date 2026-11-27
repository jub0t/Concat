<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
<!-- SPDX-FileCopyrightText: 2026 Jareer Abdullah -->

# Concat for Android (Kotlin + Jetpack Compose)

The native phone app: Compose screens over the Rust engine
(`src/crates/concat-mobile`). The app edits only through the editor's
JSON API (`concat-api`), one `EditorApi.call(json)` per request plus an
events callback, and draws the preview on a SurfaceView through `Preview`.

## Engine or in-memory stand-in

Everything in `app/src/main` talks to `engine.EngineBridge`:

- `FakeEngineBridge` (src/main): answers the requests and commands the app
  sends from memory. Projects are JSON files in their folders, the
  preview is a placeholder painted on the Surface, and an export copies the
  first video. It is used whenever the native bridge is missing or its
  library fails to load.
- `NativeEngineBridge` (`app/src/native/java`): wraps the generated
  `uniffi.concat_mobile.EditorApi` / `Preview` / `ApiEvents`.

`app/build.gradle.kts` adds `src/native/java` to the main source set only
when `app/src/main/java/uniffi` exists. `build-rust.sh` generates that
folder, and so does CI's `android-native` job. `EngineFactory` loads
`NativeEngineBridge` by name, so `src/main` never refers to it. As a result:

| `app/src/main/java/uniffi` | What builds | What runs |
|---|---|---|
| absent | the fake bridge only | the in-memory engine (the projects screen says so) |
| present (+ `jniLibs`) | both | the engine; the fake if `libconcat_mobile.so` will not load |

`BuildConfig.ENGINE_BINDINGS` records which case a build was made in.

## Building

```sh
# Without the engine (any machine):
./gradlew :app:assembleDebug

# With it: the engine and bindings first (slow; CI does this), then Gradle.
./build-rust.sh release && ./gradlew :app:assembleDebug
```

On Windows, set `JAVA_HOME` to Android Studio's `jbr` and point
`local.properties` at the SDK (`sdk.dir=C:/Users/<you>/AppData/Local/Android/Sdk`).

This is the app that ships as `app.concat.editor`, the Slint app's id, so
it installs over it as an update: same key (CI signs a release with the
repository's), a version code that continues the old numbering (see
`app/build.gradle.kts`), and the same folders, so projects in recents,
downloaded models and settings carry over. A release build refuses to be
made without the engine, and one whose engine will not load shows why
instead of running on the in-memory stand-in.

```sh
# An emulator on an x86_64 machine, with CI's export check (Smoke.kt):
./build-rust.sh release x86_64-linux-android
./gradlew :app:assembleRelease -Pabis=x86_64 -Psmoke=true
../scripts/ci/smoke-android.sh --export ../src/crates/concat-media/fixtures/picture-starts-late.mov \
  app/build/outputs/apk/release/app-release.apk
```

## Design

The look follows the design notes (MOBILE_DESIGN.md): the desktop's dark theme with
the lime accent, and a phone editor's layout: preview on top, timeline
below, tools along the bottom.

- `ui/theme/Theme.kt`: the tokens from `src/crates/concat/ui/theme/dark.slint`
  (surfaces, text, accent, status and clip-kind colours), Inter (bundled in
  `res/font`, licence in `assets/licenses`), spacing, radii, the editor's
  measurements and motion.
- `ui/icons/ConcatIcons.kt` is generated, never edited: the desktop's glyphs
  from `src/crates/concat/ui/icons.slint` as `ImageVector`s. After a change
  to icons.slint, run `python android-native/scripts/icons_from_slint.py`.
- The timeline's filmstrips and waveforms come from `media.filmstrip` and
  `media.peaks` through `media/MediaCache.kt`, an in-memory LRU cache; the
  in-memory engine paints stand-ins for both.
- The projects screen shows the frame and length the editor leaves in each
  project folder as it closes (`.concat-thumb.png`, `.concat-card.json`).
