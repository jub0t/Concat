// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Threads extraction and neural voiceover orchestration.
//!
//! Phase 3 (Complete Decoupling): both public Threads scraping (`threads_scraper`)
//! and Microsoft Edge neural TTS (`edge_tts`) run natively inside Concat in Rust
//! with zero Python / TGS backend requirement. If a user explicitly sets
//! `TGS_API_URL`, legacy TGS extraction remains available as an opt-in fallback.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use crate::edge_tts::{self, WordTiming};
use crate::threads_scraper;

/// Playback-speed presets accepted by the TGS Edge TTS endpoint.
pub const VOICE_RATES: &[(&str, &str)] = &[
    ("1.0×", "+0%"),
    ("1.15×", "+15%"),
    ("1.25×", "+25%"),
    ("1.35×", "+35%"),
];

/// Edge TTS voice presets accepted by TGS (`(voice_id, short_label)`).
pub const VOICES: &[(&str, &str)] = &[
    ("vi-VN-HoaiMyNeural", "Hoài My (VI)"),
    ("vi-VN-NamMinhNeural", "Nam Minh (VI)"),
    ("en-US-AvaNeural", "Ava (EN)"),
    ("en-US-GuyNeural", "Guy (EN)"),
];

/// Returns `(voice_id, label)` for `index`, clamped to [`VOICES`].
pub fn voice_at(index: usize) -> (&'static str, &'static str) {
    VOICES.get(index).copied().unwrap_or(VOICES[0])
}

/// Returns `(label, edge_tts_rate)` for `index`, clamped to [`VOICE_RATES`].
pub fn voice_rate_at(index: usize) -> (&'static str, &'static str) {
    VOICE_RATES
        .get(index)
        .copied()
        .unwrap_or(VOICE_RATES[1])
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommentSort {
    #[default]
    Likes,
    Replies,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub id: String,
    pub label: String,
    pub author: String,
    pub username: String,
    pub timestamp: String,
    pub verified: bool,
    pub avatar_url: Option<String>,
    pub avatar_bytes: Option<Vec<u8>>,
    pub media_url: Option<String>,
    pub media_bytes: Option<Vec<u8>>,
    pub media_is_video: bool,
    pub text: String,
    pub likes: String,
    pub replies: String,
    pub reposts: String,
    pub is_post: bool,
    pub meme_path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extraction {
    pub title: String,
    pub source_url: String,
    pub project_id: String,
    pub post_frame: Frame,
    pub pool_by_likes: Vec<Frame>,
    pub pool_by_replies: Vec<Frame>,
    pub frames: Vec<Frame>,
}

/// A generated voiceover held until the Concat project has a folder to copy it into.
#[derive(Clone, Debug)]
pub struct VoiceoverAudio {
    pub bytes: Vec<u8>,
    pub duration_seconds: f64,
    pub word_timings: Vec<WordTiming>,
}

/// Validates and normalizes a public Threads post, share, or short URL,
/// stripping tracking query strings and fragments before sending to TGS.
pub fn normalize_threads_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Enter a valid Threads post link.".to_owned());
    }

    let without_scheme = if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .or_else(|| trimmed.strip_prefix("HTTPS://"))
        .or_else(|| trimmed.strip_prefix("HTTP://"))
    {
        rest
    } else if trimmed.contains("://") {
        return Err("Only Threads post links (threads.com or threads.net) are supported.".to_owned());
    } else {
        trimmed
    };

    let (host_raw, path_and_query) = without_scheme
        .split_once('/')
        .ok_or_else(|| "Enter a Threads post link, e.g. https://www.threads.com/@name/post/...".to_owned())?;
    let host = host_raw.to_ascii_lowercase();
    let canonical_host = match host.as_str() {
        "threads.net" | "www.threads.net" | "m.threads.net" => "www.threads.net",
        "threads.com" | "www.threads.com" | "m.threads.com" => "www.threads.com",
        _ => {
            return Err("Only Threads post links (threads.com or threads.net) are supported.".to_owned());
        }
    };

    let path = path_and_query
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let is_code = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    let is_username = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    };

    match segments.as_slice() {
        [user, "post", code] if user.starts_with('@') && is_username(&user[1..]) && is_code(code) => {
            Ok(format!("https://{canonical_host}/{user}/post/{code}"))
        }
        ["t", code] if is_code(code) => Ok(format!("https://{canonical_host}/t/{code}")),
        ["share", code] if is_code(code) => Ok(format!("https://{canonical_host}/share/{code}")),
        _ => Err("Link must point to a Threads post (/@name/post/..., /t/..., or /share/...).".to_owned()),
    }
}

/// Selects the initial post frame and up to four top comments from `pool`,
/// prioritizing substantive text over pure URL-only comments while falling back
/// cleanly when fewer comments are available.
pub fn default_frames(post: &Frame, pool: &[Frame]) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(5);
    frames.push(post.clone());

    let mut chosen: Vec<&Frame> = Vec::with_capacity(4);
    for comment in pool {
        if !is_link_only_comment(&comment.text) {
            chosen.push(comment);
            if chosen.len() == 4 {
                break;
            }
        }
    }
    if chosen.len() < 4 {
        for comment in pool {
            if chosen.iter().any(|item| item.id == comment.id) {
                continue;
            }
            chosen.push(comment);
            if chosen.len() == 4 {
                break;
            }
        }
    }

    for (index, comment) in chosen.into_iter().enumerate() {
        let mut frame = comment.clone();
        frame.label = format!("Comment {}", index + 1);
        frames.push(frame);
    }
    frames
}

/// Returns the comment pool for `sort`.
pub fn pool_for(extraction: &Extraction, sort: CommentSort) -> &[Frame] {
    match sort {
        CommentSort::Likes => &extraction.pool_by_likes,
        CommentSort::Replies => &extraction.pool_by_replies,
    }
}

/// Finds the next comment in `sort`'s pool that is not already in `extraction.frames`.
pub fn next_pool_comment(extraction: &Extraction, sort: CommentSort) -> Option<Frame> {
    let used: HashSet<&str> = extraction
        .frames
        .iter()
        .map(|frame| frame.id.as_str())
        .collect();
    pool_for(extraction, sort)
        .iter()
        .find(|candidate| !used.contains(candidate.id.as_str()))
        .cloned()
        .map(|mut frame| {
            let next_num = extraction
                .frames
                .iter()
                .filter(|item| item.label.starts_with("Comment "))
                .count()
                + 1;
            frame.label = format!("Comment {next_num}");
            frame
        })
}

/// Number of unused comments left in `sort`'s pool.
pub fn remaining_pool_count(extraction: &Extraction, sort: CommentSort) -> usize {
    let used: HashSet<&str> = extraction
        .frames
        .iter()
        .map(|frame| frame.id.as_str())
        .collect();
    pool_for(extraction, sort)
        .iter()
        .filter(|candidate| !used.contains(candidate.id.as_str()))
        .count()
}

fn is_link_only_comment(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    !words.is_empty()
        && words
            .iter()
            .all(|word| word.starts_with("http://") || word.starts_with("https://"))
}

/// Extract a post and its top-level comments natively in Rust.
///
/// If the user explicitly sets `TGS_API_URL` in the environment and native
/// scraping fails, falls back to the legacy TGS HTTP service.
pub fn extract(source_url: &str, mut progress: impl FnMut(String)) -> Result<Extraction, String> {
    let normalized_url = normalize_threads_url(source_url)?;
    match threads_scraper::extract(&normalized_url, &mut progress) {
        Ok(extraction) => Ok(extraction),
        Err(native_err) => {
            if let Ok(base) = std::env::var("TGS_API_URL") {
                let base = base.trim().trim_end_matches('/').to_owned();
                if !base.is_empty() {
                    progress("Native scrape unavailable; trying configured TGS_API_URL…".to_owned());
                    return extract_via_tgs(&base, &normalized_url, progress);
                }
            }
            Err(native_err)
        }
    }
}

fn extract_via_tgs(
    base: &str,
    normalized_url: &str,
    mut progress: impl FnMut(String),
) -> Result<Extraction, String> {
    let payload = serde_json::json!({
        "source_url": normalized_url,
        "settings": {
            "max_comments": 30,
            "comment_sort_order": "top",
            "include_replies": false,
            "max_replies_per_comment": 0,
            "max_total_replies": 0
        }
    });
    let created = request_json(
        "POST",
        &format!("{base}/api/v1/extractions"),
        Some(&payload),
    )?;
    let job_id = created
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "TGS did not return an extraction job id".to_owned())?;

    let mut last_stage = String::new();
    let mut terminal = None;
    for _ in 0..120 {
        let job = request_json("GET", &format!("{base}/api/v1/extractions/{job_id}"), None)?;
        let stage = job
            .get("current_step")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Extracting Threads post")
            .to_owned();
        if stage != last_stage {
            last_stage.clone_from(&stage);
            progress(stage);
        }

        let status = job
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if matches!(status, "success" | "partial" | "empty" | "failed") {
            terminal = Some(job);
            break;
        }
        std::thread::sleep(Duration::from_millis(750));
    }

    let job = terminal.ok_or_else(|| {
        "Threads extraction is taking longer than expected. Check the TGS service and try again."
            .to_owned()
    })?;
    let status = job
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if status == "failed" || status == "empty" {
        let detail = job
            .get("errors")
            .and_then(serde_json::Value::as_array)
            .and_then(|errors| errors.first())
            .and_then(serde_json::Value::as_str)
            .unwrap_or("No post data was returned.");
        return Err(detail.to_owned());
    }

    let project_id = job
        .get("project_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "TGS finished extraction without a project id".to_owned())?;
    progress("Preparing the post and top four comments".to_owned());
    let exported = request_json(
        "GET",
        &format!("{base}/api/v1/projects/{project_id}/export"),
        None,
    )?;
    let raw = exported
        .get("raw_data")
        .ok_or_else(|| "The TGS project has no extracted post data".to_owned())?;
    let post = raw
        .get("post")
        .ok_or_else(|| "The TGS project has no post".to_owned())?;
    let comments = raw
        .get("comments")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let curated = exported.get("curated_content");
    let ranking_data = exported.get("ranking_data");

    let post_text = curated
        .and_then(|value| value.get("edited_post_text"))
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| string(post, "text"));
    let mut post_frame = Frame {
        id: string(post, "id"),
        label: "Post".to_owned(),
        author: author(post),
        username: username(post),
        avatar_url: avatar_url(post),
        avatar_bytes: None,
        text: post_text,
        likes: engagement(post, "likes"),
        replies: engagement(post, "replies"),
        reposts: engagement(post, "reposts"),
        is_post: true,
        ..Frame::default()
    };

    let comment_by_id: HashMap<String, serde_json::Value> = comments
        .into_iter()
        .filter(|comment| string(comment, "type") == "comment")
        .map(|comment| (string(&comment, "id"), comment))
        .collect();
    let edited = curated
        .and_then(|value| value.get("edited_texts"))
        .and_then(serde_json::Value::as_object);

    let mut pool_by_likes = build_comment_pool(&comment_by_id, ranking_data, "by_likes", "likes", edited);
    let mut pool_by_replies = build_comment_pool(&comment_by_id, ranking_data, "by_replies", "replies", edited);
    hydrate_avatars(&base, &mut post_frame, &mut pool_by_likes, &mut pool_by_replies);
    let frames = default_frames(&post_frame, &pool_by_likes);

    let title = exported
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("Threads quick video")
        .to_owned();
    let source_url = exported
        .get("source_url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&normalized_url)
        .to_owned();
    Ok(Extraction {
        title,
        source_url,
        project_id: project_id.to_owned(),
        post_frame,
        pool_by_likes,
        pool_by_replies,
        frames,
    })
}

fn build_comment_pool(
    comment_by_id: &HashMap<String, serde_json::Value>,
    ranking_data: Option<&serde_json::Value>,
    ranking_section: &str,
    metric_key: &str,
    edited: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<Frame> {
    let ranking = ranking_data
        .and_then(|data| data.get(ranking_section))
        .and_then(|section| section.get("items"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut ranked_ids: Vec<(i64, String)> = ranking
        .iter()
        .filter_map(|item| {
            Some((
                item.get("rank")?.as_i64().unwrap_or(i64::MAX),
                item.get("comment_id")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    ranked_ids.sort_by_key(|entry| entry.0);

    let mut ordered: Vec<serde_json::Value> = ranked_ids
        .into_iter()
        .filter_map(|(_, id)| comment_by_id.get(&id).cloned())
        .collect();

    let mut remaining: Vec<serde_json::Value> = comment_by_id
        .values()
        .filter(|comment| {
            let id = string(comment, "id");
            !ordered.iter().any(|existing| string(existing, "id") == id)
        })
        .cloned()
        .collect();
    remaining.sort_by_key(|comment| {
        std::cmp::Reverse(
            comment
                .get("engagement")
                .and_then(|item| item.get(metric_key))
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(-1),
        )
    });
    ordered.extend(remaining);

    ordered
        .into_iter()
        .enumerate()
        .filter_map(|(index, comment)| {
            let id = string(&comment, "id");
            let text = edited
                .and_then(|values| values.get(&id))
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| string(&comment, "text"));
            if text.trim().is_empty() {
                return None;
            }
            Some(Frame {
                id,
                label: format!("Comment {}", index + 1),
                author: author(&comment),
                username: username(&comment),
                avatar_url: avatar_url(&comment),
                avatar_bytes: None,
                text,
                likes: engagement(&comment, "likes"),
                replies: engagement(&comment, "replies"),
                reposts: comment_reposts(&comment),
                is_post: false,
                ..Frame::default()
            })
        })
        .collect()
}

/// Generates one voiceover per requested `(frame_index, Frame)` natively via
/// Concat's built-in Microsoft Edge neural TTS client using `voice` (e.g.
/// `"vi-VN-HoaiMyNeural"`) and `rate` (e.g. `"+15%"`), returning
/// `(frame_index, audio)` with `WordBoundary` timings for karaoke subtitles.
pub fn generate_voiceovers(
    _project_id: &str,
    frames: &[(usize, Frame)],
    voice: &str,
    rate: &str,
    mut progress: impl FnMut(String),
) -> Result<Vec<(usize, VoiceoverAudio)>, String> {
    let (voice_id, voice_label) = VOICES
        .iter()
        .copied()
        .find(|&(id, _)| id == voice)
        .unwrap_or(VOICES[0]);
    let rate = if VOICE_RATES.iter().any(|&(_, value)| value == rate) {
        rate
    } else {
        "+0%"
    };
    let total = frames.len();
    let mut audio = Vec::with_capacity(total);

    for (step, (index, frame)) in frames.iter().enumerate() {
        progress(format!(
            "Creating {voice_label} voice {}/{}",
            step + 1,
            total
        ));
        let text = if frame.text.trim().is_empty() {
            format!("{} — {}", frame.label, frame.author)
        } else {
            frame.text.trim().to_owned()
        };
        let synth = edge_tts::synthesize(&text, voice_id, rate)?;
        audio.push((
            *index,
            VoiceoverAudio {
                bytes: synth.bytes,
                duration_seconds: synth.duration_seconds,
                word_timings: synth.word_timings,
            },
        ));
    }

    Ok(audio)
}

fn request_json(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(120))
        .build();
    let response = match (method, body) {
        ("POST", Some(body)) => {
            let serialized = serde_json::to_string(body).map_err(|error| error.to_string())?;
            agent
                .post(url)
                .set("Content-Type", "application/json")
                .send_string(&serialized)
        }
        _ => agent.get(url).call(),
    };
    let response = response.map_err(|error| service_error(error, url))?;
    let body = response
        .into_string()
        .map_err(|error| format!("Could not read the TGS response: {error}"))?;
    serde_json::from_str(&body).map_err(|error| format!("TGS returned invalid JSON: {error}"))
}

fn service_error(error: ureq::Error, url: &str) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let detail = response.into_string().unwrap_or_default();
            format!("TGS service returned HTTP {code}: {}", detail.trim())
        }
        ureq::Error::Transport(error) => format!(
            "Could not reach the TGS service at {}. Start the TGS Python backend and retry. ({error})",
            url.split("/api/").next().unwrap_or(url)
        ),
    }
}

fn string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn author(value: &serde_json::Value) -> String {
    value
        .get("author")
        .and_then(|author| {
            author
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.is_empty())
                .or_else(|| author.get("username").and_then(serde_json::Value::as_str))
        })
        .filter(|name| !name.is_empty())
        .unwrap_or("Unknown author")
        .to_owned()
}

fn engagement(value: &serde_json::Value, key: &str) -> String {
    value
        .get("engagement")
        .and_then(|engagement| engagement.get(key))
        .and_then(serde_json::Value::as_i64)
        .filter(|&count| count > 0)
        .map(format_compact_count)
        .unwrap_or_default()
}

fn comment_reposts(value: &serde_json::Value) -> String {
    let primary = engagement(value, "reposts");
    if primary.is_empty() {
        engagement(value, "shares")
    } else {
        primary
    }
}

pub(crate) fn format_compact_count(count: i64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 10_000 {
        format!("{}K", count / 1_000)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

fn username(value: &serde_json::Value) -> String {
    value
        .get("author")
        .and_then(|author| author.get("username"))
        .and_then(serde_json::Value::as_str)
        .map(|name| name.trim().trim_start_matches('@').to_owned())
        .unwrap_or_default()
}

fn avatar_url(value: &serde_json::Value) -> Option<String> {
    value
        .get("author")
        .and_then(|author| author.get("avatar_url"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

fn hydrate_avatars(
    base: &str,
    post: &mut Frame,
    pool_likes: &mut [Frame],
    pool_replies: &mut [Frame],
) {
    let mut cache: HashMap<String, Option<Vec<u8>>> = HashMap::new();
    let resolve = |raw: &str| -> String {
        if raw.starts_with("http://") || raw.starts_with("https://") {
            raw.to_owned()
        } else if raw.starts_with('/') {
            format!("{base}{raw}")
        } else {
            format!("{base}/{raw}")
        }
    };
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(4))
        .build();

    let all_frames = std::iter::once(post)
        .chain(pool_likes.iter_mut())
        .chain(pool_replies.iter_mut());
    for frame in all_frames {
        let Some(raw_url) = frame.avatar_url.clone() else {
            continue;
        };
        let url = resolve(&raw_url);
        let bytes = cache
            .entry(url.clone())
            .or_insert_with(|| fetch_avatar_best_effort(&agent, &url))
            .clone();
        frame.avatar_bytes = bytes;
    }
}

fn fetch_avatar_best_effort(agent: &ureq::Agent, url: &str) -> Option<Vec<u8>> {
    let response = agent.get(url).call().ok()?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1_500_000)
        .read_to_end(&mut bytes)
        .ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_valid_threads_urls_and_strips_tracking() {
        assert_eq!(
            normalize_threads_url("https://www.threads.com/@alice/post/C123_abc?xmt=AQF99#top")
                .unwrap(),
            "https://www.threads.com/@alice/post/C123_abc"
        );
        assert_eq!(
            normalize_threads_url("m.threads.net/@bob.vn/post/XYZ-1/").unwrap(),
            "https://www.threads.net/@bob.vn/post/XYZ-1"
        );
        assert_eq!(
            normalize_threads_url("http://threads.com/share/AbC123?foo=1").unwrap(),
            "https://www.threads.com/share/AbC123"
        );
        assert_eq!(
            normalize_threads_url("https://threads.net/t/Short99").unwrap(),
            "https://www.threads.net/t/Short99"
        );
    }

    #[test]
    fn rejects_non_post_or_non_threads_urls() {
        assert!(normalize_threads_url("").is_err());
        assert!(normalize_threads_url("https://www.threads.com/@alice").is_err());
        assert!(normalize_threads_url("https://example.com/@alice/post/123").is_err());
        assert!(normalize_threads_url("ftp://www.threads.com/@alice/post/123").is_err());
    }

    #[test]
    fn default_frames_prefers_non_link_comments_and_falls_back() {
        let post = Frame {
            id: "p1".into(),
            label: "Post".into(),
            author: "Author".into(),
            text: "Main post".into(),
            likes: "100".into(),
            replies: "10".into(),
            is_post: true,
            ..Frame::default()
        };
        let pool = vec![
            Frame {
                id: "c1".into(),
                label: "Comment 1".into(),
                author: "Spam".into(),
                text: "https://spam.example.com".into(),
                likes: "99".into(),
                replies: "0".into(),
                ..Frame::default()
            },
            Frame {
                id: "c2".into(),
                label: "Comment 2".into(),
                author: "Real 1".into(),
                text: "Quá hay!".into(),
                likes: "50".into(),
                replies: "5".into(),
                ..Frame::default()
            },
            Frame {
                id: "c3".into(),
                label: "Comment 3".into(),
                author: "Real 2".into(),
                text: "Chuẩn luôn".into(),
                likes: "40".into(),
                replies: "3".into(),
                ..Frame::default()
            },
            Frame {
                id: "c4".into(),
                label: "Comment 4".into(),
                author: "Real 3".into(),
                text: "Hóng tiếp".into(),
                likes: "30".into(),
                replies: "2".into(),
                ..Frame::default()
            },
            Frame {
                id: "c5".into(),
                label: "Comment 5".into(),
                author: "Real 4".into(),
                text: "Tuyệt vời".into(),
                likes: "20".into(),
                replies: "1".into(),
                ..Frame::default()
            },
        ];

        let frames = default_frames(&post, &pool);
        assert_eq!(frames.len(), 5);
        assert_eq!(frames[0].id, "p1");
        assert!(!frames.iter().any(|frame| frame.id == "c1"));
        assert_eq!(frames[4].id, "c5");
        assert_eq!(frames[4].label, "Comment 4");

        // Fallback still includes link-only comment when fewer than 4 real comments exist.
        let small_pool = vec![pool[0].clone(), pool[1].clone()];
        let fallback = default_frames(&post, &small_pool);
        assert_eq!(fallback.len(), 3);
        assert_eq!(fallback[1].id, "c2");
        assert_eq!(fallback[2].id, "c1");
    }
}
