// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The AI edit planner for Concat's Android app (and, through the same
//! window code, every other platform it runs on).
//!
//! The shape of the feature, in one sentence: a prompt box, two
//! OpenAI-compatible providers, and a plan that is checked before it is
//! ever allowed near a timeline.
//!
//! The two providers are OpenRouter, whose base URL is fixed, and a custom
//! provider, whose base URL and API key the user enters. Both list models
//! through the provider's `/models` endpoint, and both lists are filtered
//! to free models only: an id ending in `:free`, or a model whose reported
//! prompt and completion prices are both zero. A provider that reports no
//! free model is an empty list, not a fallback to paid models — that
//! strictness is the point of the feature, and the reason a custom
//! provider that reports no pricing at all only ever yields `:free` ids.
//!
//! The model never edits directly. It returns one JSON object — a short
//! summary and a list of steps — and each step is one
//! [`concat_project::Command`] in its wire form. [`AiPlan::to_command`]
//! deserialises every step through that type, rejects any op outside the
//! allowlist, and wraps the survivors in a single
//! [`concat_project::Command::Batch`], so the window applies the whole
//! plan atomically and it undoes as one step. A plan that names a clip,
//! a track or a time the timeline does not have still fails safely: the
//! engine refuses it, the batch commits nothing, and the window says why.
//!
//! Keys live where the window's other preferences live, on the device,
//! and are sent only to the provider the user configured. Nothing here
//! logs a key, a prompt's attachments, or a plan's text.

use std::time::Duration;

use concat_project::Command;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// OpenRouter's OpenAI-compatible base. The custom provider replaces this
/// whole value with the base URL the user typed.
pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";

/// The most steps one plan may carry. A plan longer than this is almost
/// certainly a model that lost the shape of the task, and a short plan is
/// easier to read before it is applied.
pub const MAX_PLAN_STEPS: usize = 50;

/// Ops a plan may use, in wire (camelCase) form. Everything else a
/// [`Command`] can express — importing media, deleting timelines,
/// renaming projects, adding fonts, touching a project's structure — is
/// deliberately out: those are the user's own actions, not a prompt's.
/// `batch` is out too, so a nested batch cannot smuggle a disallowed op
/// past the top-level check.
pub const ALLOWED_OPS: &[&str] = &[
    "addClipAtFirstFree",
    "addClipsInSequence",
    "addTextClip",
    "addLayerClip",
    "splitClips",
    "trimClip",
    "removeClips",
    "moveClips",
    "setClipSpeed",
];

/// What can go wrong, in the window's language rather than a library's:
/// a missing key, a provider that answered badly, or a plan that failed
/// its checks, each with enough detail to show the user.
#[derive(Debug, Error)]
pub enum AiError {
    /// The provider configuration is incomplete: no key, or a custom
    /// provider with no base URL.
    #[error("AI provider is not configured: {0}")]
    NotConfigured(String),
    /// An HTTP call failed, with the status when the provider gave one.
    #[error("AI provider request failed: {0}")]
    Request(String),
    /// The provider's reply was not the shape this feature reads.
    #[error("AI provider reply was not usable: {0}")]
    BadReply(String),
    /// A plan failed validation, with the step and the reason.
    #[error("AI plan was rejected: {0}")]
    InvalidPlan(String),
}

/// Which provider a configuration talks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// OpenRouter, at its fixed base URL.
    OpenRouter,
    /// Any OpenAI-compatible endpoint the user named.
    Custom,
}

/// Everything needed to call one provider, and nothing else. The key is
/// held here only for the call that uses it; persisting it is the
/// window's preferences file, on the device, and it is never logged.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// Which provider this is.
    pub kind: ProviderKind,
    /// A custom provider's base URL, without a trailing slash. Ignored
    /// for OpenRouter, whose base is fixed.
    #[serde(default)]
    pub base_url: Option<String>,
    /// The provider's API key.
    pub api_key: String,
    /// The free model to generate with, as listed by `/models`.
    #[serde(default)]
    pub model: Option<String>,
}

impl ProviderConfig {
    /// The base URL this configuration actually calls.
    pub fn base_url(&self) -> String {
        match self.kind {
            ProviderKind::OpenRouter => OPENROUTER_BASE.to_string(),
            ProviderKind::Custom => self
                .base_url
                .as_deref()
                .unwrap_or("")
                .trim()
                .trim_end_matches('/')
                .to_string(),
        }
    }

    /// The models endpoint for this configuration.
    pub fn models_url(&self) -> String {
        format!("{}/models", self.base_url())
    }

    /// The chat-completions endpoint for this configuration.
    pub fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url())
    }

    /// Checks the configuration before any call is made, so a missing key
    /// or base URL is a plain message, not a network error.
    pub fn validate(&self) -> Result<(), AiError> {
        if self.api_key.trim().is_empty() {
            return Err(AiError::NotConfigured("an API key is needed".to_string()));
        }
        if self.kind == ProviderKind::Custom && self.base_url().is_empty() {
            return Err(AiError::NotConfigured(
                "a custom provider needs a base URL".to_string(),
            ));
        }
        if self.model.as_deref().unwrap_or("").trim().is_empty() {
            return Err(AiError::NotConfigured(
                "a free model needs to be picked first".to_string(),
            ));
        }
        Ok(())
    }
}

/// One clip the prompt may refer to, as the window knows it: its id, its
/// name, and where it sits on the timeline. The model is told only these
/// facts, so it can only name clips that exist.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipSummary {
    /// The clip's id, as a step's `clipId` would name it.
    pub id: String,
    /// The clip's display name.
    pub name: String,
    /// Timeline start, in seconds.
    pub start_seconds: f64,
    /// Timeline duration, in seconds.
    pub duration_seconds: f64,
}

/// One bin item the model may lay out: its id, as a step's `mediaId`
/// would name it, and its name. The files themselves reach the timeline
/// through the bin, the way any import does, not through the model.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSummary {
    /// The bin item's id.
    pub id: String,
    /// The bin item's display name.
    pub name: String,
}

/// Everything a generation needs besides the provider: the user's words,
/// the references they pasted, the files they attached, and the timeline
/// as it stands.
#[derive(Clone, Debug, Default)]
pub struct EditContext {
    /// The user's command, in their own words.
    pub prompt: String,
    /// Reference links, pasted as text. They are context for the model —
    /// a style or a structure to follow — not media the app downloads,
    /// which most link targets would not allow anyway.
    pub references: Vec<String>,
    /// Attached photo and video file names, for the model to name in its
    /// summary. The files themselves reach the timeline through the
    /// bin, the way any import does, not through the model.
    pub attachments: Vec<String>,
    /// The bin's media, in bin order: what the model may lay out.
    pub media: Vec<MediaSummary>,
    /// The timeline's clips, in timeline order.
    pub clips: Vec<ClipSummary>,
    /// The timeline's total duration, in seconds.
    pub duration_seconds: f64,
}

impl EditContext {
    /// The user message a generation sends: the command first, then the
    /// references, attachments and timeline facts it may use, each capped
    /// so a long project cannot blow out a free model's context.
    pub fn user_message(&self) -> String {
        let mut out = String::new();
        out.push_str("Edit this timeline as instructed:\n\n");
        out.push_str(&truncate(&self.prompt, 4000));
        if !self.references.is_empty() {
            out.push_str("\n\nReference links (style/structure references only):\n");
            for reference in self.references.iter().take(20) {
                out.push_str("- ");
                out.push_str(&truncate(reference, 500));
                out.push('\n');
            }
        }
        if !self.attachments.is_empty() {
            out.push_str("\nAttached files:\n");
            for attachment in self.attachments.iter().take(50) {
                out.push_str("- ");
                out.push_str(&truncate(attachment, 300));
                out.push('\n');
            }
        }
        if !self.media.is_empty() {
            out.push_str("\nBin media (id, name) — the only media a plan may lay out:\n");
            for item in self.media.iter().take(200) {
                out.push_str(&format!("- {} | {}\n", item.id, item.name));
            }
        }
        out.push_str(&format!(
            "\nTimeline duration: {:.2} seconds.\n",
            self.duration_seconds
        ));
        if self.clips.is_empty() {
            out.push_str("The timeline has no clips yet.\n");
        } else {
            out.push_str("Clips (id, name, start, duration):\n");
            for clip in self.clips.iter().take(200) {
                out.push_str(&format!(
                    "- {} | {} | {:.2}s | {:.2}s\n",
                    clip.id, clip.name, clip.start_seconds, clip.duration_seconds
                ));
            }
        }
        out
    }
}

/// The system prompt: the plan's shape, the allowed ops, and the rules
/// that keep a plan inside what the window can safely apply.
pub fn system_prompt() -> String {
    "You are an editor for the Concat video editor. Reply with JSON only, \
        no other text, in exactly this shape:\n\
        {\"summary\": \"one sentence saying what the plan does\", \
        \"steps\": [{\"op\": \"...\", ...}]}\n\
        Allowed ops, with the fields each step carries:\n\
        - addClipsInSequence: {\"mediaIds\": [\"m1\", ...], \"start\": 0} — lay bin items out in order. \
        Use this only for media ids from the bin list; never invent one.\n\
        - addClipAtFirstFree: {\"mediaId\": \"m1\", \"start\": 0}\n\
        - addTextClip: {\"trackId\": null, \"start\": 0, \"duration\": 3, \
        \"style\": {\"content\": \"words to show\"}} — the style holds only content; every other \
        style field is left to its default.\n\
        - addLayerClip: {\"trackId\": null, \"start\": 0, \"duration\": 3, \
        \"effectId\": \"concat.warm\", \"name\": \"Warm\"} — only an effect id the user named.\n\
        - splitClips: {\"clipIds\": [\"c1\"], \"time\": 5}\n\
        - trimClip: {\"clipId\": \"c1\", \"edge\": \"start\" or \"end\", \"delta\": 1.5}\n\
        - removeClips: {\"clipIds\": [\"c1\"]}\n\
        - moveClips: {\"moves\": [{\"clipId\": \"c1\", \"trackId\": \"t1\", \"start\": 0}]}\n\
        - setClipSpeed: {\"clipId\": \"c1\", \"speed\": 1.5}\n\
        Rules: use only clip ids from the timeline and media ids from the bin list. \
        Keep every time inside the timeline's duration. Never use any other op, \
        never use a nested batch, and never invent a file, an effect, or an id. \
        If the instruction cannot be done with these ops, return an empty steps list \
        and say so in the summary.".to_string()
}

/// A plan as the model returned it, before it becomes a command: the
/// summary the window shows, and the steps as raw JSON, each still to be
/// validated and deserialised by [`AiPlan::to_command`].
#[derive(Clone, Debug)]
pub struct AiPlan {
    /// One sentence saying what the plan does, shown before applying.
    pub summary: String,
    /// The steps, in wire form, one JSON object each.
    pub steps: Vec<Value>,
}

impl AiPlan {
    /// Turns the plan into the one command the window applies: a batch
    /// of the steps, so the whole plan commits atomically and undoes as
    /// a single step. Any step that fails validation, or that does not
    /// deserialise as a [`Command`], rejects the whole plan.
    pub fn to_command(&self) -> Result<Command, AiError> {
        validate_steps(&self.steps)?;
        let mut commands = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            let command: Command = serde_json::from_value(step.clone()).map_err(|error| {
                AiError::InvalidPlan(format!("a step did not match a known edit: {error}"))
            })?;
            commands.push(command);
        }
        Ok(Command::Batch { commands })
    }

    /// The plan in plain language, one line per step, for the preview
    /// the user reads before tapping Apply.
    pub fn summarize(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|step| {
                let op = step.get("op").and_then(Value::as_str).unwrap_or("");
                let number = |key: &str| step.get(key).and_then(Value::as_f64);
                let text = |key: &str| step.get(key).and_then(Value::as_str).unwrap_or("");
                let count = |key: &str| {
                    step.get(key)
                        .and_then(Value::as_array)
                        .map(|items| items.len())
                        .unwrap_or(0)
                };
                match op {
                    "addClipsInSequence" => format!(
                        "Lay out {} clip(s) in order from {:.2}s",
                        count("mediaIds"),
                        number("start").unwrap_or(0.0)
                    ),
                    "addClipAtFirstFree" => format!(
                        "Add {} at {:.2}s",
                        text("mediaId"),
                        number("start").unwrap_or(0.0)
                    ),
                    "addTextClip" => {
                        let words = step
                            .get("style")
                            .and_then(|style| style.get("content"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if words.is_empty() {
                            format!("Add a title at {:.2}s", number("start").unwrap_or(0.0))
                        } else {
                            format!(
                                "Add the title \"{}\" at {:.2}s",
                                truncate(words, 80),
                                number("start").unwrap_or(0.0)
                            )
                        }
                    }
                    "addLayerClip" => format!(
                        "Add the \"{}\" layer at {:.2}s",
                        text("name"),
                        number("start").unwrap_or(0.0)
                    ),
                    "splitClips" => format!(
                        "Split {} clip(s) at {:.2}s",
                        count("clipIds"),
                        number("time").unwrap_or(0.0)
                    ),
                    "trimClip" => format!(
                        "Trim clip {} by {:.2}s",
                        text("clipId"),
                        number("delta").unwrap_or(0.0)
                    ),
                    "removeClips" => format!("Remove {} clip(s)", count("clipIds")),
                    "moveClips" => format!("Move {} clip(s)", count("moves")),
                    "setClipSpeed" => format!(
                        "Set clip {} to {:.2}x speed",
                        text("clipId"),
                        number("speed").unwrap_or(1.0)
                    ),
                    other => other.to_string(),
                }
            })
            .collect()
    }
}

/// Extracts the JSON object from a model reply that may have wrapped it
/// in a fenced code block or in a sentence around it. A reply with no
/// object at all is an error, not an empty plan.
pub fn extract_json(text: &str) -> Result<String, AiError> {
    let without_fences: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n");
    let start = without_fences
        .find('{')
        .ok_or_else(|| AiError::BadReply("no JSON object in the reply".to_string()))?;
    let end = without_fences
        .rfind('}')
        .ok_or_else(|| AiError::BadReply("no JSON object in the reply".to_string()))?;
    if end < start {
        return Err(AiError::BadReply("no JSON object in the reply".to_string()));
    }
    Ok(without_fences[start..=end].to_string())
}

/// Parses and validates a model's reply into a plan. Validation here is
/// about shape and safety — op allowlist, sane numbers, bounded sizes —
/// and [`AiPlan::to_command`] adds the [`Command`] deserialisation check
/// before anything is applied.
pub fn parse_plan(text: &str) -> Result<AiPlan, AiError> {
    let json = extract_json(text)?;
    let raw: Value = serde_json::from_str(&json)
        .map_err(|error| AiError::BadReply(format!("the reply was not valid JSON: {error}")))?;
    let object = raw
        .as_object()
        .ok_or_else(|| AiError::BadReply("the plan was not a JSON object".to_string()))?;
    let summary = object
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if summary.chars().count() > 500 {
        return Err(AiError::InvalidPlan(
            "the plan's summary is too long".to_string(),
        ));
    }
    let steps = object
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::InvalidPlan("the plan has no steps list".to_string()))?
        .clone();
    if steps.is_empty() {
        // An empty plan is a legitimate answer — "this cannot be done
        // with the allowed ops" — and the window shows the summary
        // instead of applying nothing.
        return Ok(AiPlan { summary, steps });
    }
    validate_steps(&steps)?;
    Ok(AiPlan { summary, steps })
}

/// The checks every plan passes, whether it came from a model or from a
/// test: an allowlisted op, an object per step, and numbers that are
/// finite, in range, and attached to non-empty ids where an id is named.
fn validate_steps(steps: &[Value]) -> Result<(), AiError> {
    if steps.len() > MAX_PLAN_STEPS {
        return Err(AiError::InvalidPlan(format!(
            "the plan has {} steps, more than the {} allowed",
            steps.len(),
            MAX_PLAN_STEPS
        )));
    }
    for (index, step) in steps.iter().enumerate() {
        let object = step
            .as_object()
            .ok_or_else(|| AiError::InvalidPlan(format!("step {} is not an object", index + 1)))?;
        let op = object
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| AiError::InvalidPlan(format!("step {} has no op", index + 1)))?;
        if !ALLOWED_OPS.contains(&op) {
            return Err(AiError::InvalidPlan(format!(
                "step {} uses op \"{op}\", which a plan may not use",
                index + 1
            )));
        }
        validate_step_numbers(index, object)?;
    }
    Ok(())
}

/// The numeric and id checks for one step, kept separate so the reasons
/// stay specific: a bad number names its field, a missing id names its
/// step.
fn validate_step_numbers(
    index: usize,
    object: &serde_json::Map<String, Value>,
) -> Result<(), AiError> {
    let step = index + 1;
    let reject = |what: &str| AiError::InvalidPlan(format!("step {step}: {what}"));
    for key in ["start", "time", "duration"] {
        if let Some(value) = object.get(key).and_then(Value::as_f64)
            && (!value.is_finite() || !(0.0..=86_400.0).contains(&value))
        {
            return Err(reject(&format!("{key} is out of range")));
        }
    }
    if let Some(delta) = object.get("delta").and_then(Value::as_f64)
        && (!delta.is_finite() || delta == 0.0 || delta.abs() > 86_400.0)
    {
        return Err(reject("delta is out of range"));
    }
    if let Some(speed) = object.get("speed").and_then(Value::as_f64)
        && (!speed.is_finite() || speed <= 0.0)
    {
        return Err(reject("speed must be positive"));
    }
    for key in ["clipId", "mediaId", "effectId"] {
        if let Some(value) = object.get(key)
            && value.is_string()
            && value.as_str().unwrap_or("").trim().is_empty()
        {
            return Err(reject(&format!("{key} is empty")));
        }
    }
    for key in ["clipIds", "mediaIds"] {
        if let Some(items) = object.get(key).and_then(Value::as_array) {
            if items.is_empty() || items.len() > 100 {
                return Err(reject(&format!("{key} must name 1 to 100 items")));
            }
            if items
                .iter()
                .any(|item| item.as_str().map(str::trim).unwrap_or("").is_empty())
            {
                return Err(reject(&format!("{key} contains an empty id")));
            }
        }
    }
    if let Some(content) = object
        .get("style")
        .and_then(|style| style.get("content"))
        .and_then(Value::as_str)
        && content.chars().count() > 500
    {
        return Err(reject("title text is too long"));
    }
    Ok(())
}

/// Whether one model entry, as `/models` reported it, counts as free:
/// a `:free` id, or a pricing block whose prompt and completion prices
/// are both zero. A model with no pricing block counts only by its id,
/// so a provider that reports no pricing cannot leak paid models in.
pub fn is_free_model(entry: &Value) -> bool {
    let id = match entry {
        Value::String(text) => text.as_str(),
        Value::Object(object) => object.get("id").and_then(Value::as_str).unwrap_or(""),
        _ => "",
    };
    if id.ends_with(":free") {
        return true;
    }
    if let Value::Object(object) = entry
        && let Some(pricing) = object.get("pricing").and_then(Value::as_object)
    {
        let prompt_free = pricing.get("prompt").map(is_zero_price).unwrap_or(false);
        let completion_free = pricing
            .get("completion")
            .map(is_zero_price)
            .unwrap_or(false);
        return prompt_free && completion_free;
    }
    false
}

/// A price as `/models` reports it — the string `"0"`, the number `0`,
/// or a zero-valued decimal — is free; anything else, including a
/// missing price, is not.
fn is_zero_price(value: &Value) -> bool {
    match value {
        Value::String(text) => text
            .trim()
            .parse::<f64>()
            .map(|price| price == 0.0)
            .unwrap_or(false),
        Value::Number(number) => number.as_f64().map(|price| price == 0.0).unwrap_or(false),
        _ => false,
    }
}

/// Parses a `/models` reply into the free model ids it lists, in the
/// provider's order, with duplicates removed. The reply's shape varies
/// by provider — an array under `data`, or a bare array — and both are
/// read.
pub fn parse_models_response(body: &str) -> Result<Vec<String>, AiError> {
    let raw: Value = serde_json::from_str(body).map_err(|error| {
        AiError::BadReply(format!("the models list was not valid JSON: {error}"))
    })?;
    let entries = raw
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| raw.as_array())
        .ok_or_else(|| AiError::BadReply("the models list had no data array".to_string()))?;
    let mut models = Vec::new();
    for entry in entries {
        if !is_free_model(entry) {
            continue;
        }
        let id = match entry {
            Value::String(text) => text.clone(),
            Value::Object(object) => object
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        };
        if !id.is_empty() && !models.contains(&id) {
            models.push(id);
        }
    }
    Ok(models)
}

/// The client that talks to a provider: one `ureq` agent, bounded
/// timeouts, and no retries here — rotation across free models is the
/// window's job, so a failure is reported, not hidden behind a retry
/// that would burn a free tier's rate limit.
pub struct AiClient {
    config: ProviderConfig,
    /// How long a models fetch may take.
    pub models_timeout: Duration,
    /// How long a generation may take. Free models can be slow to start,
    /// so this is generous, and the window still shows progress while it
    /// waits.
    pub chat_timeout: Duration,
}

impl AiClient {
    /// A client for one provider configuration.
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            models_timeout: Duration::from_secs(30),
            chat_timeout: Duration::from_secs(180),
        }
    }

    /// Fetches the provider's free models. The configuration's key is
    /// sent as a Bearer token, as OpenAI-compatible endpoints expect.
    pub fn fetch_free_models(&self) -> Result<Vec<String>, AiError> {
        let body = self.get(&self.config.models_url(), self.models_timeout)?;
        parse_models_response(&body)
    }

    /// Generates an edit plan for a timeline, from the provider's
    /// configured model. The model is re-checked as free at call time
    /// only by its id in the configuration, which came from
    /// [`AiClient::fetch_free_models`] in the first place.
    pub fn generate_plan(&self, context: &EditContext) -> Result<AiPlan, AiError> {
        self.config.validate()?;
        let model = self.config.model.clone().unwrap_or_default();
        let body = serde_json::json!({
            "model": model,
            "messages": [
                { "role": "system", "content": system_prompt() },
                { "role": "user", "content": context.user_message() },
            ],
            "temperature": 0.2,
            "max_tokens": 2000,
        });
        let reply = self.post(
            &self.config.chat_url(),
            &body.to_string(),
            self.chat_timeout,
        )?;
        let reply_json: Value = serde_json::from_str(&reply).map_err(|error| {
            AiError::BadReply(format!("the chat reply was not valid JSON: {error}"))
        })?;
        let text = reply_json
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AiError::BadReply("the chat reply had no message content".to_string())
            })?;
        parse_plan(text)
    }

    /// A GET with the provider's key, returning the body as text.
    fn get(&self, url: &str, timeout: Duration) -> Result<String, AiError> {
        let response = ureq::get(url)
            .timeout(timeout)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .set("Accept", "application/json")
            .call()
            .map_err(http_error)?;
        response.into_string().map_err(|error| {
            AiError::Request(format!("could not read the provider's reply: {error}"))
        })
    }

    /// A POST of a JSON body, returning the body as text.
    fn post(&self, url: &str, body: &str, timeout: Duration) -> Result<String, AiError> {
        let response = ureq::post(url)
            .timeout(timeout)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .send_string(body)
            .map_err(http_error)?;
        response.into_string().map_err(|error| {
            AiError::Request(format!("could not read the provider's reply: {error}"))
        })
    }
}

/// Turns a `ureq` failure into an [`AiError`] that names the status and
/// the provider's own message, truncated, when there is one.
fn http_error(error: ureq::Error) -> AiError {
    match error {
        ureq::Error::Status(code, response) => {
            let detail = response
                .into_string()
                .map(|body| truncate(&body, 300))
                .unwrap_or_default();
            if detail.is_empty() {
                AiError::Request(format!("provider returned HTTP {code}"))
            } else {
                AiError::Request(format!("provider returned HTTP {code}: {detail}"))
            }
        }
        other => AiError::Request(other.to_string()),
    }
}

/// The first `limit` characters of a string, on a character boundary, so
/// a long prompt or provider message cannot grow a message without bound.
fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        text.chars().take(limit).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_json() -> &'static str {
        r#"{
            "summary": "Add a title and split the first clip.",
            "steps": [
                {"op": "addTextClip", "trackId": null, "start": 1.0, "duration": 3.0,
                 "style": {"content": "Hello"}},
                {"op": "splitClips", "clipIds": ["c1"], "time": 5.0}
            ]
        }"#
    }

    #[test]
    fn free_models_keep_only_free_ids_and_zero_prices() {
        let body = r#"{"data": [
            {"id": "a/model:free"},
            {"id": "b/model", "pricing": {"prompt": "0", "completion": "0"}},
            {"id": "c/model", "pricing": {"prompt": "0.001", "completion": "0"}},
            {"id": "d/model", "pricing": {"prompt": "0"}},
            {"id": "e/model"}
        ]}"#;
        let models = parse_models_response(body).expect("parses");
        assert_eq!(models, vec!["a/model:free", "b/model"]);
    }

    #[test]
    fn free_models_accept_a_bare_array_and_string_entries() {
        let body = r#"["x/model:free", "y/model"]"#;
        assert_eq!(
            parse_models_response(body).expect("parses"),
            vec!["x/model:free"]
        );
    }

    #[test]
    fn plan_parses_from_a_fenced_reply_and_becomes_one_batch() {
        let reply = format!("Here is the plan:\n```json\n{}\n```", plan_json());
        let plan = parse_plan(&reply).expect("parses");
        assert_eq!(plan.summary, "Add a title and split the first clip.");
        assert_eq!(plan.steps.len(), 2);
        match plan.to_command().expect("converts") {
            Command::Batch { commands } => assert_eq!(commands.len(), 2),
            _ => panic!("a plan must become one batch"),
        }
        let lines = plan.summarize();
        assert!(lines[0].contains("Hello"));
        assert!(lines[1].contains("Split"));
    }

    #[test]
    fn plan_rejects_a_disallowed_op() {
        let reply = r#"{"summary": "No.", "steps": [
            {"op": "removeTimeline", "timelineId": "t1"}
        ]}"#;
        let error = parse_plan(reply).expect_err("must reject");
        assert!(matches!(error, AiError::InvalidPlan(_)));
    }

    #[test]
    fn plan_rejects_a_nested_batch() {
        let reply = r#"{"summary": "No.", "steps": [
            {"op": "batch", "commands": [{"op": "splitClips", "clipIds": ["c1"], "time": 1.0}]}
        ]}"#;
        assert!(matches!(
            parse_plan(reply).expect_err("must reject"),
            AiError::InvalidPlan(_)
        ));
    }

    #[test]
    fn plan_rejects_negative_times_and_empty_ids() {
        let reply = r#"{"summary": "No.", "steps": [
            {"op": "trimClip", "clipId": "", "edge": "end", "delta": -1.0}
        ]}"#;
        assert!(matches!(
            parse_plan(reply).expect_err("must reject"),
            AiError::InvalidPlan(_)
        ));
        let reply = r#"{"summary": "No.", "steps": [
            {"op": "splitClips", "clipIds": ["c1"], "time": -5.0}
        ]}"#;
        assert!(matches!(
            parse_plan(reply).expect_err("must reject"),
            AiError::InvalidPlan(_)
        ));
    }

    #[test]
    fn plan_rejects_too_many_steps() {
        let steps = (0..MAX_PLAN_STEPS + 1)
            .map(|_| r#"{"op": "splitClips", "clipIds": ["c1"], "time": 1.0}"#)
            .collect::<Vec<_>>()
            .join(",");
        let reply = format!(r#"{{"summary": "Too long.", "steps": [{steps}]}}"#);
        assert!(matches!(
            parse_plan(&reply).expect_err("must reject"),
            AiError::InvalidPlan(_)
        ));
    }

    #[test]
    fn provider_config_requires_a_key_and_a_custom_base() {
        let openrouter = ProviderConfig {
            kind: ProviderKind::OpenRouter,
            base_url: None,
            api_key: String::new(),
            model: Some("a/model:free".to_string()),
        };
        assert!(matches!(
            openrouter.validate().expect_err("no key"),
            AiError::NotConfigured(_)
        ));
        let custom = ProviderConfig {
            kind: ProviderKind::Custom,
            base_url: None,
            api_key: "key".to_string(),
            model: Some("a/model:free".to_string()),
        };
        assert!(matches!(
            custom.validate().expect_err("no base"),
            AiError::NotConfigured(_)
        ));
        assert_eq!(
            ProviderConfig {
                kind: ProviderKind::Custom,
                base_url: Some("https://example.test/v1/".to_string()),
                api_key: "key".to_string(),
                model: None,
            }
            .models_url(),
            "https://example.test/v1/models"
        );
    }

    #[test]
    fn user_message_names_clips_references_and_attachments() {
        let context = EditContext {
            prompt: "Make a fast cut.".to_string(),
            references: vec!["https://example.test/ref".to_string()],
            attachments: vec!["clip.mp4".to_string()],
            media: vec![MediaSummary {
                id: "m1".to_string(),
                name: "clip.mp4".to_string(),
            }],
            clips: vec![ClipSummary {
                id: "c1".to_string(),
                name: "Intro".to_string(),
                start_seconds: 0.0,
                duration_seconds: 4.0,
            }],
            duration_seconds: 4.0,
        };
        let message = context.user_message();
        assert!(message.contains("Make a fast cut."));
        assert!(message.contains("https://example.test/ref"));
        assert!(message.contains("clip.mp4"));
        assert!(message.contains("c1 | Intro"));
    }
}
