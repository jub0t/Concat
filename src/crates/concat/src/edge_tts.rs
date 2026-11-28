// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Native Rust Microsoft Edge Read Aloud neural TTS client.
//!
//! Speaks Edge's WebSocket synthesis protocol directly over TLS (`native-tls`),
//! returning MP3 audio bytes together with per-word `WordBoundary` timestamps
//! for karaoke subtitles — with zero Python or external backend dependency.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use native_tls::TlsConnector;

const EDGE_HOST: &str = "speech.platform.bing.com";
const EDGE_PORT: u16 = 443;
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const SEC_MS_GEC_VERSION: &str = "2-143.0.3650.75";
const CHROMIUM_MAJOR_VERSION: &str = "143";
const WIN_EPOCH_SECS: i64 = 11_644_473_600;
const AUDIO_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";
const MAX_CHUNK_BYTES: usize = 1_800;

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// A single spoken word's timing returned by Edge TTS `WordBoundary` metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct WordTiming {
    /// Start time in seconds relative to the start of the voiceover clip.
    pub start: f64,
    /// Duration in seconds.
    pub duration: f64,
    /// Word text as spoken/reported by Edge TTS.
    pub text: String,
}

/// Complete synthesis result for one scene.
#[derive(Clone, Debug)]
pub struct SynthesisResult {
    /// Raw MP3 bytes (`audio-24khz-48kbitrate-mono-mp3`).
    pub bytes: Vec<u8>,
    /// Estimated duration in seconds (from word boundaries or 48 kbps bitrate fallback).
    pub duration_seconds: f64,
    /// Word-level timing cues for dynamic karaoke subtitles.
    pub word_timings: Vec<WordTiming>,
}

/// Synthesizes `text` using Microsoft Edge neural TTS (`voice`, e.g.
/// `"vi-VN-HoaiMyNeural"`, and `rate`, e.g. `"+15%"`), returning MP3 audio
/// bytes and word boundary timings.
pub fn synthesize(text: &str, voice: &str, rate: &str) -> Result<SynthesisResult, String> {
    let cleaned = sanitize_text(text);
    if cleaned.is_empty() {
        return Err("Cannot synthesize empty scene text.".to_owned());
    }

    let chunks = split_text_chunks(&cleaned, MAX_CHUNK_BYTES);
    let mut all_bytes = Vec::new();
    let mut all_words = Vec::new();
    let mut offset_secs = 0.0f64;

    for chunk in chunks {
        let part = synthesize_chunk_with_retries(&chunk, voice, rate)?;
        for mut word in part.word_timings {
            word.start += offset_secs;
            all_words.push(word);
        }
        all_bytes.extend_from_slice(&part.bytes);
        offset_secs += part.duration_seconds;
    }

    if all_bytes.is_empty() {
        return Err("Edge TTS returned empty audio.".to_owned());
    }

    Ok(SynthesisResult {
        bytes: all_bytes,
        duration_seconds: offset_secs,
        word_timings: all_words,
    })
}

fn synthesize_chunk_with_retries(
    text: &str,
    voice: &str,
    rate: &str,
) -> Result<SynthesisResult, String> {
    let mut clock_skew = 0.0f64;
    let mut last_error = String::new();

    for attempt in 0..3 {
        match synthesize_once(text, voice, rate, clock_skew) {
            Ok(result) if !result.bytes.is_empty() => return Ok(result),
            Ok(_) => {
                last_error = "Edge TTS returned empty audio.".to_owned();
            }
            Err(AttemptError::ClockSkew(new_skew, msg)) => {
                clock_skew = new_skew;
                last_error = msg;
                continue;
            }
            Err(AttemptError::Other(msg)) => {
                last_error = msg;
            }
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(650 * (attempt as u64 + 1)));
        }
    }

    Err(format!(
        "Native Edge TTS synthesis failed after 3 attempts: {last_error}"
    ))
}

enum AttemptError {
    ClockSkew(f64, String),
    Other(String),
}

fn synthesize_once(
    text: &str,
    voice: &str,
    rate: &str,
    clock_skew_secs: f64,
) -> Result<SynthesisResult, AttemptError> {
    let conn_id = random_hex_16();
    let gec = generate_sec_ms_gec(unix_now_secs() + clock_skew_secs);
    let path = format!(
        "/consumer/speech/synthesize/readaloud/edge/v1?TrustedClientToken={TRUSTED_CLIENT_TOKEN}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}&ConnectionId={conn_id}"
    );

    let mut ws = WsClient::connect(EDGE_HOST, EDGE_PORT, &path)?;

    let ts = format_edge_timestamp(unix_now_secs() + clock_skew_secs);
    let config_msg = format!(
        "X-Timestamp:{ts}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"true\"}},\"outputFormat\":\"{AUDIO_FORMAT}\"}}}}}}}}\r\n"
    );
    ws.send_text(&config_msg).map_err(AttemptError::Other)?;

    let req_id = random_hex_16();
    let escaped = escape_xml(text);
    let ssml_msg = format!(
        "X-RequestId:{req_id}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{ts}Z\r\nPath:ssml\r\n\r\n\
         <speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{voice}'><prosody pitch='+0Hz' rate='{rate}' volume='+0%'>{escaped}</prosody></voice></speak>"
    );
    ws.send_text(&ssml_msg).map_err(AttemptError::Other)?;

    let mut audio_bytes = Vec::new();
    let mut word_timings = Vec::new();

    loop {
        match ws.read_message().map_err(AttemptError::Other)? {
            WsMessage::Text(payload) => {
                let (headers, body) = split_edge_text_frame(&payload);
                if has_edge_path(headers, "audio.metadata") {
                    parse_word_boundaries(body, &mut word_timings);
                } else if has_edge_path(headers, "turn.end") {
                    break;
                }
            }
            WsMessage::Binary(payload) => {
                if payload.len() >= 2 {
                    let header_len = u16::from_be_bytes([payload[0], payload[1]]) as usize;
                    if 2 + header_len <= payload.len() {
                        let headers = std::str::from_utf8(&payload[2..2 + header_len])
                            .unwrap_or_default();
                        if has_edge_path(headers, "audio") {
                            audio_bytes.extend_from_slice(&payload[2 + header_len..]);
                        }
                    }
                }
            }
            WsMessage::Close => break,
        }
    }

    let _ = ws.send_close();

    // Estimate duration: prefer the last WordBoundary end time when present,
    // otherwise fall back to 48 kbps MP3 bitrate (6000 bytes/sec).
    let by_words = word_timings
        .last()
        .map(|w| w.start + w.duration)
        .unwrap_or(0.0);
    let by_bytes = audio_bytes.len() as f64 / 6_000.0;
    let duration_seconds = if by_words > 0.0 {
        by_words.max(by_bytes * 0.85)
    } else {
        by_bytes
    };

    Ok(SynthesisResult {
        bytes: audio_bytes,
        duration_seconds,
        word_timings,
    })
}

/// Computes Microsoft Edge's v2 `Sec-MS-GEC` security token (`<16 hex>-<64 hex>`)
/// for the given Unix timestamp in seconds.
pub fn generate_sec_ms_gec(unix_seconds: f64) -> String {
    let total_secs = (unix_seconds.floor() as i64 + WIN_EPOCH_SECS).max(0) as u128;
    let quantized_secs = total_secs - (total_secs % 300);
    let ticks_100ns = quantized_secs * 10_000_000;
    let input = format!("{ticks_100ns}{TRUSTED_CLIENT_TOKEN}");
    let digest = v2_sha256_digest(input.as_bytes());
    let (cs_a, cs_b) = v2_cs64_digest(input.as_bytes(), &digest);
    let mut cs_bytes = [0u8; 8];
    cs_bytes[..4].copy_from_slice(&cs_a.to_le_bytes());
    cs_bytes[4..].copy_from_slice(&cs_b.to_le_bytes());

    let mut out = String::with_capacity(81);
    for b in cs_bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02X}");
    }
    out.push('-');
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02X}");
    }
    out
}

const SHA256_INITIAL: [u32; 8] = [
    0x6A09_E667, 0xBB67_AE85, 0x3C6E_F372, 0xA54F_F53A,
    0x510E_527F, 0x9B05_688C, 0x1F83_D9AB, 0x5BE0_CD19,
];
const SHA256_STATE_V2: [u32; 8] = [
    0xFD9F_BEA2, 0x9124_3CE7, 0xBE49_9094, 0xCD2E_1E0A,
    0x6C00_6F49, 0x04BD_35AB, 0xFEB0_A7E4, 0x03EF_FBD2,
];
const SHA256_SCHEDULE_V2: [u32; 14] = [
    0xEB24_BF9E, 0x1558_691F, 0xA0C0_9EE2, 0x0CFA_32BA,
    0xA622_EE8C, 0xD1DD_9491, 0xC0DD_25A0, 0xF9E7_0AB6,
    0xF0C3_1573, 0x7058_CE4E, 0x7188_E606, 0x00E5_AB33,
    0xD258_B060, 0x7276_6564,
];
const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428A_2F98, 0x7137_4491, 0xB5C0_FBCF, 0xE9B5_DBA5, 0x3956_C25B, 0x59F1_11F1, 0x923F_82A4, 0xAB1C_5ED5,
    0xD807_AA98, 0x1283_5B01, 0x2431_85BE, 0x550C_7DC3, 0x72BE_5D74, 0x80DE_B1FE, 0x9BDC_06A7, 0xC19B_F174,
    0xE49B_69C1, 0xEFBE_4786, 0x0FC1_9DC6, 0x240C_A1CC, 0x2DE9_2C6F, 0x4A74_84AA, 0x5CB0_A9DC, 0x76F9_88DA,
    0x983E_5152, 0xA831_C66D, 0xB003_27C8, 0xBF59_7FC7, 0xC6E0_0BF3, 0xD5A7_9147, 0x06CA_6351, 0x1429_2967,
    0x27B7_0A85, 0x2E1B_2138, 0x4D2C_6DFC, 0x5338_0D13, 0x650A_7354, 0x766A_0ABB, 0x81C2_C92E, 0x9272_2C85,
    0xA2BF_E8A1, 0xA81A_664B, 0xC24B_8B70, 0xC76C_51A3, 0xD192_E819, 0xD699_0624, 0xF40E_3585, 0x106A_A070,
    0x19A4_C116, 0x1E37_6C08, 0x2748_774C, 0x34B0_BCB5, 0x391C_0CB3, 0x4ED8_AA4A, 0x5B9C_CA4F, 0x682E_6FF3,
    0x748F_82EE, 0x78A5_636F, 0x84C8_7814, 0x8CC7_0208, 0x90BE_FFFA, 0xA450_6CEB, 0xBEF9_A3F7, 0xC671_78F2,
];
const SHA256_ADJUSTMENT: u32 = 0xC9DC_06A7;
const SHA256_SIGMA0_ADJ: u32 = 0x059C_0B80;
const SHA256_SIGMA1_ADJ: u32 = 0x000B_92C0;

const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E_91B1, 0xB327_00D0),
    (0x44DE_24C6, 0x88DF_F51D),
    (0x3BEB_65BC, 0x2951_4038),
    (0xC89C_5933, 0xB328_CBDF),
    (0x8D1E_C211, 0xD405_B4D5),
    (0x314E_F5D5, 0x26E5_5F3C),
    (0xE924_91B1, 0x6078_01C6),
    (0xF0A5_B9D5, 0x82B2_C1FB),
    (0x0606_4CF3, 0xFA51_A3DA),
    (0xDA79_FA60, 0x207C_C464),
    (0xBED2_0560, 0x3C3C_74F3),
    (0x89F9_C9F8, 0xD2D1_4B61),
    (0x35FA_38C6, 0xBB04_A2C1),
    (0xCB49_E3C6, 0xC562_6167),
];
const CS64_FACTOR: u32 = 0x8289_3933;
const CS64_ADJ0: u32 = 0xF2F9_0B1A;
const CS64_ADJ1: u32 = 0x0B1A_0000;

#[inline]
fn sigma0(v: u32) -> u32 {
    v.rotate_right(7) ^ v.rotate_right(18) ^ (v >> 3)
}

#[inline]
fn sigma1(v: u32) -> u32 {
    v.rotate_right(17) ^ v.rotate_right(19) ^ (v >> 10)
}

fn expand_sha256_schedule(words: &mut [u32; 64], first_word: usize) {
    for i in first_word..64 {
        words[i] = sigma1(words[i - 2])
            .wrapping_add(words[i - 7])
            .wrapping_add(sigma0(words[i - 15]))
            .wrapping_add(words[i - 16]);
    }
}

fn sha256_rounds(state: [u32; 8], words: &[u32; 64], first_round: usize) -> [u32; 8] {
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
    for i in first_round..64 {
        let k = if first_round == 14 && i == 14 {
            SHA256_ADJUSTMENT
        } else {
            SHA256_ROUND_CONSTANTS[i]
        };
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = g ^ (e & (f ^ g));
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(k)
            .wrapping_add(words[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) | (c & (a | b));
        let t2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    [a, b, c, d, e, f, g, h]
}

fn v2_sha256_init(prefix7: &[u8]) -> [u32; 8] {
    let mut words = [0u32; 64];
    words[14] = u32::from_be_bytes([0, prefix7[0], prefix7[1], prefix7[2]]);
    words[15] = u32::from_be_bytes([prefix7[3], prefix7[4], prefix7[5], prefix7[6]]);
    for i in 16..30 {
        let mut val = SHA256_SCHEDULE_V2[i - 16];
        if i - 2 >= 14 {
            let s1 = if i == 16 {
                sigma1(words[i - 2]) ^ SHA256_SIGMA1_ADJ
            } else {
                sigma1(words[i - 2])
            };
            val = val.wrapping_add(s1);
        }
        if i - 7 >= 14 {
            val = val.wrapping_add(words[i - 7]);
        }
        if i - 15 >= 14 {
            let s0 = if i == 29 {
                sigma0(words[i - 15]) ^ SHA256_SIGMA0_ADJ
            } else {
                sigma0(words[i - 15])
            };
            val = val.wrapping_add(s0);
        }
        words[i] = val;
    }
    words[30] = sigma1(words[28])
        .wrapping_add(words[23])
        .wrapping_add(sigma0(words[15]))
        .wrapping_add(words[14])
        .wrapping_add(SHA256_ADJUSTMENT)
        .wrapping_sub(SHA256_ROUND_CONSTANTS[14]);
    expand_sha256_schedule(&mut words, 31);
    let working = sha256_rounds(SHA256_STATE_V2, &words, 14);
    let mut out = [0u32; 8];
    for i in 0..8 {
        out[i] = SHA256_INITIAL[i].wrapping_add(working[i]);
    }
    out
}

fn v2_sha256_digest(data: &[u8]) -> [u8; 32] {
    let total_len = 57usize + data.len();
    let pad_zeros = (55usize.wrapping_sub(total_len)) % 64;
    let mut padded = Vec::with_capacity(data.len().saturating_sub(7) + 1 + pad_zeros + 8);
    padded.extend_from_slice(&data[7..]);
    padded.push(0x80);
    padded.resize(padded.len() + pad_zeros, 0);
    padded.extend_from_slice(&((total_len as u64) * 8).to_be_bytes());

    let mut state = v2_sha256_init(&data[..7]);
    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for i in 0..16 {
            words[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        expand_sha256_schedule(&mut words, 16);
        let working = sha256_rounds(state, &words, 0);
        for i in 0..8 {
            state[i] = state[i].wrapping_add(working[i]);
        }
    }

    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn v2_cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let key_a = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) | 1;
    let key_b = u32::from_le_bytes([digest[4], digest[5], digest[6], digest[7]]) | 1;
    let factors = [
        key_a.wrapping_mul(CS64_FACTOR),
        key_b.wrapping_mul(CS64_FACTOR),
    ];

    let mut terms = Vec::with_capacity(32);
    let mut offsets = Vec::with_capacity(32);
    for (i, &(coeff, off)) in CS64_COEFFICIENTS.iter().enumerate() {
        terms.push(coeff.wrapping_mul(factors[i & 1]));
        offsets.push(off);
    }

    // Split b"\x00" + data into little-endian u32 words, zero-padded to an even count.
    let mut prefixed = Vec::with_capacity(1 + data.len() + 7);
    prefixed.push(0u8);
    prefixed.extend_from_slice(data);
    while prefixed.len() % 4 != 0 {
        prefixed.push(0u8);
    }
    let mut le_words: Vec<u32> = prefixed
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if le_words.len() % 2 != 0 {
        le_words.push(0);
    }

    for (i, &word) in le_words.iter().enumerate() {
        let mut term = word.wrapping_mul(if i % 2 == 0 { key_a } else { key_b });
        let mut off = word
            .rotate_left(16)
            .wrapping_mul(if i % 2 == 0 { 0xEF05_69FB } else { 0xC317_13DB });
        if i == 0 {
            term = term.wrapping_add(CS64_ADJ0.wrapping_mul(factors[0]));
            off = off.wrapping_add(CS64_ADJ1);
        }
        terms.push(term);
        offsets.push(off);
    }

    let mut swap_acc = 0u32;
    let mut swap_last = 0u32;
    let mut state = 0u32;
    let mut acc = 0u32;

    for i in (0..terms.len()).step_by(2) {
        let x = terms[i];
        let y = terms[i + 1];

        let t0 = offsets[i].wrapping_add(x).rotate_left(16);
        let q0 = t0.wrapping_mul(0x76F5_0F34);
        swap_acc = swap_acc.wrapping_add(q0.rotate_left(16).wrapping_mul(0xC3EF_EA97).wrapping_add(q0));

        let t1 = offsets[i + 1].wrapping_add(y).rotate_left(16);
        let q1 = t1.wrapping_mul(0x1881_CE3C);
        swap_last = q1.rotate_left(16).wrapping_mul(0x35BD_1EC9).wrapping_add(q1);
        swap_acc = swap_acc.wrapping_add(swap_last);

        state = state.wrapping_mul(key_a).wrapping_add(x);
        state = state.rotate_left(16).wrapping_mul(0xCF98_B111);
        state = state.rotate_left(16).wrapping_mul(0x8708_5B9F);
        state = state.rotate_left(16).wrapping_mul(0x12CE_B96D);
        state = state.rotate_left(16).wrapping_mul(0x257E_1D83);
        acc = acc.wrapping_add(state);

        let mut t = state.wrapping_mul(key_b).wrapping_add(y);
        t = t.rotate_left(16).wrapping_mul(0xA274_16F5);
        t = t.rotate_left(16).wrapping_mul(0xD383_96FF);
        state = t
            .rotate_left(16)
            .wrapping_mul(0x7C93_2B89)
            .rotate_left(16)
            .wrapping_mul(0xBFA4_9F69);
        acc = acc.wrapping_add(state);
    }

    (swap_last ^ state, swap_acc ^ acc)
}

fn unix_now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn format_edge_timestamp(unix_seconds: f64) -> String {
    let secs = unix_seconds.floor() as i64;
    let (year, month, day, hour, minute, second, weekday) = civil_from_unix(secs);
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let dow = DAYS[weekday as usize % 7];
    let mon = MONTHS[(month.saturating_sub(1) as usize).min(11)];
    format!("{dow} {mon} {day:02} {year:04} {hour:02}:{minute:02}:{second:02} GMT+0000 (Coordinated Universal Time)")
}

/// Howard Hinnant's civil-from-days algorithm: Unix timestamp -> UTC components + weekday (0=Sun).
fn civil_from_unix(unix_secs: i64) -> (i32, u32, u32, u32, u32, u32, u32) {
    let days = unix_secs.div_euclid(86_400);
    let sod = unix_secs.rem_euclid(86_400) as u32;
    let hour = sod / 3_600;
    let minute = (sod % 3_600) / 60;
    let second = sod % 60;
    let weekday = (days + 4).rem_euclid(7) as u32;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = (y + if m <= 2 { 1 } else { 0 }) as i32;
    (year, m, d, hour, minute, second, weekday)
}

/// Parses an HTTP `Date` header (RFC 2822 / IMF-fixdate, e.g.
/// `Fri, 27 Nov 2026 14:35:00 GMT`) into Unix seconds.
fn parse_http_date(raw: &str) -> Option<f64> {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    let day: u32 = parts[1].parse().ok()?;
    let month: u32 = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i32 = parts[3].parse().ok()?;
    let mut hms = parts[4].split(':');
    let hour: u32 = hms.next()?.parse().ok()?;
    let min: u32 = hms.next()?.parse().ok()?;
    let sec: u32 = hms.next()?.parse().ok()?;

    let days = days_from_civil(year, month, day);
    Some((days * 86_400 + i64::from(hour) * 3_600 + i64::from(min) * 60 + i64::from(sec)) as f64)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - if month <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u64;
    let m = u64::from(month);
    let d = u64::from(day);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

fn random_bytes_16() -> [u8; 16] {
    let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();

    let mut h1 = DefaultHasher::new();
    (now, seq, pid, 0x9E37_79B9_7F4A_7C15u64).hash(&mut h1);
    let a = h1.finish().to_le_bytes();

    let mut h2 = DefaultHasher::new();
    (seq.wrapping_mul(6364136223846793005), now, pid, a).hash(&mut h2);
    let b = h2.finish().to_le_bytes();

    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a);
    out[8..].copy_from_slice(&b);
    out
}

fn random_hex_16() -> String {
    let bytes = random_bytes_16();
    let mut hex = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

fn sanitize_text(raw: &str) -> String {
    raw.chars()
        .filter(|&c| {
            !matches!(
                c as u32,
                0x00..=0x08 | 0x0B..=0x0C | 0x0E..=0x1F
            )
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn split_text_chunks(text: &str, max_bytes: usize) -> Vec<String> {
    if text.len() <= max_bytes {
        return vec![text.to_owned()];
    }
    let mut chunks = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if rest.len() <= max_bytes {
            chunks.push(rest.to_owned());
            break;
        }
        let mut end = max_bytes.min(rest.len());
        while end > 0 && !rest.is_char_boundary(end) {
            end -= 1;
        }
        let slice = &rest[..end];
        let split_at = slice
            .rfind(['.', '!', '?', '\n'])
            .map(|i| i + 1)
            .filter(|&i| i > end / 3)
            .or_else(|| slice.rfind(char::is_whitespace).filter(|&i| i > 0))
            .unwrap_or(end);
        let piece = rest[..split_at].trim();
        if !piece.is_empty() {
            chunks.push(piece.to_owned());
        }
        rest = rest[split_at..].trim_start();
    }
    chunks
}

fn split_edge_text_frame(payload: &str) -> (&str, &str) {
    payload
        .split_once("\r\n\r\n")
        .unwrap_or((payload, ""))
}

fn has_edge_path(headers: &str, expected: &str) -> bool {
    headers.lines().any(|line| {
        line.trim()
            .strip_prefix("Path:")
            .is_some_and(|val| val.trim().eq_ignore_ascii_case(expected))
    })
}

fn parse_word_boundaries(body: &str, out: &mut Vec<WordTiming>) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return;
    };
    let Some(items) = json.get("Metadata").and_then(serde_json::Value::as_array) else {
        return;
    };
    for item in items {
        let kind = item
            .get("Type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if kind != "WordBoundary" {
            continue;
        }
        let Some(data) = item.get("Data") else {
            continue;
        };
        let offset = data
            .get("Offset")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let duration = data
            .get("Duration")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let text = data
            .get("text")
            .and_then(|t| t.get("Text"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        out.push(WordTiming {
            start: offset as f64 / 10_000_000.0,
            duration: (duration as f64 / 10_000_000.0).max(0.05),
            text: text.to_owned(),
        });
    }
}

// ---------------------------------------------------------------------------
// Minimal synchronous RFC 6455 WebSocket client over native-tls
// ---------------------------------------------------------------------------

enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Close,
}

struct WsClient {
    stream: native_tls::TlsStream<TcpStream>,
    read_buf: Vec<u8>,
}

impl WsClient {
    fn connect(host: &str, port: u16, path: &str) -> Result<Self, AttemptError> {
        let addr = format!("{host}:{port}")
            .to_socket_addrs()
            .map_err(|e| AttemptError::Other(format!("DNS lookup failed for {host}: {e}")))?
            .next()
            .ok_or_else(|| AttemptError::Other(format!("No IP address resolved for {host}")))?;

        let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(12))
            .map_err(|e| AttemptError::Other(format!("Could not connect to Edge TTS: {e}")))?;
        let _ = tcp.set_read_timeout(Some(Duration::from_secs(45)));
        let _ = tcp.set_write_timeout(Some(Duration::from_secs(15)));

        let connector = TlsConnector::new()
            .map_err(|e| AttemptError::Other(format!("TLS initialization failed: {e}")))?;
        let mut stream = connector
            .connect(host, tcp)
            .map_err(|e| AttemptError::Other(format!("TLS handshake failed: {e}")))?;

        let ws_key = base64::engine::general_purpose::STANDARD.encode(random_bytes_16());
        let muid = random_hex_16().to_ascii_uppercase();
        let request = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {ws_key}\r\n\
             Sec-WebSocket-Version: 13\r\n\
             Pragma: no-cache\r\n\
             Cache-Control: no-cache\r\n\
             Origin: chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold\r\n\
             Cookie: muid={muid};\r\n\
             User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{CHROMIUM_MAJOR_VERSION}.0.0.0 Safari/537.36 Edg/{CHROMIUM_MAJOR_VERSION}.0.0.0\r\n\
             Accept-Encoding: identity\r\n\
             Accept-Language: en-US,en;q=0.9\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|e| AttemptError::Other(format!("WebSocket handshake write failed: {e}")))?;
        stream
            .flush()
            .map_err(|e| AttemptError::Other(format!("WebSocket handshake flush failed: {e}")))?;

        // Read HTTP response headers until \r\n\r\n, preserving any trailing WS bytes.
        let mut raw = Vec::with_capacity(1024);
        let mut chunk = [0u8; 512];
        let header_end = loop {
            let n = stream.read(&mut chunk).map_err(|e| {
                AttemptError::Other(format!("WebSocket handshake read failed: {e}"))
            })?;
            if n == 0 {
                return Err(AttemptError::Other(
                    "Edge TTS closed the connection during handshake.".to_owned(),
                ));
            }
            raw.extend_from_slice(&chunk[..n]);
            if let Some(pos) = find_subslice(&raw, b"\r\n\r\n") {
                break pos + 4;
            }
            if raw.len() > 16_384 {
                return Err(AttemptError::Other(
                    "Edge TTS handshake response headers exceeded 16 KiB.".to_owned(),
                ));
            }
        };

        let header_str = String::from_utf8_lossy(&raw[..header_end]);
        let status_line = header_str.lines().next().unwrap_or_default();
        if !status_line.contains(" 101 ") {
            if status_line.contains(" 403 ") {
                for line in header_str.lines() {
                    if let Some((k, v)) = line.split_once(':')
                        && k.trim().eq_ignore_ascii_case("date")
                        && let Some(server_ts) = parse_http_date(v.trim())
                    {
                        let skew = server_ts - unix_now_secs();
                        return Err(AttemptError::ClockSkew(
                            skew,
                            format!("Edge TTS rejected token (403); adjusting clock skew by {skew:.0}s"),
                        ));
                    }
                }
            }
            return Err(AttemptError::Other(format!(
                "Edge TTS WebSocket upgrade rejected: {status_line}"
            )));
        }

        let leftover = raw[header_end..].to_vec();
        Ok(Self {
            stream,
            read_buf: leftover,
        })
    }

    fn send_text(&mut self, text: &str) -> Result<(), String> {
        self.send_frame(0x1, text.as_bytes())
    }

    fn send_pong(&mut self, payload: &[u8]) -> Result<(), String> {
        self.send_frame(0xA, payload)
    }

    fn send_close(&mut self) -> Result<(), String> {
        self.send_frame(0x8, &[])
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let mask_seed = random_bytes_16();
        let mask = [mask_seed[0], mask_seed[1], mask_seed[2], mask_seed[3]];
        let frame = encode_client_frame(opcode, payload, mask);
        self.stream
            .write_all(&frame)
            .map_err(|e| format!("WebSocket write failed: {e}"))?;
        self.stream
            .flush()
            .map_err(|e| format!("WebSocket flush failed: {e}"))
    }

    fn read_exact_buffered(&mut self, out: &mut [u8]) -> Result<(), String> {
        let mut filled = 0usize;
        if !self.read_buf.is_empty() {
            let take = self.read_buf.len().min(out.len());
            out[..take].copy_from_slice(&self.read_buf[..take]);
            self.read_buf.drain(..take);
            filled += take;
        }
        while filled < out.len() {
            let n = self
                .stream
                .read(&mut out[filled..])
                .map_err(|e| format!("WebSocket read failed: {e}"))?;
            if n == 0 {
                return Err("Unexpected EOF while reading Edge TTS stream.".to_owned());
            }
            filled += n;
        }
        Ok(())
    }

    fn read_message(&mut self) -> Result<WsMessage, String> {
        let mut msg_opcode: Option<u8> = None;
        let mut assembled = Vec::new();

        loop {
            let mut hdr = [0u8; 2];
            self.read_exact_buffered(&mut hdr)?;
            let fin = (hdr[0] & 0x80) != 0;
            let opcode = hdr[0] & 0x0F;
            let masked = (hdr[1] & 0x80) != 0;
            let mut len = u64::from(hdr[1] & 0x7F);
            if len == 126 {
                let mut ext = [0u8; 2];
                self.read_exact_buffered(&mut ext)?;
                len = u64::from(u16::from_be_bytes(ext));
            } else if len == 127 {
                let mut ext = [0u8; 8];
                self.read_exact_buffered(&mut ext)?;
                len = u64::from_be_bytes(ext);
            }
            if len > 16 * 1024 * 1024 {
                return Err("Edge TTS frame exceeded 16 MiB limit.".to_owned());
            }
            let mut mask = [0u8; 4];
            if masked {
                self.read_exact_buffered(&mut mask)?;
            }
            let mut payload = vec![0u8; len as usize];
            self.read_exact_buffered(&mut payload)?;
            if masked {
                for (i, byte) in payload.iter_mut().enumerate() {
                    *byte ^= mask[i & 3];
                }
            }

            match opcode {
                0x8 => return Ok(WsMessage::Close),
                0x9 => {
                    let _ = self.send_pong(&payload);
                    continue;
                }
                0xA => continue,
                0x1 | 0x2 => {
                    msg_opcode = Some(opcode);
                    assembled.extend_from_slice(&payload);
                    if fin {
                        break;
                    }
                }
                0x0 => {
                    assembled.extend_from_slice(&payload);
                    if fin {
                        break;
                    }
                }
                _ => continue,
            }
        }

        match msg_opcode {
            Some(0x1) => Ok(WsMessage::Text(
                String::from_utf8_lossy(&assembled).into_owned(),
            )),
            Some(0x2) => Ok(WsMessage::Binary(assembled)),
            _ => Ok(WsMessage::Close),
        }
    }
}

fn encode_client_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | (opcode & 0x0F));
    if payload.len() < 126 {
        frame.push(0x80 | (payload.len() as u8));
    } else if payload.len() <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    for (i, &byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask[i & 3]);
    }
    frame
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_ms_gec_is_deterministic_within_5_minute_window() {
        // 1_700_000_100 is an exact 300-second boundary, so +10s and +150s share a window.
        let a = generate_sec_ms_gec(1_700_000_110.0);
        let b = generate_sec_ms_gec(1_700_000_250.0);
        let c = generate_sec_ms_gec(1_700_000_410.0);
        assert_eq!(
            a,
            "0919602B5750900A-EC9DD394F23CB5635CF62A0E8E24136DCD32B26D10C543B10082E482D7516397"
        );
        assert_eq!(a, b);
        assert_eq!(
            c,
            "73C7C85FE76CF347-452672184D494A9F80E179C2FBFEE2E98480C4D4AAFC1F4A577E43B023BEEEC9"
        );
    }

    #[test]
    fn parses_http_date_and_roundtrips_civil_time() {
        let ts = parse_http_date("Fri, 27 Nov 2026 14:35:00 GMT").expect("valid date");
        let formatted = format_edge_timestamp(ts);
        assert!(formatted.starts_with("Fri Nov 27 2026 14:35:00 GMT+0000"));
    }

    #[test]
    fn escapes_xml_and_splits_long_text_on_word_boundaries() {
        assert_eq!(escape_xml("A & B < C > \"D\" 'E'"), "A &amp; B &lt; C &gt; &quot;D&quot; &apos;E&apos;");
        let long = "Xin chào mọi người. ".repeat(120);
        let chunks = split_text_chunks(&long, 300);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.len() <= 300));
    }

    #[test]
    fn parses_edge_word_boundary_metadata() {
        let sample = r#"{
            "Metadata": [
                {
                    "Type": "WordBoundary",
                    "Data": {
                        "Offset": 500000,
                        "Duration": 3250000,
                        "text": { "Text": "Xin" }
                    }
                },
                {
                    "Type": "WordBoundary",
                    "Data": {
                        "Offset": 4000000,
                        "Duration": 2500000,
                        "text": { "Text": "chào" }
                    }
                }
            ]
        }"#;
        let mut out = Vec::new();
        parse_word_boundaries(sample, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "Xin");
        assert!((out[0].start - 0.05).abs() < 1e-6);
        assert!((out[0].duration - 0.325).abs() < 1e-6);
        assert_eq!(out[1].text, "chào");
        assert!((out[1].start - 0.4).abs() < 1e-6);
    }

    #[test]
    fn encodes_masked_rfc6455_frames() {
        let mask = [0x11, 0x22, 0x33, 0x44];
        let frame = encode_client_frame(0x1, b"hello", mask);
        assert_eq!(frame[0], 0x81);
        assert_eq!(frame[1], 0x85);
        assert_eq!(&frame[2..6], &mask);
        let unmasked: Vec<u8> = frame[6..]
            .iter()
            .enumerate()
            .map(|(i, &b)| b ^ mask[i & 3])
            .collect();
        assert_eq!(unmasked, b"hello");
    }
}
