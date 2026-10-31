// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The AI edit sheet's side of the window: the state the sheet reads,
//! and the handlers its callbacks land on.
//!
//! The shape follows the rest of the window. The sheet itself is Slint
//! state on the `Editor` global (see `ui/editor.slint`); this module is
//! the Rust that owns it. Provider settings live in `Preferences`, so
//! they survive a restart, and the prompt, the fetched free models and
//! the pending plan live here, in [`AiState`], for this run. A generation
//! and a models fetch both run on a worker through `host::spawn`, never
//! on the event-loop thread: a free model can take minutes to answer,
//! and the window must stay usable while it does.
//!
//! What a plan becomes is `concat-ai`'s business, not this module's: a
//! plan converts to a single `Command::Batch`, and `Studio::apply` applies
//! it atomically, so a plan that fails part-way commits nothing and
//! undoes as one step.

use std::rc::Rc;

use concat_ai::{AiClient, AiPlan, ClipSummary, EditContext, MediaSummary};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::studio::Studio;
use crate::ui::{App, Editor};

/// The AI sheet's run-time state: what the user typed, what the provider
/// listed, and the plan waiting for Apply. Provider settings themselves
/// are in `Preferences`, not here.
#[derive(Default)]
pub struct AiState {
    /// The command the user typed.
    pub prompt: String,
    /// Reference links, as typed, separated by commas or newlines.
    pub references: String,
    /// Free models the provider last listed, in the provider's order.
    pub models: Vec<String>,
    /// The plan waiting for Apply, if a generation produced one with
    /// steps in it.
    pub plan: Option<AiPlan>,
    /// What the sheet shows under the buttons: progress, a count, or an
    /// error, in the user's words.
    pub status: String,
    /// A fetch or a generation is running on a worker.
    pub busy: bool,
}

impl Studio {
    /// Publishes the AI sheet's state to the `Editor` global, alongside
    /// the rest of `publish`. The preview is the plan's summary and its
    /// plain-language lines, joined, so the sheet shows what Apply would
    /// do before it does it.
    pub fn publish_ai(&self, app: &App) {
        let editor = app.global::<Editor>();
        editor.set_ai_provider(self.prefs.ai_provider.clone().unwrap_or_default().into());
        editor.set_ai_base_url(self.prefs.ai_base_url.clone().unwrap_or_default().into());
        editor.set_ai_api_key(self.prefs.ai_api_key.clone().unwrap_or_default().into());
        editor.set_ai_models(ModelRc::from(Rc::new(VecModel::from(
            self.ai
                .models
                .iter()
                .map(|model| SharedString::from(model.as_str()))
                .collect::<Vec<_>>(),
        ))));
        editor.set_ai_model(self.prefs.ai_model.clone().unwrap_or_default().into());
        editor.set_ai_prompt(self.ai.prompt.clone().into());
        editor.set_ai_references(self.ai.references.clone().into());
        editor.set_ai_status(self.ai.status.clone().into());
        let preview = match &self.ai.plan {
            Some(plan) => {
                let mut lines = vec![plan.summary.clone()];
                lines.extend(plan.summarize());
                lines.join("\n")
            }
            None => String::new(),
        };
        editor.set_ai_preview(preview.into());
        editor.set_ai_busy(self.ai.busy);
        editor.set_ai_has_plan(self.ai.plan.is_some());
    }

    /// The provider, the base URL and the key, as remembered — or a
    /// message naming what is missing, which the sheet shows instead of
    /// calling anything.
    fn ai_config(&self) -> Result<concat_ai::ProviderConfig, String> {
        let Some(config) = self.prefs.ai_provider_config() else {
            return Err("Pick a provider first: OpenRouter or Custom.".to_string());
        };
        config.validate().map_err(|error| error.to_string())?;
        Ok(config)
    }

    /// The timeline as a generation sees it: the prompt, the references
    /// split on commas and newlines, the bin's media, and the timeline's
    /// clips in time order, with its total duration.
    fn ai_context(&self) -> EditContext {
        let project = self.project();
        let timeline = project.active();
        let mut clips: Vec<ClipSummary> = timeline
            .clips
            .iter()
            .map(|clip| ClipSummary {
                id: clip.id.clone(),
                name: clip.name.clone(),
                start_seconds: clip.start,
                duration_seconds: clip.duration,
            })
            .collect();
        clips.sort_by(|a, b| {
            a.start_seconds
                .partial_cmp(&b.start_seconds)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        EditContext {
            prompt: self.ai.prompt.clone(),
            references: self
                .ai
                .references
                .split([',', '\n'])
                .map(str::trim)
                .filter(|reference| !reference.is_empty())
                .map(str::to_string)
                .collect(),
            attachments: Vec::new(),
            media: project
                .media
                .iter()
                .map(|item| MediaSummary {
                    id: item.id.clone(),
                    name: item.name.clone(),
                })
                .collect(),
            clips,
            duration_seconds: timeline.duration(),
        }
    }

    /// Remembers the provider choice and clears anything that belonged
    /// to the old one: its free models, its picked model, and any plan.
    pub fn ai_set_provider(&mut self, provider: &str) {
        let provider = provider.trim();
        if provider.is_empty() {
            return;
        }
        self.prefs.ai_provider = Some(provider.to_string());
        self.prefs.ai_model = None;
        self.ai.models.clear();
        self.ai.plan = None;
        self.ai.status.clear();
        self.prefs.save(&self.host.dirs);
    }

    /// Remembers a custom provider's base URL, and clears the free
    /// models it listed, since they belong to the old URL.
    pub fn ai_set_base_url(&mut self, url: &str) {
        self.prefs.ai_base_url = Some(url.trim().to_string());
        self.ai.models.clear();
        self.prefs.ai_model = None;
        self.ai.plan = None;
        self.prefs.save(&self.host.dirs);
    }

    /// Remembers the provider's API key. It is written to the private
    /// settings file and sent only to the configured provider; nothing
    /// here logs it.
    pub fn ai_set_api_key(&mut self, key: &str) {
        self.prefs.ai_api_key = Some(key.trim().to_string());
        self.prefs.save(&self.host.dirs);
    }

    /// Remembers the picked free model.
    pub fn ai_set_model(&mut self, model: &str) {
        self.prefs.ai_model = Some(model.trim().to_string());
        self.prefs.save(&self.host.dirs);
    }

    /// The prompt, as typed. Kept in run-time state, not in preferences:
    /// a command is for this edit, not for every launch.
    pub fn ai_set_prompt(&mut self, prompt: &str) {
        self.ai.prompt = prompt.to_string();
    }

    /// Reference links, as typed.
    pub fn ai_set_references(&mut self, references: &str) {
        self.ai.references = references.to_string();
    }

    /// Fetches the configured provider's free models on a worker. The
    /// sheet shows progress while it runs, and the count — or the error,
    /// in full — when it lands. A picked model that is no longer listed
    /// is dropped, so Apply can never name a model the provider does
    /// not offer.
    pub fn ai_fetch_models(&mut self) {
        if self.ai.busy {
            return;
        }
        let config = match self.ai_config() {
            Ok(config) => config,
            Err(message) => {
                self.ai.status = message;
                return;
            }
        };
        // A models fetch needs no model picked yet, so the key and base
        // URL are checked on their own rather than through the full
        // configuration validation.
        if config.api_key.trim().is_empty()
            || (config.kind == concat_ai::ProviderKind::Custom && config.base_url().is_empty())
        {
            self.ai.status =
                "An API key — and, for a custom provider, a base URL — is needed first."
                    .to_string();
            return;
        }
        self.ai.busy = true;
        self.ai.status = "Fetching free models…".to_string();
        crate::host::spawn(
            move || AiClient::new(config).fetch_free_models(),
            |studio, _, _, result| {
                studio.ai.busy = false;
                match result {
                    Ok(models) => {
                        if models.is_empty() {
                            studio.ai.status =
                                "No free models found for this provider.".to_string();
                        } else {
                            studio.ai.status = format!("{} free model(s) found.", models.len());
                        }
                        if let Some(picked) = &studio.prefs.ai_model {
                            if !models.contains(picked) {
                                studio.prefs.ai_model = None;
                                studio.prefs.save(&studio.host.dirs);
                            }
                        }
                        studio.ai.models = models;
                    }
                    Err(error) => {
                        studio.ai.status = error.to_string();
                    }
                }
            },
        );
    }

    /// Generates a plan on a worker, for the project still open when it
    /// lands. An empty plan is a legitimate answer — the instruction
    /// cannot be done with the allowed ops — and the sheet shows its
    /// summary instead of offering Apply.
    pub fn ai_generate(&mut self) {
        if self.ai.busy {
            return;
        }
        if self.ai.prompt.trim().is_empty() {
            self.ai.status = "Type what should change first.".to_string();
            return;
        }
        let config = match self.ai_config() {
            Ok(config) => config,
            Err(message) => {
                self.ai.status = message;
                return;
            }
        };
        let context = self.ai_context();
        self.ai.busy = true;
        self.ai.status = "Asking the model for a plan…".to_string();
        self.ai.plan = None;
        crate::host::spawn_in_project(
            move || AiClient::new(config).generate_plan(&context),
            |studio, _, _, result| {
                studio.ai.busy = false;
                match result {
                    Ok(plan) => {
                        if plan.steps.is_empty() {
                            studio.ai.status = if plan.summary.is_empty() {
                                "The model returned an empty plan.".to_string()
                            } else {
                                plan.summary.clone()
                            };
                            studio.ai.plan = None;
                        } else {
                            studio.ai.plan = Some(plan);
                            studio.ai.status = "Plan ready — review it, then Apply.".to_string();
                        }
                    }
                    Err(error) => {
                        studio.ai.plan = None;
                        studio.ai.status = error.to_string();
                    }
                }
            },
        );
    }

    /// Applies the pending plan as one atomic, undoable edit, then
    /// clears it. A plan that fails validation here — it passed when it
    /// was generated, but the timeline may have moved since — is shown
    /// as an error, and nothing is applied.
    pub fn ai_apply(&mut self) {
        let Some(plan) = self.ai.plan.take() else {
            self.ai.status = "No plan to apply.".to_string();
            return;
        };
        match plan.to_command() {
            Ok(command) => {
                self.apply(command);
                self.ai.status = "Plan applied.".to_string();
            }
            Err(error) => {
                self.ai.status = error.to_string();
            }
        }
    }

    /// Drops the pending plan without applying it.
    pub fn ai_discard(&mut self) {
        self.ai.plan = None;
        self.ai.status.clear();
    }
}
