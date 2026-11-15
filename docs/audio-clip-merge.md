# Merge audio clips

The existing **Merge selected clips** action accepts either two or more video
clips or two or more audio clips on one unlocked track. Mixed audio/video
selections, overlaps and unselected clips within the selected span are refused.
There are no new controls or dependencies.

Audio selections produce `media/merged-<stamp>/merged.wav` inside the project.
The file is uncompressed IEEE float PCM, 32-bit, 48 kHz stereo, using the same
working rate/channel layout as Concat's audio mixer. This avoids an AAC/MP3
encoding generation; it does not claim bit-identical preservation of the
original sample rate or channel layout.

The existing audio graph applies source trims, speed, pitch behavior, filters,
gain automation and fades. Removed source ranges remain removed. Timeline
gaps remain silence. Track mute is retained on the track rather than baked
into the WAV; individually muted clips contribute silence. No pictures or GPU
compositor are needed. Audio duration uses the 48 kHz sample clock, without
the video merge's lower-picture-layer restrictions.

Timeline sound layers stay separate and continue to process the mix after
replacement. They are not baked into the selected clips' WAV, which would
apply them twice. AAC export, WAV merge and sound-layer playback use the
shared upstream mixer, retaining its source timing and channel-layout fixes.
Video merge progress also retains upstream's hardware-encoder restart handling.

On success, one audio clip replaces the selection at its original start and
span. Its gain is unity and fades/filters are cleared because these are baked
into the WAV. Original source media remains in the bin and on disk. One Undo
restores the original pieces; Redo reinstates the merged clip. This creates a
rendered source, not an editable compound clip.

The operation shares the existing export slot, runs off the UI thread, reports
progress and polls cancellation during source setup and mixing. Cancel/error
and stale-project delivery remove unadopted WAV files. Completed media remains
outside disposable caches. Adoption rechecks the captured clips, source media
and timeline settings and honors track locks.

## Validation

Regression tests added for:

- Decoded float WAV codec, channels and exact sample count; source cuts, gain,
  fade-in/out and silent timeline gaps, using a synthetic source.
- All-muted/silent output, cancellation before opening files and during mixing.
- Audio-specific planning, track mute, stale edits and cleanup of partial WAV.
- Replacement type/duration validation, exact timeline span, no double gain or
  fades, persistence and Undo/Redo. Existing video regression tests remain.

Historical automated validation passed on Linux and Windows, with macOS
compilation, in [fork CI run 40098157962](https://github.com/D3monX/Concat/actions/runs/40098157962)
for `d777c147`. Those results do not validate the updated head.

The branch now includes upstream main `020d8caddb8219c77a0bab809c73bc5ca7351509`.
The compatibility update preserves upstream's Opus output, platform AAC
encoders (including Android), continuous speed-curve timing, and video
consolidation of subframe cuts. The short-first-frame regression also covers
the source-time mapping used by tempo steps. All upstream tests remain.

Local checks cover Rust formatting of the resolved files, Git whitespace,
and locale/model inventories. No local compilation or executable tests were
run. The current head needs CI verification and manual comparison of edited
playback before/after merging, cancellation, save/reopen, a muted track, mixed
sources, sound layers, speed curves and existing video merge behavior. Use a
copied project for manual tests.
