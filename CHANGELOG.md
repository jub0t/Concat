# Changelog

What changed in each release, written for the person using the app and
grouped by where they will notice it. A release's notes on GitHub are its
section here: `.github/release-notes.sh` takes the `## <version>` section
for the tag being released, and falls back to the commit subjects for a
release that has none. Releases before 0.2.5 have their notes on the
releases page only: https://github.com/jub0t/Concat/releases

## 0.2.5 — 2026-09-28

The release where the picture pipeline moves onto the GPU end to end.
Every picture effect, transition and colour tool is now a shader drawn in
linear light; HDR footage keeps its light from the file through to an HDR
export; a Scopes pane reads the picture; and a phone gets an editor of its
own instead of a shrunken desktop. It is also the first release the app can
update itself from: Settings › Version lists every release from this one
on and installs the one you pick.

### Colour and HDR

- A timeline has a colour: SDR (Rec. 709), HDR (HLG) or HDR (PQ), set in
  the Modify sheet. The first HDR clip dropped on an SDR timeline turns it
  HLG in the same undo step, and a notice says where to set it back.
- HDR clips - an iPhone's HLG, HDR10's PQ - are decoded at ten bits and
  converted on the GPU. 4K iPhone HLG previews about five times faster
  than in 0.2.4, where the CPU tone-mapped every frame.
- On an HDR timeline the clips keep their light above white. The monitor
  shows it tone-mapped for now; a true HDR preview is next.
- An HDR timeline exports as HDR - HEVC or AV1, ten bits, BT.2020 with HLG
  or PQ, a PQ file carrying its mastering-display, MaxCLL and MaxFALL
  metadata - or as tone-mapped SDR, chosen on a Colour row of the export
  sheet. The API takes `hdr` too.
- A proxy of an HDR file stays HDR (HEVC, ten bits), so a paused HDR
  timeline matches the original. The tone-mapped proxies earlier builds
  wrote of HDR files are made again.
- A Scopes pane: waveform, RGB parade, vectorscope and histogram, counted
  on the GPU from the picture before anything is clipped. It is in the
  pane switcher and docks like the others, scaled in percent on an SDR
  timeline and in nits on an HDR one.
- Adjust works in light. Exposure is in true stops, contrast is in stops
  about middle grey, white balance is a proper chromatic adaptation, and a
  highlight past white is carried on rather than clipped.
- Colour wheels (lift, gamma, gain) and RGB and luma curves, as sections
  of the Adjust tab and as Color Wheels and Curves effects for an
  adjustment layer. A wheel can be keyframed.
- Contrast, Saturation and White Balance as effects of their own.
- Import LUT reads a .cube saved with a byte-order mark and DaVinci
  Resolve's `LUT_3D_INPUT_RANGE` (#208, by theworker02). Tables are kept
  in float, and one named for ACEScct is read in log, so a graded sky does
  not band.

### Effects

- Every picture effect and transition is drawn by its own shader on the
  GPU, in linear light. The FFmpeg chains for pictures are gone, and so is
  the CPU compositor: a machine without a GPU draws the same picture on a
  software adapter. Sound effects keep their FFmpeg chains.
- New: Chroma Key with spill suppression, Luma Key, Zoom Blur, Motion Blur
  with an angle, Color Wheels, Curves, Contrast, Saturation, White Balance
  and Exposure.
- Gaussian Blur is two to four times faster and no longer bleeds colour out
  of transparent pixels; glows and blooms draw at a quarter of the layer;
  Fisheye no longer leaves black holes at the edges.
- The effect cards in the library are drawn by each effect's own shader,
  so a card shows what the effect does. Before, nine effects had no
  preview and sixteen showed another effect's.
- Grain and glitch effects are the same on every GPU and in an export.
- Green Screen and Blue Screen open as Chroma Key keyed to that colour,
  and Box Blur as a Gaussian Blur of the same spread. The other old-format
  effects are retired; see Changes to note.

### Compositing and playback

- The compositor blends in linear light at half-float precision: opacity,
  dissolves, fades, Screen and Add behave as light does, as in Resolve and
  Final Cut. A picture with nothing on it comes out exactly as it went in.
- Fades to black or white and wipes are drawn in the monitor as the export
  draws them. Before, the monitor showed a dissolve for a wipe and nothing
  for a fade.
- A packaged transition is drawn whole on the GPU, with no round trip
  through memory, and the GPU keeps the frames it uploaded, so a scrub
  back finds them there.
- Each stream is decoded by whichever of the CPU and the chip is faster
  for it: 8-bit H.264 on the CPU, everything else on the device. Hardware
  decode is on by default on Windows and Android now.

### Phone

- Android and iOS get an editor shell of their own: the monitor over lanes
  that slide under a centred playhead, Play under the thumb, and a bar of
  tools that opens the inspector and the library as sheets over the lanes.
- The start screen keeps its heading clear of the top edge, and New
  project is a row in the accent colour.
- The Settings page column starts narrow on a phone, scrolls under a
  finger and drops the Remote page; on a desk it can be dragged to size.
- A ProMotion iPhone animates at 120 Hz.

### Editing

- The playhead can follow a trimmed edge, so the monitor shows the frame
  the cut lands on: a toggle on the tray beside the hand tool, off by
  default (#194, by Raphael Bernardo).
- Reverse writes the clip's span backwards into the project's cache, sound
  included, so a reversed clip plays like any other. Freeze, Reverse,
  Mirror and a new Rotate - a quarter turn clockwise, keyable - sit behind
  one mark on the tray.
- A title can be set in any font on the machine, or in one of your own,
  imported from the Text inspector and kept for every project.
- Eleven frame rates to pick from on the launch sheet and the project
  sheet alike, 10 to 60 with the NTSC rates as exact fractions, plus a
  typed rate and a typed frame size up to 8192 (#199). A 23.976 project
  no longer opens the project sheet on 30 fps.
- "Render the sound as a file" is Export Audio: a save dialog asks where
  the file goes, and it comes into Media, selected.

### Export

- A constant-bitrate export gets its bitrate: H.264 holds it, and a CBR
  HEVC export through libx265 no longer fails to open (#104).
- The export draws on the window's GPU device. On Windows laptops with
  NVIDIA chips, pressing Export could take the app down at the first frame
  (#202). The log names the adapter and API an export drew on.

### Settings and updates

- Settings › Version says which release this is, checks GitHub for a newer
  one, and lists every stable release from 0.2.5 on. Install downloads the
  package for this machine, checks its SHA-256 and puts it in place: the
  installer runs on Windows, the app bundle is replaced on macOS, an
  AppImage is written over itself, and a .deb, .rpm or pacman package
  opens in the desktop's installer. Flatpaks and phones are told why the
  button is off.
- Speech voices run on CoreML on every Mac and fall back to the CPU on
  their own; the Engine switch is gone. Chatterbox's first load takes
  about eleven seconds longer per run while CoreML compiles it.
- The Timeline shortcuts section is gone: J no longer flips a clip.
  Flipping is on the inspector and the clip menu's Mirror.

### Fixes

- Delete on a right-click menu over several selected clips left an empty
  menu standing (#160).
- The Captions and Speech sheets lose the stray note in their header's
  corner (#152).
- On Windows the launch sheet's Location came up empty, and a folder
  chosen with Choose... did not show (#203).
- The Traditional Chinese locale is revised (by Peter Dave Hello).

### Changes to note

- Old-format effects are retired. Fifty-six built-in picture effects with
  no GPU twin are gone; a project that used one opens with a notice naming
  them and keeps the link, marked not installed. A picture effect package
  of your own written for format 1, or as an FFmpeg chain, is refused at
  load with a message saying why. Sound packages are unaffected.
- Exposure is in true stops. A project that pushed Exposure in an earlier
  release looks less pushed in this one.
- Dissolves, and the mixes inside spins, zooms and glitches, are brighter
  at the middle, as light is.
- Hardware decode is on by default on Windows and Android, and the
  Decoding switch is gone from Settings.
- The new Concat mark replaces the cat in every icon.
- Under the hood: Slint 1.18, wgpu 30, and locales keyed by ids rather
  than by the English (see TRANSLATING.md). The developer docs and the
  roadmap now live at https://concatenate.pages.dev.
