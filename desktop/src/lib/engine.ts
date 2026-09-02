// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

/**
 * The typed boundary between the UI and the Rust engine.
 *
 * Every `invoke` in this app goes through this file. Nothing else imports
 * `@tauri-apps/api/core` for commands, so when a command's shape changes there
 * is exactly one place to fix and the compiler finds every call site.
 *
 * The engine-owned wire types (`EditorView`, `EditorCommand`, `ExportClip`,
 * ...) are generated from the Rust serde structs by ts-rs and re-exported
 * from `./generated/` - see the README there - so the compiler checks the
 * IPC boundary against the source of truth. The smaller host-only shapes
 * (probe results, template info, transcriber status) are still hand-mirrored
 * here; keep those in step with `src-tauri/src/*.rs`.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { EditorCommand, EditorView, NewMedia } from "./editor";
import type { ExportClip } from "./generated/export/ExportClip";

export interface VideoStreamInfo {
  index: number;
  codec: string;
  width: number;
  height: number;
  /** Frames per second as a decimal, for display only. */
  frameRate: number;
  /** The exact fraction, e.g. "30000/1001". The engine works in these. */
  frameRateFraction: string;
}

export interface AudioStreamInfo {
  index: number;
  codec: string;
  sampleRate: number;
  channels: number;
}

export interface MediaSummary {
  path: string;
  /** Seconds, or null when the container does not say. Always null for stills. */
  duration: number | null;
  /** Decided by the host, which sees the extension as well as the streams. */
  kind: "video" | "audio" | "image";
  video: VideoStreamInfo | null;
  audio: AudioStreamInfo | null;
}

/** Asks the engine what is inside a media file. Throws with FFmpeg's message. */
export async function probeMedia(path: string): Promise<MediaSummary> {
  return invoke<MediaSummary>("probe_media", { path });
}

/** A probe result in the shape `addMedia` and `fillSlot` take. */
export function newMediaFromSummary(summary: MediaSummary): NewMedia {
  return {
    path: summary.path,
    name: summary.path.split(/[\\/]/).pop() ?? summary.path,
    duration: summary.duration,
    kind: summary.kind,
    width: summary.video?.width ?? null,
    height: summary.video?.height ?? null,
    frameRate: summary.video?.frameRate ?? null,
    frameRateFraction: summary.video?.frameRateFraction ?? null,
    videoCodec: summary.video?.codec ?? null,
    audioCodec: summary.audio?.codec ?? null,
    hasAudio: summary.audio !== null,
  };
}

/** Reports the app version the UI is talking to. */
export async function engineVersion(): Promise<string> {
  return invoke<string>("engine_version");
}

/**
 * Reads a whole file into memory as bytes.
 *
 * Two callers: still-image decode in `lib/assets.ts`, and custom font
 * registration. It loads the entire file and the host refuses anything past
 * its size cap, so do not reach for it as a general-purpose reader.
 */
export async function readMediaBytes(path: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("read_media_bytes", { path });
}

/**
 * Waveform peaks for one media file, decoded and bucketed by the engine.
 *
 * Returns the encoded form `decodePeaks` in `lib/assets.ts` reads. The host
 * caches the result in the project folder, so only the first call per file
 * pays for a decode; `project: null` skips that cache.
 */
export async function extractPeaks(path: string, project: string | null): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("extract_peaks", { path, project });
}


/** A project on disk, as the launch screen sees it. */
export interface ProjectInfo {
  path: string;
  name: string;
  width: number;
  height: number;
  rateNum: number;
  rateDen: number;
  /** Milliseconds since the epoch. */
  openedAt: number;
}

/**
 * Creates the project folder and writes its manifest.
 *
 * The returned path may differ from `location/name`, because the name has to
 * be made filesystem-safe first.
 */
export async function createProject(request: {
  location: string;
  name: string;
  width: number;
  height: number;
  rateNum: number;
  rateDen: number;
}): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("create_project", request);
}

/** Reads an existing project's settings and marks it as recently opened. */
export async function openProject(path: string): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("open_project", { path });
}

// ── playback, transport and artwork ────────────────────────────────────────

/** One audible clip, as `playback.rs` mixes it. */
export interface AudioClipSpec {
  path: string;
  /** Timeline seconds. */
  start: number;
  duration: number;
  /** Seconds into the source file. */
  sourceStart: number;
  volume: number;
  fadeIn: number;
  fadeOut: number;
  speed: number;
  preservePitch: boolean;
  /** FFmpeg audio filter chain, or empty. */
  chain: string;
}

/** Replaces the engine mixer's audible clip set wholesale. */
export async function audioSetClips(project: string, clips: AudioClipSpec[]): Promise<void> {
  return invoke<void>("audio_set_clips", { project, clips });
}

export async function transportPlay(position: number): Promise<void> {
  return invoke<void>("transport_play", { position });
}

export async function transportPause(): Promise<void> {
  return invoke<void>("transport_pause");
}

export async function transportSeek(position: number): Promise<void> {
  return invoke<void>("transport_seek", { position });
}

/**
 * Subscribes to playback failures the engine would otherwise swallow - a
 * missing output device, a clip whose audio would not decode. Resolves to an
 * unsubscribe function.
 */
export async function onAudioError(handler: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>("audio://error", (event) => handler(event.payload));
}

/** One cached artwork file from the project's cache, or a throw the caller
 * treats as a miss. */
export async function readArtwork(project: string, key: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("read_artwork", { project, key });
}

/** Stores one artwork file in the project's cache. Fire-and-forget shaped:
 * a failed write only means regenerating next launch. */
export async function writeArtwork(
  project: string,
  key: string,
  bytes: Uint8Array,
): Promise<void> {
  return invoke<void>("write_artwork", { project, key, bytes: Array.from(bytes) });
}

/** A strip of evenly spaced frames as one JPEG, for timeline filmstrips. */
export async function extractFilmstrip(
  path: string,
  count: number,
  height: number,
): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("extract_filmstrip", { path, count, height });
}

// ── the editing session ────────────────────────────────────────────────────
// The engine owns the edit (see engine decision 0007): the UI opens a
// session, sends commands, and renders the state that comes back.

/** Opens a project folder as the engine's editing session. */
export async function editorOpen(session: {
  path: string;
  name: string;
  width: number;
  height: number;
  rateNum: number;
  rateDen: number;
}): Promise<EditorView> {
  return invoke<EditorView>("editor_open", session);
}

/** Applies one edit command; the returned state is the truth. */
export async function editorApply(command: EditorCommand): Promise<EditorView> {
  return invoke<EditorView>("editor_apply", { command });
}

export async function editorUndo(): Promise<EditorView> {
  return invoke<EditorView>("editor_undo");
}

export async function editorRedo(): Promise<EditorView> {
  return invoke<EditorView>("editor_redo");
}

/** The current state, without changing anything. */
export async function editorState(): Promise<EditorView> {
  return invoke<EditorView>("editor_state");
}

/** Writes the session's document to disk. The output size rides along
 * because the preview footer can edit it; the name because the project
 * details dialog can. */
export async function editorSave(update?: {
  width: number;
  height: number;
  name?: string;
}): Promise<void> {
  return invoke<void>("editor_save", {
    name: update?.name ?? null,
    width: update?.width ?? null,
    height: update?.height ?? null,
  });
}

/** Closes the session, dropping its undo history. */
export async function editorClose(): Promise<void> {
  return invoke<void>("editor_close");
}

/** Recently opened projects, newest first, with vanished folders left out. */
export async function recentProjects(): Promise<ProjectInfo[]> {
  return invoke<ProjectInfo[]>("recent_projects");
}

/** Drops a project from the recents list. The folder itself is untouched. */
export async function forgetProject(path: string): Promise<void> {
  return invoke<void>("forget_project", { path });
}

/**
 * A poster frame for one project, as JPEG bytes - the launch screen's
 * thumbnail. Cached in the project folder by the host; throws when the
 * project has nothing visual to show, which the caller treats as "no art".
 */
export async function projectPreview(path: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("project_preview", { path });
}

// ── templates ──────────────────────────────────────────────────────────────
// A template is a project packed into a bundle: its design media (music,
// overlays) ship inside, its placeholder media become slots the user fills.
// The host owns the library; see src-tauri/src/templates.rs.

/** One placeholder the user's media will replace. */
export interface TemplateSlot {
  mediaId: string;
  name: string;
  kind: "video" | "audio" | "image";
  /** Timeline seconds this slot covers, for the fill list. */
  seconds: number;
}

/** One template, as the gallery sees it. */
export interface TemplateInfo {
  /** The bundle folder on disk. */
  path: string;
  name: string;
  width: number;
  height: number;
  rateNum: number;
  rateDen: number;
  /** Slots in the order they first appear on the timeline. */
  slots: TemplateSlot[];
  hasPoster: boolean;
}

/** The user's media for one slot, straight from a probe. */
export interface SlotFill {
  mediaId: string;
  item: NewMedia;
}

/** The template library, in name order. */
export async function templateList(): Promise<TemplateInfo[]> {
  return invoke<TemplateInfo[]>("template_list");
}

/** Packs the open project into a new template bundle. */
export async function templateSave(name: string): Promise<TemplateInfo> {
  return invoke<TemplateInfo>("template_save", { name });
}

/**
 * Unpacks a template into a fresh project with every slot filled, and
 * returns it ready to open. Every slot needs a fill; the host refuses a
 * partial set before anything is written.
 */
export async function templateInstantiate(request: {
  template: string;
  location: string;
  name: string;
  fills: SlotFill[];
}): Promise<ProjectInfo> {
  return invoke<ProjectInfo>("template_instantiate", request);
}

/** Removes one template bundle for good. */
export async function templateDelete(path: string): Promise<void> {
  return invoke<void>("template_delete", { path });
}

/** A template's poster as JPEG bytes; throws when there is none. */
export async function templatePoster(path: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("template_poster", { path });
}

/**
 * One clip, flattened for the exporter - generated from the engine's own
 * `wolfcut_export::ExportClip` (see ./generated/README.md), so the UI's
 * rasterised title overlays are typed by the struct that deserialises them.
 */
export type { ExportClip } from "./generated/export/ExportClip";

export interface ExportRequest {
  output: string;
  crf: number;
  preset: string;
  /**
   * Rasterised titles, rejoining as image clips. The timeline itself is
   * deliberately absent: the engine flattens its own session (engine
   * decision 0009), so size, rate and clips all come from the model. The
   * webview contributes only what it genuinely owns - fonts and layout.
   */
  titles: ExportClip[];
}

export interface ExportProgress {
  frame: number;
  total: number;
  stage: string;
}

/** Renders the timeline. Resolves with the path written. */
export async function exportProject(request: ExportRequest): Promise<string> {
  return invoke<string>("export_project", { request });
}

/**
 * Asks the running export to stop at the next frame. The pending
 * `exportProject` call then rejects with "export cancelled".
 */
export async function cancelExport(): Promise<void> {
  return invoke<void>("cancel_export");
}

/**
 * The engine-composited frame at one instant: the exporter's own plan,
 * compositor and effects, fed from the host's reader pool and flattened
 * from the engine's own session. Raw RGBA bytes, exactly
 * `width * height * 4` of them. This is what the paused monitor shows -
 * the true frame, not the approximation.
 */
export async function previewFrame(request: {
  time: number;
  width: number;
  height: number;
}): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("preview_frame", { request });
}

/**
 * Warms the engine's frame cache for the instants about to play.
 *
 * Fire and forget, from the playback stream only: after presenting a frame,
 * asking for the next `frames` starting at `request.time` turns the next
 * `previewFrame` pulls into cache hits instead of decode waits. Failures
 * are silent here - a source that will not decode fails the pull too, and
 * the pull is the path that reports it.
 */
export async function previewPrefetch(
  request: { time: number; width: number; height: number },
  frames: number,
): Promise<void> {
  return invoke<void>("preview_prefetch", { request, frames });
}

/**
 * Writes one file into the project's cache folder and returns its path.
 *
 * The same store the artwork cache uses, so it travels with the project and
 * vanishes with it. Keys are flat filenames; the host refuses anything else.
 */
export async function writeCacheFile(
  project: string,
  key: string,
  bytes: Uint8Array,
): Promise<string> {
  await invoke<void>("write_artwork", { project, key, bytes: Array.from(bytes) });
  return `${project}/cache/${key}`;
}

/** Subscribes to export progress. Resolves to an unsubscribe function. */
export async function onExportProgress(
  handler: (progress: ExportProgress) => void,
): Promise<UnlistenFn> {
  return listen<ExportProgress>("export://progress", (event) => handler(event.payload));
}

// ── transcription ──────────────────────────────────────────────────────────

/** One Whisper model, as the settings panel sees it. */
export interface TranscriberModel {
  id: string;
  label: string;
  blurb: string;
  englishOnly: boolean;
  /** Approximate download size in bytes, for display. */
  sizeBytes: number;
  downloaded: boolean;
}

export interface TranscriberStatus {
  /** Where `whisper-cli` was found, or null when it was not. */
  binary: string | null;
  /** True when the binary in use is the copy shipped inside the app. */
  bundled: boolean;
  /** Where models are stored on disk. */
  modelsDir: string;
  models: TranscriberModel[];
}

/** Progress of a model download, via `transcriber://download`. */
export interface TranscriberDownload {
  id: string;
  received: number;
  total: number;
  done: boolean;
}

/** One caption, in seconds relative to the transcribed window's start. */
export interface TranscribedSegment {
  start: number;
  end: number;
  text: string;
}

export interface TranscribeRequest {
  path: string;
  /** Seconds into the file where the clip's source window begins. */
  sourceStart: number;
  /** How much source the clip covers, in seconds (`duration * speed`). */
  window: number;
  /** Whisper language code, or "auto". */
  language: string;
  modelId: string;
}

export async function transcriberStatus(): Promise<TranscriberStatus> {
  return invoke<TranscriberStatus>("transcriber_status");
}

/** Remembers a user-chosen `whisper-cli`. Throws if the path is not a file. */
export async function setTranscriberBinary(path: string): Promise<TranscriberStatus> {
  return invoke<TranscriberStatus>("set_transcriber_binary", { path });
}

/** Downloads one model. Resolves when the file is complete and renamed. */
export async function downloadTranscriberModel(id: string): Promise<void> {
  return invoke<void>("download_transcriber_model", { id });
}

export async function cancelModelDownload(): Promise<void> {
  return invoke<void>("cancel_model_download");
}

export async function deleteTranscriberModel(id: string): Promise<void> {
  return invoke<void>("delete_transcriber_model", { id });
}

/** Subscribes to model download progress. Resolves to an unsubscribe function. */
export async function onTranscriberDownload(
  handler: (progress: TranscriberDownload) => void,
): Promise<UnlistenFn> {
  return listen<TranscriberDownload>("transcriber://download", (event) => handler(event.payload));
}

/** Transcribes one clip's audio window. Runs until done or cancelled. */
export async function transcribeClip(
  request: TranscribeRequest,
): Promise<TranscribedSegment[]> {
  return invoke<TranscribedSegment[]>("transcribe_clip", { request });
}

/** Kills the running transcription, if any. */
export async function cancelTranscribe(): Promise<void> {
  return invoke<void>("cancel_transcribe");
}

// ── text to speech ─────────────────────────────────────────────────────────

/** One Kokoro bundle, as the settings panel sees it. */
export interface TtsModel {
  id: string;
  label: string;
  blurb: string;
  /** Approximate download size in bytes, for display. */
  sizeBytes: number;
  downloaded: boolean;
}

/**
 * One Kokoro speaker. The name encodes accent and gender in its prefix -
 * `af` American female, `bm` British male, `zf` Chinese female - which the
 * speech dialog decodes for display.
 */
export interface TtsVoice {
  /** Kokoro speaker id, what `speakText` wants back. */
  id: number;
  name: string;
}

export interface TtsStatus {
  /** Where models are stored on disk. */
  modelsDir: string;
  models: TtsModel[];
  voices: TtsVoice[];
}

/** Progress of a model download, via `tts://download`. */
export interface TtsDownload {
  id: string;
  received: number;
  total: number;
  /** True while the archive unpacks - bytes stop moving, the job continues. */
  unpacking: boolean;
  done: boolean;
}

export interface SpeakRequest {
  modelId: string;
  /** Kokoro speaker id, from the status' voices table. */
  voice: number;
  text: string;
  /** Speaking rate; 1.0 is the voice's natural pace. */
  speed: number;
  /** The project folder the WAV should land in. */
  project: string;
}

export interface SpeakResult {
  /** Absolute path of the written WAV, ready for the media import path. */
  path: string;
  /** Seconds of audio. */
  duration: number;
}

export async function ttsStatus(): Promise<TtsStatus> {
  return invoke<TtsStatus>("tts_status");
}

/** Downloads one voice model. Resolves once unpacked into place. */
export async function downloadTtsModel(id: string): Promise<void> {
  return invoke<void>("download_tts_model", { id });
}

export async function cancelTtsModelDownload(): Promise<void> {
  return invoke<void>("cancel_tts_model_download");
}

export async function deleteTtsModel(id: string): Promise<void> {
  return invoke<void>("delete_tts_model", { id });
}

/** Subscribes to voice model download progress. Resolves to an unsubscribe function. */
export async function onTtsDownload(
  handler: (progress: TtsDownload) => void,
): Promise<UnlistenFn> {
  return listen<TtsDownload>("tts://download", (event) => handler(event.payload));
}

/** Synthesizes narration into `<project>/audio/` and returns the WAV's path. */
export async function speakText(request: SpeakRequest): Promise<SpeakResult> {
  return invoke<SpeakResult>("speak_text", { request });
}

/** Asks the running synthesis to stop at the next sentence boundary. */
export async function cancelSpeak(): Promise<void> {
  return invoke<void>("cancel_speak");
}

/** Subscribes to synthesis progress (0..1). Resolves to an unsubscribe function. */
export async function onSpeakProgress(
  handler: (progress: { fraction: number }) => void,
): Promise<UnlistenFn> {
  return listen<{ fraction: number }>("tts://progress", (event) => handler(event.payload));
}
