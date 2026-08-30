import { convertFileSrc } from "@tauri-apps/api/core";

import { mediaEndpoint } from "./engine";

/**
 * Where preview media comes from.
 *
 * Everywhere but Linux that is the asset protocol, same as any other file the
 * webview reads. WebKitGTK cannot play `<video>` from a custom URI scheme - it
 * hands the URL to GStreamer, which has no source for `asset://`, so the
 * element fails with MEDIA_ERR_SRC_NOT_SUPPORTED before a byte is read. There
 * the host runs a loopback HTTP server instead and this module addresses it.
 *
 * `initMediaSource` resolves before the first render, so building a URL stays
 * synchronous at every call site.
 */

let base: string | null = null;

/** Asks the host where media should be fetched from. Safe to call twice. */
export async function initMediaSource(): Promise<void> {
  try {
    const endpoint = await mediaEndpoint();
    base = endpoint
      ? `http://127.0.0.1:${endpoint.port}/media?t=${encodeURIComponent(endpoint.token)}&path=`
      : null;
  } catch {
    // An older host, or a server that could not bind: the asset protocol is
    // still correct everywhere it works, so fall back rather than fail.
    base = null;
  }
}

/** A URL the `<video>` element can actually load. */
export function mediaSrc(path: string): string {
  return base ? base + encodeURIComponent(path) : convertFileSrc(path);
}
