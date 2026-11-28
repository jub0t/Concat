// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Standalone Rust public Threads post & comment extractor.
//!
//! Fetches public Threads post HTML directly via `ureq`, parses embedded
//! Relay `<script type="application/json">` payloads (with OpenGraph meta
//! fallback), ranks comments by likes and replies, and hydrates author
//! avatars — completely decoupled from the legacy Python TGS service.
//!
//! Access controls, login walls, and CAPTCHA / anti-bot gates are strictly
//! respected: if encountered, extraction halts cleanly with a clear error.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::tgs::{Extraction, Frame, default_frames, format_compact_count, normalize_threads_url};

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// Extracts a public Threads post and its top comments directly in Rust.
pub fn extract(source_url: &str, mut progress: impl FnMut(String)) -> Result<Extraction, String> {
    let normalized_url = normalize_threads_url(source_url)?;
    progress("Fetching public Threads post directly…".to_owned());

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(25))
        .redirects(6)
        .build();

    let response = agent
        .get(&normalized_url)
        .set("User-Agent", BROWSER_UA)
        .set(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        )
        .set("Accept-Language", "vi-VN,vi;q=0.9,en-US;q=0.8,en;q=0.7")
        .set("Sec-Fetch-Dest", "document")
        .set("Sec-Fetch-Mode", "navigate")
        .set("Sec-Fetch-Site", "none")
        .set("Sec-Fetch-User", "?1")
        .set("Upgrade-Insecure-Requests", "1")
        .call()
        .map_err(|error| match error {
            ureq::Error::Status(code, _) => {
                format!("Threads returned HTTP {code} when loading this post.")
            }
            ureq::Error::Transport(err) => {
                format!("Could not reach Threads ({err}). Check your internet connection and retry.")
            }
        })?;

    let final_url = response.get_url().to_owned();
    let html = read_limited_string(response, 8 * 1024 * 1024)?;

    progress("Parsing Threads post and comments…".to_owned());
    let mut extraction = parse_threads_html(&normalized_url, &final_url, &html)?;

    progress("Loading author avatars & post media…".to_owned());
    hydrate_assets_direct(
        &mut extraction.post_frame,
        &mut extraction.pool_by_likes,
        &mut extraction.pool_by_replies,
    );
    extraction.frames = default_frames(&extraction.post_frame, &extraction.pool_by_likes);

    Ok(extraction)
}

/// Parses a fetched Threads HTML document into an [`Extraction`] without
/// performing any network I/O (also used by unit tests).
pub fn parse_threads_html(
    normalized_url: &str,
    final_url: &str,
    html: &str,
) -> Result<Extraction, String> {
    let lower_url = final_url.to_ascii_lowercase();
    let lower_html = html.to_ascii_lowercase();

    // Strictly respect login walls and anti-bot / CAPTCHA gates.
    if lower_url.contains("/login")
        || (lower_html.contains("log in to threads") && html.len() < 40_000 && !html.contains("data-sjs"))
    {
        return Err(
            "Threads required login to view this post. Only public Threads posts are supported."
                .to_owned(),
        );
    }
    if lower_html.contains("verify you are human")
        || lower_html.contains("security check")
        || (lower_html.contains("captcha") && !html.contains("application/json"))
    {
        return Err(
            "Threads presented a verification / CAPTCHA gate; extraction halted cleanly."
                .to_owned(),
        );
    }

    let og_url = extract_meta_content(html, "og:url");
    let og_title = extract_meta_content(html, "og:title");
    let og_desc = extract_meta_content(html, "og:description");
    let og_image = extract_meta_content(html, "og:image");

    let (effective_username, effective_code) = resolve_url_identity(
        normalized_url,
        final_url,
        og_url.as_deref(),
    );

    let mut posts_by_pk: HashMap<String, Value> = HashMap::new();
    let mut thread_order: Vec<String> = Vec::new();

    for script_body in extract_json_scripts(html) {
        if !(script_body.contains("\"caption\"")
            || script_body.contains("\"RelayPrefetchedStreamCache\"")
            || script_body.contains("\"thread_items\"")
            || script_body.contains("\"text_post_app_info\""))
        {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<Value>(script_body) {
            collect_post_dicts(&json, &mut posts_by_pk, &mut thread_order);
        }
    }

    if let Some((root_pk, primary)) = find_primary_post(
        &posts_by_pk,
        effective_code.as_deref(),
        effective_username.as_deref(),
    ) {
        let post_frame = build_frame_from_post(&primary, &root_pk, true, "Post".to_owned());
        let raw_comments = build_conversation_comments(&posts_by_pk, &root_pk, &thread_order);

        let mut pool_by_likes = raw_comments.clone();
        pool_by_likes.sort_by(|a, b| b.likes_count.cmp(&a.likes_count).then_with(|| b.replies_count.cmp(&a.replies_count)));
        let pool_by_likes: Vec<Frame> = pool_by_likes
            .into_iter()
            .enumerate()
            .map(|(i, mut c)| {
                c.frame.label = format!("Comment {}", i + 1);
                c.frame
            })
            .collect();

        let mut pool_by_replies = raw_comments;
        pool_by_replies.sort_by(|a, b| b.replies_count.cmp(&a.replies_count).then_with(|| b.likes_count.cmp(&a.likes_count)));
        let pool_by_replies: Vec<Frame> = pool_by_replies
            .into_iter()
            .enumerate()
            .map(|(i, mut c)| {
                c.frame.label = format!("Comment {}", i + 1);
                c.frame
            })
            .collect();

        let frames = default_frames(&post_frame, &pool_by_likes);
        let title = derive_title(&post_frame);
        let project_id = format!("native-{}", effective_code.as_deref().unwrap_or(&root_pk));

        return Ok(Extraction {
            title,
            source_url: final_url.to_owned(),
            project_id,
            post_frame,
            pool_by_likes,
            pool_by_replies,
            frames,
        });
    }

    // OpenGraph fallback when Threads serves meta tags without full Relay JSON.
    let fallback_text = og_desc.unwrap_or_default().trim().to_owned();
    if !fallback_text.is_empty() {
        let (author, username) = parse_og_author(og_title.as_deref(), effective_username.as_deref());
        let post_id = effective_code
            .clone()
            .unwrap_or_else(|| "post-og".to_owned());
        let og_media = og_image.filter(|url| {
            let low = url.to_ascii_lowercase();
            !low.contains("threads_logo") && !low.contains("rsrc.php")
        });
        let post_frame = Frame {
            id: post_id.clone(),
            label: "Post".to_owned(),
            author,
            username,
            media_url: og_media,
            text: fallback_text,
            is_post: true,
            ..Frame::default()
        };
        let title = derive_title(&post_frame);
        let frames = vec![post_frame.clone()];
        return Ok(Extraction {
            title,
            source_url: final_url.to_owned(),
            project_id: format!("native-{post_id}"),
            post_frame,
            pool_by_likes: Vec::new(),
            pool_by_replies: Vec::new(),
            frames,
        });
    }

    Err(
        "Could not extract post data from this Threads link. Make sure the post is public and try again."
            .to_owned(),
    )
}

#[derive(Clone)]
struct RankedComment {
    frame: Frame,
    likes_count: i64,
    replies_count: i64,
}

fn read_limited_string(response: ureq::Response, max_bytes: usize) -> Result<String, String> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max_bytes as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read Threads page response: {error}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn resolve_url_identity(
    normalized_url: &str,
    final_url: &str,
    og_url: Option<&str>,
) -> (Option<String>, Option<String>) {
    for candidate in [Some(final_url), og_url, Some(normalized_url)]
        .into_iter()
        .flatten()
    {
        if let Some((user, code)) = parse_user_and_code(candidate) {
            return (user, Some(code));
        }
    }
    (None, None)
}

fn parse_user_and_code(url: &str) -> Option<(Option<String>, String)> {
    let without_scheme = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url);
    let path_part = without_scheme.split_once('/').map(|(_, p)| p)?;
    let path = path_part.split(['?', '#']).next().unwrap_or_default().trim_matches('/');
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        [user, "post", code, ..] if user.starts_with('@') => {
            Some((Some(user[1..].to_owned()), (*code).to_owned()))
        }
        ["t", code, ..] | ["share", code, ..] => Some((None, (*code).to_owned())),
        _ => None,
    }
}

/// Scans HTML for `<script ... type="application/json" ...>...</script>` blocks.
fn extract_json_scripts(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0usize;

    while let Some(rel_start) = lower[cursor..].find("<script") {
        let tag_start = cursor + rel_start;
        let Some(rel_gt) = lower[tag_start..].find('>') else {
            break;
        };
        let open_end = tag_start + rel_gt + 1;
        let open_tag = &lower[tag_start..open_end];

        let Some(rel_close) = lower[open_end..].find("</script") else {
            break;
        };
        let close_start = open_end + rel_close;

        if open_tag.contains("application/json") {
            let body = html[open_end..close_start].trim();
            if body.starts_with('{') || body.starts_with('[') {
                out.push(body);
            }
        }
        cursor = close_start + 9;
    }

    out
}

fn extract_meta_content(html: &str, property: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let prop_lower = property.to_ascii_lowercase();
    let mut cursor = 0usize;

    while let Some(rel_start) = lower[cursor..].find("<meta") {
        let tag_start = cursor + rel_start;
        let Some(rel_gt) = lower[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + rel_gt + 1;
        let tag_orig = &html[tag_start..tag_end];
        let tag_low = &lower[tag_start..tag_end];

        let matches_prop = tag_low.contains(&format!("property=\"{prop_lower}\""))
            || tag_low.contains(&format!("property='{prop_lower}'"))
            || tag_low.contains(&format!("name=\"{prop_lower}\""))
            || tag_low.contains(&format!("name='{prop_lower}'"));

        if matches_prop
            && let Some(content) = extract_html_attr(tag_orig, "content")
        {
            let decoded = unescape_html_entities(&content);
            if !decoded.trim().is_empty() {
                return Some(decoded);
            }
        }
        cursor = tag_end;
    }
    None
}

fn extract_html_attr(tag: &str, attr_name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{attr_name}=");
    let pos = lower.find(&needle)?;
    let rest = tag[pos + needle.len()..].trim_start();
    let quote = rest.as_bytes().first().copied()?;
    if quote == b'"' || quote == b'\'' {
        let inner = &rest[1..];
        let end = inner.find(quote as char)?;
        Some(inner[..end].to_owned())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        Some(rest[..end].to_owned())
    }
}

fn unescape_html_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }
        let mut entity = String::new();
        let mut closed = false;
        while let Some(&next) = chars.peek() {
            if next == ';' {
                chars.next();
                closed = true;
                break;
            }
            if entity.len() >= 12 || next.is_whitespace() || next == '&' {
                break;
            }
            entity.push(next);
            chars.next();
        }
        if !closed {
            out.push('&');
            out.push_str(&entity);
            continue;
        }
        match entity.as_str() {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" | "#39" | "#x27" => out.push('\''),
            "nbsp" => out.push(' '),
            num if num.starts_with("#x") || num.starts_with("#X") => {
                if let Ok(code) = u32::from_str_radix(&num[2..], 16)
                    && let Some(c) = char::from_u32(code)
                {
                    out.push(c);
                }
            }
            num if num.starts_with('#') => {
                if let Ok(code) = num[1..].parse::<u32>()
                    && let Some(c) = char::from_u32(code)
                {
                    out.push(c);
                }
            }
            _ => {
                out.push('&');
                out.push_str(&entity);
                out.push(';');
            }
        }
    }
    out
}

fn normalize_pk(raw: Option<&Value>) -> Option<String> {
    let val = match raw? {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if val.is_empty() {
        return None;
    }
    if let Some((prefix, _)) = val.split_once('_')
        && !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_digit())
    {
        return Some(prefix.to_owned());
    }
    Some(val)
}

fn post_richness(post: &Value) -> i32 {
    let mut score = 0;
    if post
        .get("caption")
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .is_some_and(|t| !t.trim().is_empty())
    {
        score += 10;
    }
    if post
        .get("user")
        .and_then(|u| u.get("username"))
        .and_then(Value::as_str)
        .is_some_and(|u| !u.trim().is_empty())
    {
        score += 6;
    }
    if post.get("code").and_then(Value::as_str).is_some() {
        score += 4;
    }
    if post.get("like_count").and_then(Value::as_i64).is_some() {
        score += 2;
    }
    if post.get("text_post_app_info").is_some() {
        score += 2;
    }
    if post.get("carousel_media").is_some()
        || post.get("image_versions2").is_some()
        || post.get("video_versions").is_some()
        || post.get("giphy_media_info").is_some()
    {
        score += 3;
    }
    score
}

fn merge_post_value(existing: Value, candidate: Value) -> Value {
    let (mut primary, secondary) = if post_richness(&candidate) > post_richness(&existing) {
        (candidate, existing)
    } else {
        (existing, candidate)
    };
    if let (Some(p_obj), Some(s_obj)) = (primary.as_object_mut(), secondary.as_object()) {
        for key in [
            "carousel_media",
            "image_versions2",
            "video_versions",
            "giphy_media_info",
            "like_count",
            "taken_at",
            "code",
            "text_post_app_info",
            "caption",
            "user",
        ] {
            let missing = p_obj.get(key).is_none_or(Value::is_null);
            if missing && let Some(val) = s_obj.get(key) {
                p_obj.insert(key.to_owned(), val.clone());
            }
        }
    }
    primary
}

fn collect_post_dicts(
    node: &Value,
    by_pk: &mut HashMap<String, Value>,
    thread_order: &mut Vec<String>,
) {
    match node {
        Value::Object(map) => {
            let has_id = map.contains_key("pk") || map.contains_key("id");
            let has_post_shape = map.contains_key("caption") || map.contains_key("text_post_app_info");
            let has_user = map.get("user").is_some_and(Value::is_object);
            if has_id && has_post_shape && has_user {
                let pk = normalize_pk(map.get("pk").or_else(|| map.get("id")));
                if let Some(pk) = pk {
                    if !by_pk.contains_key(&pk) {
                        thread_order.push(pk.clone());
                    }
                    let merged = match by_pk.remove(&pk) {
                        Some(existing) => merge_post_value(existing, node.clone()),
                        None => node.clone(),
                    };
                    by_pk.insert(pk, merged);
                }
            }
            for val in map.values() {
                if val.is_object() || val.is_array() {
                    collect_post_dicts(val, by_pk, thread_order);
                }
            }
        }
        Value::Array(items) => {
            for elem in items {
                if elem.is_object() || elem.is_array() {
                    collect_post_dicts(elem, by_pk, thread_order);
                }
            }
        }
        _ => {}
    }
}

fn find_primary_post(
    by_pk: &HashMap<String, Value>,
    effective_code: Option<&str>,
    effective_username: Option<&str>,
) -> Option<(String, Value)> {
    if by_pk.is_empty() {
        return None;
    }

    // 1. Exact post shortcode match.
    if let Some(code) = effective_code {
        if let Some((pk, val)) = by_pk
            .iter()
            .filter(|(_, p)| p.get("code").and_then(Value::as_str) == Some(code))
            .max_by_key(|(_, p)| post_richness(p))
        {
            return Some((pk.clone(), val.clone()));
        }
        // 2. Direct PK match.
        if let Some(val) = by_pk.get(code) {
            return Some((code.to_owned(), val.clone()));
        }
    }

    // 3. Root non-reply post by target username.
    if let Some(uname) = effective_username {
        let target = uname.to_ascii_lowercase();
        if let Some((pk, val)) = by_pk
            .iter()
            .filter(|(_, p)| {
                let matches_user = p
                    .get("user")
                    .and_then(|u| u.get("username"))
                    .and_then(Value::as_str)
                    .is_some_and(|u| u.to_ascii_lowercase() == target);
                let is_root = normalize_pk(
                    p.get("text_post_app_info")
                        .and_then(|i| i.get("reply_to_post_id")),
                )
                .is_none();
                matches_user && is_root
            })
            .max_by_key(|(_, p)| post_richness(p))
        {
            return Some((pk.clone(), val.clone()));
        }
    }

    // 4. Fallback: richest non-reply post in the payload.
    by_pk
        .iter()
        .filter(|(_, p)| {
            normalize_pk(
                p.get("text_post_app_info")
                    .and_then(|i| i.get("reply_to_post_id")),
            )
            .is_none()
        })
        .max_by_key(|(_, p)| post_richness(p))
        .map(|(pk, val)| (pk.clone(), val.clone()))
}

fn build_conversation_comments(
    by_pk: &HashMap<String, Value>,
    root_pk: &str,
    thread_order: &[String],
) -> Vec<RankedComment> {
    let mut out = Vec::new();
    let mut added: HashSet<String> = HashSet::new();
    added.insert(root_pk.to_owned());

    // Pass 1: explicit top-level replies pointing directly to `root_pk`.
    for (pk, item) in by_pk {
        if added.contains(pk) {
            continue;
        }
        let reply_to = normalize_pk(
            item.get("text_post_app_info")
                .and_then(|info| info.get("reply_to_post_id")),
        );
        if reply_to.as_deref() == Some(root_pk) {
            if let Some(comment) = build_ranked_comment(item, pk) {
                out.push(comment);
                added.insert(pk.clone());
            }
        }
    }

    // Pass 2: if Threads SSR omitted `reply_to_post_id` on replies in the
    // page's Relay stream, include non-root posts that have substantive caption
    // text and appear after the root post in the page stream.
    if out.is_empty() {
        let root_pos = thread_order
            .iter()
            .position(|pk| pk == root_pk)
            .unwrap_or(0);
        for pk in thread_order.iter().skip(root_pos + 1) {
            if added.contains(pk) {
                continue;
            }
            let Some(item) = by_pk.get(pk) else {
                continue;
            };
            // Skip posts explicitly replying to some unrelated post.
            let reply_to = normalize_pk(
                item.get("text_post_app_info")
                    .and_then(|info| info.get("reply_to_post_id")),
            );
            if reply_to.is_some() && reply_to.as_deref() != Some(root_pk) {
                continue;
            }
            if let Some(comment) = build_ranked_comment(item, pk) {
                out.push(comment);
                added.insert(pk.clone());
            }
        }
    }

    out
}

fn build_ranked_comment(item: &Value, pk: &str) -> Option<RankedComment> {
    let text = item
        .get("caption")
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let (media_url, _) = extract_primary_media(item);
    if text.is_empty() && media_url.is_none() {
        return None;
    }
    let likes_count = item.get("like_count").and_then(Value::as_i64).unwrap_or(0);
    let replies_count = item
        .get("text_post_app_info")
        .and_then(|i| i.get("direct_reply_count"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let frame = build_frame_from_post(item, pk, false, "Comment".to_owned());
    Some(RankedComment {
        frame,
        likes_count,
        replies_count,
    })
}

fn build_frame_from_post(item: &Value, pk: &str, is_post: bool, label: String) -> Frame {
    let user = item.get("user");
    let username = user
        .and_then(|u| u.get("username"))
        .and_then(Value::as_str)
        .map(|s| s.trim().trim_start_matches('@').to_owned())
        .unwrap_or_default();
    let display_name = user
        .and_then(|u| u.get("full_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if username.is_empty() {
                "Threads".to_owned()
            } else {
                username.clone()
            }
        });
    let verified = user
        .and_then(|u| u.get("is_verified"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let avatar_url = user
        .and_then(|u| u.get("profile_pic_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let text = item
        .get("caption")
        .and_then(|c| c.get("text"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    let timestamp = item
        .get("taken_at")
        .and_then(Value::as_i64)
        .map(format_relative_age)
        .unwrap_or_default();
    let (media_url, media_is_video) = extract_primary_media(item);

    let likes = item
        .get("like_count")
        .and_then(Value::as_i64)
        .filter(|&c| c > 0)
        .map(format_compact_count)
        .unwrap_or_default();
    let info = item.get("text_post_app_info");
    let replies = info
        .and_then(|i| i.get("direct_reply_count"))
        .and_then(Value::as_i64)
        .filter(|&c| c > 0)
        .map(format_compact_count)
        .unwrap_or_default();
    let reposts = info
        .and_then(|i| i.get("reshare_count").or_else(|| i.get("repost_count")))
        .and_then(Value::as_i64)
        .filter(|&c| c > 0)
        .map(format_compact_count)
        .unwrap_or_default();

    Frame {
        id: pk.to_owned(),
        label,
        author: display_name,
        username,
        timestamp,
        verified,
        avatar_url,
        avatar_bytes: None,
        media_url,
        media_bytes: None,
        media_is_video,
        text,
        likes,
        replies,
        reposts,
        is_post,
        meme_path: None,
    }
}

/// Extracts the primary displayable image/thumbnail URL and video flag from a
/// Threads Relay post or comment object (`carousel_media`, `image_versions2`,
/// `video_versions`, or `giphy_media_info`).
fn extract_primary_media(item: &Value) -> (Option<String>, bool) {
    if let Some(carousel) = item.get("carousel_media").and_then(Value::as_array) {
        for entry in carousel {
            let is_video = entry
                .get("video_versions")
                .and_then(Value::as_array)
                .is_some_and(|v| !v.is_empty());
            if let Some(url) = pick_best_image_url(entry) {
                return (Some(url), is_video);
            }
        }
    }

    let is_video = item
        .get("video_versions")
        .and_then(Value::as_array)
        .is_some_and(|v| !v.is_empty());
    if let Some(url) = pick_best_image_url(item) {
        return (Some(url), is_video);
    }

    if let Some(gif_images) = item
        .get("giphy_media_info")
        .and_then(|g| g.get("images"))
    {
        for variant in ["fixed_height", "original", "downsized"] {
            if let Some(url) = gif_images
                .get(variant)
                .and_then(|v| v.get("url"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|u| !u.is_empty())
            {
                return (Some(url.to_owned()), false);
            }
        }
    }

    (None, false)
}

fn pick_best_image_url(node: &Value) -> Option<String> {
    let candidates = node
        .get("image_versions2")
        .and_then(|iv| iv.get("candidates"))
        .and_then(Value::as_array)?;
    candidates
        .iter()
        .filter_map(|cand| {
            let url = cand.get("url").and_then(Value::as_str)?.trim();
            if url.is_empty() {
                return None;
            }
            let w = cand.get("width").and_then(Value::as_u64).unwrap_or(0);
            let h = cand.get("height").and_then(Value::as_u64).unwrap_or(0);
            Some((w.saturating_mul(h), url.to_owned()))
        })
        .max_by_key(|(area, _)| *area)
        .map(|(_, url)| url)
}

fn format_relative_age(taken_at_unix: i64) -> String {
    if taken_at_unix <= 0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(taken_at_unix);
    let diff = (now - taken_at_unix).max(0);
    if diff < 60 {
        "now".to_owned()
    } else if diff < 3_600 {
        format!("{}m", diff / 60)
    } else if diff < 86_400 {
        format!("{}h", diff / 3_600)
    } else if diff < 86_400 * 7 {
        format!("{}d", diff / 86_400)
    } else {
        format!("{}w", diff / (86_400 * 7))
    }
}

fn parse_og_author(og_title: Option<&str>, fallback_username: Option<&str>) -> (String, String) {
    let fallback_user = fallback_username.unwrap_or_default().to_owned();
    let Some(title) = og_title.map(str::trim).filter(|t| !t.is_empty()) else {
        let author = if fallback_user.is_empty() {
            "Threads".to_owned()
        } else {
            fallback_user.clone()
        };
        return (author, fallback_user);
    };
    // Typical format: "Display Name (@username) on Threads"
    if let Some((before_paren, after_paren)) = title.split_once("(@") {
        let author = before_paren.trim().to_owned();
        let username = after_paren
            .split(')')
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        let author = if author.is_empty() {
            username.clone()
        } else {
            author
        };
        return (author, if username.is_empty() { fallback_user } else { username });
    }
    (title.to_owned(), fallback_user)
}

fn derive_title(post: &Frame) -> String {
    let snippet: String = post
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(48)
        .collect();
    if !snippet.is_empty() {
        snippet
    } else if !post.author.trim().is_empty() {
        format!("Threads · {}", post.author.trim())
    } else {
        "Threads quick video".to_owned()
    }
}

fn hydrate_assets_direct(
    post: &mut Frame,
    pool_likes: &mut [Frame],
    pool_replies: &mut [Frame],
) {
    let mut cache: HashMap<String, Option<Vec<u8>>> = HashMap::new();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout_read(Duration::from_secs(8))
        .build();

    let mut fetch_cached = |url: &str, max_bytes: u64| -> Option<Vec<u8>> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return None;
        }
        cache
            .entry(url.to_owned())
            .or_insert_with(|| {
                let resp = agent
                    .get(url)
                    .set("User-Agent", BROWSER_UA)
                    .set("Referer", "https://www.threads.com/")
                    .call()
                    .ok()?;
                let mut buf = Vec::new();
                resp.into_reader()
                    .take(max_bytes)
                    .read_to_end(&mut buf)
                    .ok()?;
                (!buf.is_empty()).then_some(buf)
            })
            .clone()
    };

    let all_frames = std::iter::once(post)
        .chain(pool_likes.iter_mut())
        .chain(pool_replies.iter_mut());
    for frame in all_frames {
        if let Some(url) = frame.avatar_url.clone() {
            frame.avatar_bytes = fetch_cached(&url, 1_500_000);
        }
        if let Some(url) = frame.media_url.clone() {
            frame.media_bytes = fetch_cached(&url, 6_000_000);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_relay_json_post_and_ranks_comments_by_likes_and_replies() {
        let html = r#"<!doctype html>
        <html>
        <head>
            <meta property="og:url" content="https://www.threads.com/@minh.dev/post/C999abc" />
        </head>
        <body>
            <script type="application/json" data-sjs>
            {
              "require": [[{
                "thread_items": [
                  {
                    "post": {
                      "pk": "1001_999",
                      "code": "C999abc",
                      "taken_at": 1700000000,
                      "caption": { "text": "Hôm nay code Rust phê quá mọi người ơi!" },
                      "like_count": 2450,
                      "image_versions2": {
                        "candidates": [
                          { "url": "https://cdn.example.com/small.jpg", "width": 320, "height": 240 },
                          { "url": "https://cdn.example.com/full.jpg", "width": 1080, "height": 810 }
                        ]
                      },
                      "user": {
                        "username": "minh.dev",
                        "full_name": "Minh Dev",
                        "is_verified": true,
                        "profile_pic_url": "https://cdn.example.com/minh.jpg"
                      },
                      "text_post_app_info": {
                        "direct_reply_count": 32,
                        "reshare_count": 14
                      }
                    }
                  },
                  {
                    "post": {
                      "pk": "2001",
                      "caption": { "text": "Chuẩn luôn, compile xong chạy mượt!" },
                      "like_count": 180,
                      "carousel_media": [
                        {
                          "image_versions2": {
                            "candidates": [
                              { "url": "https://cdn.example.com/comment-photo.jpg", "width": 800, "height": 800 }
                            ]
                          }
                        }
                      ],
                      "user": { "username": "lan.ui", "full_name": "Lan UI" },
                      "text_post_app_info": {
                        "reply_to_post_id": "1001",
                        "direct_reply_count": 3
                      }
                    }
                  },
                  {
                    "post": {
                      "pk": "2002",
                      "caption": { "text": "Bạn dùng crate gì cho TTS vậy?" },
                      "like_count": 45,
                      "user": { "username": "nam.ai", "full_name": "Nam AI" },
                      "text_post_app_info": {
                        "reply_to_post_id": "1001",
                        "direct_reply_count": 19
                      }
                    }
                  }
                ]
              }]]
            }
            </script>
        </body>
        </html>"#;

        let ext = parse_threads_html(
            "https://www.threads.com/@minh.dev/post/C999abc",
            "https://www.threads.com/@minh.dev/post/C999abc",
            html,
        )
        .expect("parses relay html");

        assert_eq!(ext.post_frame.author, "Minh Dev");
        assert_eq!(ext.post_frame.username, "minh.dev");
        assert!(ext.post_frame.verified);
        assert_eq!(
            ext.post_frame.media_url.as_deref(),
            Some("https://cdn.example.com/full.jpg")
        );
        assert_eq!(ext.post_frame.likes, "2.5K");
        assert_eq!(ext.post_frame.replies, "32");
        assert_eq!(ext.post_frame.reposts, "14");
        assert_eq!(ext.pool_by_likes.len(), 2);
        assert_eq!(ext.pool_by_likes[0].id, "2001");
        assert_eq!(
            ext.pool_by_likes[0].media_url.as_deref(),
            Some("https://cdn.example.com/comment-photo.jpg")
        );
        assert_eq!(ext.pool_by_replies[0].id, "2002");
        assert_eq!(ext.frames.len(), 3);
    }

    #[test]
    fn falls_back_to_opengraph_metadata_with_entity_unescaping() {
        let html = r#"<html><head>
            <meta property="og:title" content="Hải Anh (@haianh.vn) on Threads" />
            <meta property="og:description" content="Cà phê sáng &amp; đọc sách &#39;hay&#39;" />
            <meta property="og:url" content="https://www.threads.com/@haianh.vn/post/XYZ77" />
        </head><body></body></html>"#;

        let ext = parse_threads_html(
            "https://www.threads.com/share/XYZ77",
            "https://www.threads.com/@haianh.vn/post/XYZ77",
            html,
        )
        .expect("parses og fallback");

        assert_eq!(ext.post_frame.author, "Hải Anh");
        assert_eq!(ext.post_frame.username, "haianh.vn");
        assert_eq!(ext.post_frame.text, "Cà phê sáng & đọc sách 'hay'");
    }

    #[test]
    fn halts_cleanly_on_login_wall_or_captcha() {
        let login = parse_threads_html(
            "https://www.threads.com/@a/post/1",
            "https://www.threads.com/login?next=/@a/post/1",
            "<html><body>Log in to Threads</body></html>",
        );
        assert!(login.is_err());

        let captcha = parse_threads_html(
            "https://www.threads.com/@a/post/1",
            "https://www.threads.com/@a/post/1",
            "<html><body>Please verify you are human to continue.</body></html>",
        );
        assert!(captcha.is_err());
    }
}
