// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The small boundary between Concat and the Python Threads extractor.
//!
//! Extraction stays in the existing TGS service for this first migration
//! step. Concat owns what follows it: the native project, speech and render.

use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub label: String,
    pub author: String,
    pub text: String,
    pub likes: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extraction {
    pub title: String,
    pub source_url: String,
    pub frames: Vec<Frame>,
}

/// Extract a post and its four highest-ranked top-level comments from the
/// local TGS service. `progress` receives only meaningful stage changes so
/// the window is not republished for every poll tick.
pub fn extract(source_url: &str, mut progress: impl FnMut(String)) -> Result<Extraction, String> {
    let base = std::env::var("TGS_API_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8000".to_owned())
        .trim_end_matches('/')
        .to_owned();

    let payload = serde_json::json!({
        "source_url": source_url,
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
    let ranking = exported
        .get("ranking_data")
        .and_then(|data| data.get("by_likes"))
        .and_then(|section| section.get("items"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut frames = Vec::with_capacity(5);
    let post_text = curated
        .and_then(|value| value.get("edited_post_text"))
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| string(post, "text"));
    frames.push(Frame {
        label: "Post".to_owned(),
        author: author(post),
        text: post_text,
        likes: engagement(post, "likes"),
    });

    let comment_by_id: HashMap<String, serde_json::Value> = comments
        .into_iter()
        .filter(|comment| string(comment, "type") == "comment")
        .map(|comment| (string(&comment, "id"), comment))
        .collect();
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

    let mut selected: Vec<serde_json::Value> = ranked_ids
        .into_iter()
        .filter_map(|(_, id)| comment_by_id.get(&id).cloned())
        .take(4)
        .collect();
    if selected.len() < 4 {
        let mut fallback: Vec<serde_json::Value> = comment_by_id.values().cloned().collect();
        fallback.sort_by_key(|comment| {
            std::cmp::Reverse(
                comment
                    .get("engagement")
                    .and_then(|item| item.get("likes"))
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
            )
        });
        for comment in fallback {
            if selected
                .iter()
                .any(|item| string(item, "id") == string(&comment, "id"))
            {
                continue;
            }
            selected.push(comment);
            if selected.len() == 4 {
                break;
            }
        }
    }

    let edited = curated
        .and_then(|value| value.get("edited_texts"))
        .and_then(serde_json::Value::as_object);
    for (index, comment) in selected.into_iter().enumerate() {
        let id = string(&comment, "id");
        let text = edited
            .and_then(|values| values.get(&id))
            .and_then(serde_json::Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| string(&comment, "text"));
        frames.push(Frame {
            label: format!("Comment {}", index + 1),
            author: author(&comment),
            text,
            likes: engagement(&comment, "likes"),
        });
    }

    let title = exported
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("Threads quick video")
        .to_owned();
    let source_url = exported
        .get("source_url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(source_url)
        .to_owned();
    Ok(Extraction {
        title,
        source_url,
        frames,
    })
}

fn request_json(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let response = match (method, body) {
        ("POST", Some(body)) => {
            let serialized = serde_json::to_string(body).map_err(|error| error.to_string())?;
            ureq::post(url)
                .set("Content-Type", "application/json")
                .send_string(&serialized)
        }
        _ => ureq::get(url).call(),
    };
    let response = response.map_err(|error| match error {
        ureq::Error::Status(code, response) => {
            let detail = response.into_string().unwrap_or_default();
            format!("TGS service returned HTTP {code}: {}", detail.trim())
        }
        ureq::Error::Transport(error) => format!(
            "Could not reach the TGS service at {}. Start the TGS Python backend and retry. ({error})",
            url.split("/api/").next().unwrap_or(url)
        ),
    })?;
    let body = response
        .into_string()
        .map_err(|error| format!("Could not read the TGS response: {error}"))?;
    serde_json::from_str(&body).map_err(|error| format!("TGS returned invalid JSON: {error}"))
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
        .map(|count| count.to_string())
        .unwrap_or_default()
}
