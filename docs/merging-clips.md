# Merge selected clips

Select two or more video clips on one track, then choose **Merge selected
clips** in the right-click menu or use the merge button beside the scissors
in the timeline toolbar. There is no limit of two clips. Clips can come from
different files, and source sections removed while editing stay removed.

Concat renders the selection in timeline order into `media/merged-*/merged.mp4`
inside the project. Once the render finishes, one clip replaces the selection
at the same position, with the same timeline duration. Timeline gaps inside
the selection remain black/silent gaps. Other tracks are not included.

The file uses the timeline's frame size, frame rate and colour space (H.264
for SDR, ten-bit HEVC for HDR). Clip transforms, effects, speed changes,
internal transitions and sound settings are baked into it. These individual
settings can no longer be edited separately on the merged clip. Encoding
is not lossless. Original source files and bin entries remain available.

Click the merge button again, or choose **Cancel merging** from the context
menu, to cancel. Closing the project also cancels the operation. Errors and
cancellation leave the original clips in place. If the selected clips or their
media change while rendering, Concat discards the result rather than replacing
a newer edit. Undo restores all the original clips in one step; redo restores
the merged clip. Keep the generated media when saving or moving the project.

The action refuses locked tracks, overlapping clips, missing/duplicate ids,
a selection with an unselected clip inside its span, subframe boundaries whose
rounding cannot fit one clip without losing frames, or transitions that cross
its boundary. Include both sides of a transition and reattach detached audio
before merging. MP4 has no alpha channel: merging above other visible content
is refused so a baked black background cannot obscure lower tracks. Rendered
files are project media, not disposable preview cache.

For API callers, the existing `mergeClips` command still rejoins continuous
parts without rendering. The new `consolidateClips` command adopts a completed,
probed render as an atomic edit; callers must render the selection before
applying it. `concat_host::consolidate::Consolidation` builds that request and
checks that its inputs are still current before adoption.
