// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! The Extras sheet: what repositories offer, what is installed, what has
//! an update, and what is on its way in - and the queue that installs one
//! extra at a time, dependencies first.
//!
//! The decisions are `concat-extras`'s and the bytes `concat-host`'s; this
//! pane holds the indexes it was last given, the queue, and each running
//! install's progress, and turns them into rows. After anything that
//! changes what is installed, the parts of the window that read extras are
//! reloaded: the effects catalogue, the fonts the picker offers, the title
//! presets.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use concat_extras::index::Entry;
use concat_extras::registry::{Record, Registry};
use concat_extras::resolve::{self, Action, Step};
use concat_extras::{Category, Kind, human_size};
use concat_host::extras::{self as host_extras, Fetched, Inspected, Progress};

use crate::host::{on_ui, spawn};
use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::studio::{Confirm, Studio};
use crate::ui::{ExtraCardData, ExtraDetailData, ExtrasData};

/// The pages, as the sheet numbers them.
pub mod page {
    /// Everything offered, featured first.
    pub const DISCOVER: i32 = 0;
    /// What is installed.
    pub const INSTALLED: i32 = 1;
    /// What is installed and has a newer version offered.
    pub const UPDATES: i32 = 2;
    /// What is downloading, waiting, or failed.
    pub const DOWNLOADS: i32 = 3;
    /// The first shelf; the shelves follow in `Category::ALL` order.
    pub const FIRST_SHELF: i32 = 4;
    /// Concat's own AI models.
    pub const AI_MODELS: i32 = 11;
}

/// Everything that can happen to the Extras sheet.
#[derive(Debug)]
pub enum ExtrasMsg {
    /// The sheet is asked for.
    Open,
    Close,
    PageChanged(i32),
    QueryEdited(String),
    /// Read every repository again, now.
    Refresh,
    /// The repositories' indexes, and a sentence for each that could not
    /// be read.
    Fetched(Vec<Fetched>, Vec<String>),
    /// A row was pressed: its page opens over the list.
    Opened(String),
    DetailClosed,
    Install(String),
    Update(String),
    UpdateAll,
    /// The person said yes to a large download.
    InstallConfirmed(Vec<String>),
    Remove(String),
    RemoveConfirmed(String),
    Enable(String),
    Disable(String),
    Cancel(String),
    InstallFile,
    FileChosen(PathBuf),
    FileInspected(PathBuf, Box<Result<Inspected, String>>),
    /// The person said yes to a file nothing vouches for.
    InstallFileConfirmed(PathBuf),
    OpenFolder,
    /// A running install's report.
    Progress {
        id: String,
        progress: Progress,
    },
    /// A running install is done.
    Finished {
        id: String,
        result: Box<Result<Record, String>>,
    },
}

/// One install under way.
#[derive(Debug)]
struct Job {
    name: String,
    progress: Progress,
    cancel: Arc<AtomicBool>,
}

/// One step waiting for its turn: from a repository, or a chosen file.
#[derive(Debug, Clone)]
enum Queued {
    Step(Step),
    File {
        id: String,
        name: String,
        path: PathBuf,
        accept_unverified: bool,
    },
}

impl Queued {
    fn id(&self) -> &str {
        match self {
            Queued::Step(step) => &step.id,
            Queued::File { id, .. } => id,
        }
    }
}

/// The Extras sheet's state.
#[derive(Default)]
pub struct ExtrasPane {
    pub open: bool,
    page: i32,
    query: String,
    /// The repositories' indexes, as last read.
    fetched: Vec<Fetched>,
    /// Why a repository could not be read, or is not asked.
    problems: Vec<String>,
    /// Asked for and not answered yet.
    refreshing: bool,
    /// Asked once this run; the sheet opening again uses the cache.
    loaded: bool,
    /// The extra whose page is open over the list.
    detail: Option<String>,
    /// The install running now, by id.
    jobs: BTreeMap<String, Job>,
    /// Waiting, in order.
    queue: VecDeque<Queued>,
    /// Why the last attempt at an extra failed.
    failures: HashMap<String, String>,
}

impl ExtrasPane {
    /// Applies one message.
    pub fn update(&mut self, msg: ExtrasMsg, studio: &mut Studio) {
        match msg {
            ExtrasMsg::Open => {
                self.open = true;
                if !self.loaded {
                    self.load(studio, false);
                }
            }
            ExtrasMsg::Close => {
                self.open = false;
                self.detail = None;
            }
            ExtrasMsg::PageChanged(page) => {
                self.page = page;
                self.detail = None;
            }
            ExtrasMsg::QueryEdited(query) => {
                self.query = query;
                self.detail = None;
            }
            ExtrasMsg::Refresh => self.load(studio, true),
            ExtrasMsg::Fetched(fetched, problems) => {
                self.refreshing = false;
                self.loaded = true;
                for one in &fetched {
                    log::info!(
                        "extras: {} offers {} extras (revision {}){}",
                        one.repository.name,
                        one.index.extras.len(),
                        one.index.revision,
                        if one.stale.is_some() {
                            ", from the copy read before"
                        } else {
                            ""
                        }
                    );
                }
                for problem in &problems {
                    log::info!("extras: {problem}");
                }
                self.fetched = fetched;
                self.problems = problems;
            }
            ExtrasMsg::Opened(id) => self.detail = Some(id),
            ExtrasMsg::DetailClosed => self.detail = None,
            ExtrasMsg::Install(id) | ExtrasMsg::Update(id) => self.plan(vec![id], false, studio),
            ExtrasMsg::UpdateAll => {
                let registry = host_extras::registry(&studio.host.dirs);
                let ids = self.updatable(&registry);
                self.plan(ids, false, studio);
            }
            ExtrasMsg::InstallConfirmed(ids) => self.plan(ids, true, studio),
            ExtrasMsg::Remove(id) => {
                let registry = host_extras::registry(&studio.host.dirs);
                let Some(record) = registry.get(&id) else {
                    return;
                };
                let needed_by = resolve::dependents(&id, &registry.installed());
                if !needed_by.is_empty() {
                    let names: Vec<&str> = needed_by
                        .iter()
                        .map(|other| {
                            registry
                                .get(other)
                                .map_or(other.as_str(), |r| r.name.as_str())
                        })
                        .collect();
                    studio.notify(
                        &tf("extras.neededBy", &[&record.name, &names.join(", ")]),
                        true,
                    );
                    return;
                }
                studio.confirm = Some(Confirm::ExtrasRemove {
                    id: id.clone(),
                    name: record.name.clone(),
                    in_use: in_use(studio, record),
                });
            }
            ExtrasMsg::RemoveConfirmed(id) => {
                let root = host_extras::dir(&studio.host.dirs);
                let name = host_extras::registry(&studio.host.dirs)
                    .get(&id)
                    .map(|record| record.name.clone())
                    .unwrap_or_else(|| id.clone());
                match concat_extras::install::uninstall(&root, &id, false) {
                    Ok(()) => {
                        log::info!("extras: removed {id}");
                        self.failures.remove(&id);
                        if self.detail.as_deref() == Some(&id) && self.entry(&id).is_none() {
                            self.detail = None;
                        }
                        reload(studio);
                        studio.notify(&tf("extras.removedNotice", &[&name]), false);
                    }
                    Err(error) => {
                        log::warn!("extras: removing {id} failed: {error}; it stays installed");
                        studio.notify(&error.to_string(), true);
                    }
                }
            }
            ExtrasMsg::Enable(id) => self.switch(&id, true, studio),
            ExtrasMsg::Disable(id) => self.switch(&id, false, studio),
            ExtrasMsg::Cancel(id) => {
                if let Some(job) = self.jobs.get(&id) {
                    job.cancel.store(true, Ordering::Relaxed);
                }
                self.queue.retain(|queued| queued.id() != id);
            }
            ExtrasMsg::InstallFile => {
                crate::host::pick_files_then(
                    t("extras.chooseFile"),
                    Some((t("extras.fileKind"), &["concatpkg"])),
                    move |studio, _, _, paths| {
                        if let Some(path) = paths.into_iter().next() {
                            studio.handle(Msg::Extras(ExtrasMsg::FileChosen(path)));
                        }
                    },
                );
            }
            ExtrasMsg::FileChosen(path) => {
                let fetched = self.fetched.clone();
                spawn(
                    move || {
                        let result = host_extras::inspect_file(&path, &fetched);
                        (path, result)
                    },
                    |studio, _, _, (path, result)| {
                        studio.handle(Msg::Extras(ExtrasMsg::FileInspected(
                            path,
                            Box::new(result),
                        )));
                    },
                );
            }
            ExtrasMsg::FileInspected(path, result) => match *result {
                Err(error) => {
                    log::warn!("extras: {} was not installed: {error}", path.display());
                    studio.notify(&tf("extras.fileRefused", &[&error]), true);
                }
                Ok(inspected) if inspected.vouched_by.is_some() => {
                    self.enqueue_file(&inspected, path, false, studio);
                }
                Ok(inspected) => {
                    studio.confirm = Some(Confirm::ExtrasUnverified {
                        path,
                        name: inspected.manifest.extra.name.clone(),
                        author: inspected.manifest.extra.author.clone(),
                    });
                }
            },
            ExtrasMsg::InstallFileConfirmed(path) => {
                // Read again: the answer came after a question, and the
                // file is only ever installed as what it is now.
                match concat_extras::archive::read_manifest(&path) {
                    Ok(manifest) => {
                        let inspected = Inspected {
                            manifest,
                            sha256: String::new(),
                            vouched_by: None,
                        };
                        self.enqueue_file(&inspected, path, true, studio);
                    }
                    Err(error) => studio.notify(&tf("extras.fileRefused", &[&error]), true),
                }
            }
            ExtrasMsg::OpenFolder => {
                let dir = host_extras::dir(&studio.host.dirs);
                let _ = std::fs::create_dir_all(dir.join("packages"));
                if let Err(error) = crate::platform::reveal(&dir.join("packages").to_string_lossy())
                {
                    studio.notify(&error, true);
                }
            }
            ExtrasMsg::Progress { id, progress } => {
                if let Some(job) = self.jobs.get_mut(&id) {
                    job.progress = progress;
                }
            }
            ExtrasMsg::Finished { id, result } => {
                let job = self.jobs.remove(&id);
                let name = job
                    .as_ref()
                    .map(|job| job.name.clone())
                    .unwrap_or_else(|| id.clone());
                match *result {
                    Ok(record) => {
                        log::info!(
                            "extras: installed {} {} ({:?})",
                            record.id,
                            record.version,
                            record.origin
                        );
                        self.failures.remove(&id);
                        reload(studio);
                        studio.notify(&tf("extras.installedNotice", &[&name]), false);
                    }
                    Err(error) if error == "cancelled" || error.contains("cancelled") => {
                        log::info!("extras: {id} cancelled");
                        // What waited on it cannot go in without it.
                        self.drop_dependents(&id);
                    }
                    Err(error) => {
                        log::warn!("extras: {id} was not installed: {error}");
                        self.failures.insert(id.clone(), error);
                        self.drop_dependents(&id);
                        studio.notify(&tf("extras.failedNotice", &[&name]), true);
                    }
                }
                self.pump(studio);
            }
        }
    }

    /// Reads every repository, on a worker.
    fn load(&mut self, studio: &Studio, force: bool) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        let dirs = studio.host.dirs.clone();
        spawn(
            move || host_extras::load_all(&dirs, force),
            |studio, _, _, (fetched, problems)| {
                studio.handle(Msg::Extras(ExtrasMsg::Fetched(fetched, problems)));
            },
        );
    }

    /// The entry an id is offered as, and the repository it comes from.
    fn entry(&self, id: &str) -> Option<(&Fetched, &Entry)> {
        host_extras::offered(&self.fetched)
            .into_iter()
            .find(|(_, entry)| entry.id == id)
    }

    fn updatable(&self, registry: &Registry) -> Vec<String> {
        registry
            .records()
            .iter()
            .filter(|record| {
                self.entry(&record.id)
                    .is_some_and(|(_, entry)| entry.version() > record.version())
            })
            .map(|record| record.id.clone())
            .collect()
    }

    /// Plans installing `ids`, asks first when it is large, and queues it.
    fn plan(&mut self, ids: Vec<String>, confirmed: bool, studio: &mut Studio) {
        if ids.is_empty() {
            return;
        }
        let registry = host_extras::registry(&studio.host.dirs);
        let offered = host_extras::offered(&self.fetched);
        let entries: Vec<&Entry> = offered.iter().map(|(_, entry)| *entry).collect();
        let requested: Vec<&str> = ids.iter().map(String::as_str).collect();
        let plan = match resolve::plan(&requested, &entries, &registry.installed()) {
            Ok(plan) => plan,
            Err(error) => {
                for id in &ids {
                    self.failures.insert(id.clone(), error.to_string());
                }
                return;
            }
        };
        if !plan.is_runnable() {
            let why: Vec<String> = plan
                .missing
                .iter()
                .map(|missing| missing.why.clone())
                .collect();
            for id in &ids {
                self.failures
                    .insert(id.clone(), tf("extras.cannotInstall", &[&why.join("; ")]));
            }
            return;
        }
        // An extra that cannot be used here is not downloaded.
        for step in &plan.steps {
            if let Some((_, entry)) = self.entry(&step.id)
                && step.action != Action::Enable
                && let Err(why) = entry.compatible(
                    concat_extras::APP_VERSION,
                    &concat_extras::current_platform(),
                )
            {
                self.failures.insert(step.id.clone(), why);
                return;
            }
        }
        let size = plan.download_size();
        if !confirmed && size > host_extras::CONFIRM_ABOVE {
            let names: Vec<String> = plan
                .steps
                .iter()
                .map(|step| {
                    self.entry(&step.id)
                        .map_or(step.id.clone(), |(_, e)| e.name.clone())
                })
                .collect();
            studio.confirm = Some(Confirm::ExtrasDownload {
                ids,
                names: names.join(", "),
                size: human_size(size),
            });
            return;
        }
        for step in plan.steps {
            let held =
                self.jobs.contains_key(&step.id) || self.queue.iter().any(|q| q.id() == step.id);
            if !held {
                self.failures.remove(&step.id);
                self.queue.push_back(Queued::Step(step));
            }
        }
        self.pump(studio);
    }

    fn enqueue_file(
        &mut self,
        inspected: &Inspected,
        path: PathBuf,
        accept_unverified: bool,
        studio: &mut Studio,
    ) {
        let id = inspected.manifest.extra.id.clone();
        if self.jobs.contains_key(&id) || self.queue.iter().any(|q| q.id() == id) {
            return;
        }
        self.failures.remove(&id);
        self.queue.push_back(Queued::File {
            id,
            name: inspected.manifest.extra.name.clone(),
            path,
            accept_unverified,
        });
        self.pump(studio);
    }

    /// Takes everything queued after a failed `id` that needs it off the
    /// queue, with the reason.
    fn drop_dependents(&mut self, id: &str) {
        let mut failed = BTreeSet::from([id.to_owned()]);
        let mut kept = VecDeque::new();
        for queued in self.queue.drain(..) {
            let needs_failed = match &queued {
                Queued::Step(step) => self
                    .fetched
                    .iter()
                    .find_map(|one| one.index.get(&step.id))
                    .is_some_and(|entry| entry.requires.iter().any(|r| failed.contains(&r.id))),
                Queued::File { .. } => false,
            };
            if needs_failed {
                failed.insert(queued.id().to_owned());
                self.failures
                    .insert(queued.id().to_owned(), tf("extras.neededFailed", &[&id]));
            } else {
                kept.push_back(queued);
            }
        }
        self.queue = kept;
    }

    /// Starts the next thing on the queue, when nothing is running.
    fn pump(&mut self, studio: &mut Studio) {
        while self.jobs.is_empty() {
            let Some(next) = self.queue.pop_front() else {
                return;
            };
            match next {
                Queued::Step(step) if step.action == Action::Enable => {
                    self.switch(&step.id, true, studio);
                }
                Queued::Step(step) => {
                    let Some((fetched, entry)) = self.entry(&step.id) else {
                        self.failures
                            .insert(step.id.clone(), t("extras.noLongerOffered"));
                        continue;
                    };
                    let (fetched, entry) = (fetched.clone(), entry.clone());
                    let dirs = studio.host.dirs.clone();
                    self.run(
                        step.id.clone(),
                        entry.name.clone(),
                        move |cancel, report| {
                            host_extras::install_entry(&dirs, &fetched, &entry, cancel, report)
                        },
                    );
                }
                Queued::File {
                    id,
                    name,
                    path,
                    accept_unverified,
                } => {
                    let dirs = studio.host.dirs.clone();
                    let fetched = self.fetched.clone();
                    self.run(id, name, move |cancel, report| {
                        host_extras::install_file(
                            &dirs,
                            &path,
                            &fetched,
                            accept_unverified,
                            cancel,
                            report,
                        )
                    });
                }
            }
        }
    }

    /// Runs one install on a worker, reporting at most once a percent.
    fn run(
        &mut self,
        id: String,
        name: String,
        work: impl FnOnce(&AtomicBool, &mut dyn FnMut(Progress)) -> Result<Record, String>
        + Send
        + 'static,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.insert(
            id.clone(),
            Job {
                name,
                progress: Progress::Downloading(0, 1),
                cancel: cancel.clone(),
            },
        );
        let reported = id.clone();
        spawn(
            move || {
                let mut last: Option<(u8, u32)> = None;
                work(&cancel, &mut |progress| {
                    let key = progress_key(progress);
                    if last == Some(key) {
                        return;
                    }
                    last = Some(key);
                    let id = reported.clone();
                    on_ui(move |studio, _, _| {
                        studio.handle(Msg::Extras(ExtrasMsg::Progress { id, progress }));
                    });
                })
            },
            move |studio, _, _, result| {
                studio.handle(Msg::Extras(ExtrasMsg::Finished {
                    id,
                    result: Box::new(result),
                }));
            },
        );
    }

    fn switch(&mut self, id: &str, enabled: bool, studio: &mut Studio) {
        let root = host_extras::dir(&studio.host.dirs);
        match concat_extras::install::set_enabled(&root, id, enabled, false) {
            Ok(()) => {
                log::info!(
                    "extras: {id} switched {}",
                    if enabled { "on" } else { "off" }
                );
                self.failures.remove(id);
                reload(studio);
            }
            Err(error) => studio.notify(&error.to_string(), true),
        }
    }

    // ── what the sheet shows ──

    /// The rows of the page, narrowed by the search.
    pub fn cards(&self, studio: &Studio) -> Vec<ExtraCardData> {
        let registry = host_extras::registry(&studio.host.dirs);
        let mut ids: Vec<String> = Vec::new();
        let mut push = |id: &str| {
            if !ids.iter().any(|held| held == id) {
                ids.push(id.to_owned());
            }
        };
        let offered = host_extras::offered(&self.fetched);
        match self.page {
            page::DISCOVER => {
                for (_, entry) in offered.iter().filter(|(_, e)| e.featured) {
                    push(&entry.id);
                }
                for (_, entry) in &offered {
                    push(&entry.id);
                }
            }
            page::INSTALLED => {
                for record in registry.records() {
                    push(&record.id);
                }
            }
            page::UPDATES => {
                for id in self.updatable(&registry) {
                    push(&id);
                }
            }
            page::DOWNLOADS => {
                for id in self.jobs.keys() {
                    push(id);
                }
                for queued in &self.queue {
                    push(queued.id());
                }
                let mut failed: Vec<&String> = self.failures.keys().collect();
                failed.sort();
                for id in failed {
                    push(id);
                }
            }
            page::AI_MODELS => {}
            shelf => {
                let category = shelf_category(shelf);
                for (_, entry) in offered.iter().filter(|(_, e)| Some(e.category) == category) {
                    push(&entry.id);
                }
                for record in registry
                    .records()
                    .iter()
                    .filter(|r| Some(r.category) == category)
                {
                    push(&record.id);
                }
            }
        }
        let query = self.query.trim();
        ids.iter()
            .filter_map(|id| {
                let entry = self.entry(id).map(|(_, entry)| entry);
                let record = registry.get(id);
                if !query.is_empty() {
                    let found = match (entry, record) {
                        (Some(entry), _) => entry.matches(query),
                        (None, Some(record)) => {
                            format!("{} {} {}", record.name, record.author, record.id)
                                .to_lowercase()
                                .contains(&query.to_lowercase())
                        }
                        (None, None) => false,
                    };
                    if !found {
                        return None;
                    }
                }
                Some(self.card(id, entry, record))
            })
            .collect()
    }

    fn card(&self, id: &str, entry: Option<&Entry>, record: Option<&Record>) -> ExtraCardData {
        let job = self.jobs.get(id);
        let queued = self.queue.iter().any(|q| q.id() == id);
        let incompatible = entry.and_then(|entry| {
            entry
                .compatible(
                    concat_extras::APP_VERSION,
                    &concat_extras::current_platform(),
                )
                .err()
        });
        let (state, badge, progress) = if let Some(job) = job {
            let (badge, progress) = describe(job.progress);
            (3, badge, progress)
        } else if queued {
            (3, t("extras.waiting"), 0.0)
        } else if self.failures.contains_key(id) {
            (4, t("extras.failed"), 0.0)
        } else if let Some(record) = record {
            match entry {
                Some(entry) if entry.version() > record.version() => {
                    (2, tf("extras.updateTo", &[&entry.version]), 0.0)
                }
                _ if !record.enabled => (6, t("extras.switchedOff"), 0.0),
                _ => (1, t("extras.installedBadge"), 0.0),
            }
        } else if incompatible.is_some() {
            (5, t("extras.notForThisConcat"), 0.0)
        } else {
            (0, t("extras.free"), 0.0)
        };
        let problem = self
            .failures
            .get(id)
            .cloned()
            .or_else(|| (record.is_none()).then(|| incompatible.clone()).flatten())
            .unwrap_or_default();
        let name = entry
            .map(|e| e.name.clone())
            .or_else(|| record.map(|r| r.name.clone()))
            .unwrap_or_else(|| id.to_owned());
        let author = entry
            .map(|e| e.author.clone())
            .or_else(|| record.map(|r| r.author.clone()))
            .unwrap_or_default();
        let version = match (entry, record) {
            (_, Some(record)) => record.version.clone(),
            (Some(entry), None) => entry.version.clone(),
            (None, None) => String::new(),
        };
        let size = match (entry, record) {
            (_, Some(record)) => human_size(record.size),
            (Some(entry), None) => human_size(entry.download.size),
            (None, None) => String::new(),
        };
        let category = entry
            .map(|e| e.category)
            .or_else(|| record.map(|r| r.category))
            .unwrap_or_default();
        let verified = record.is_none_or(|record| record.origin.verified());
        ExtraCardData {
            id: id.into(),
            name: name.into(),
            summary: entry.map(|e| e.summary.clone()).unwrap_or_default().into(),
            meta: [author, version, size]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ")
                .into(),
            category: category_label(category).into(),
            state,
            badge: badge.into(),
            progress,
            problem: problem.into(),
            verified,
        }
    }

    fn detail_data(&self, studio: &Studio) -> ExtraDetailData {
        let Some(id) = &self.detail else {
            return ExtraDetailData::default();
        };
        let registry = host_extras::registry(&studio.host.dirs);
        let entry = self.entry(id).map(|(_, entry)| entry);
        let record = registry.get(id);
        if entry.is_none() && record.is_none() {
            return ExtraDetailData::default();
        }
        let card = self.card(id, entry, record);
        let working = card.state == 3;
        let mut facts = Vec::new();
        if let Some(licence) = entry.map(|e| e.licence.clone()) {
            facts.push(tf("extras.factLicence", &[&licence]));
        }
        if let Some(record) = record {
            facts.push(tf(
                "extras.factInstalled",
                &[&record.version, &human_size(record.size)],
            ));
        }
        if let Some(entry) = entry {
            facts.push(tf(
                "extras.factOffered",
                &[&entry.version, &human_size(entry.download.size)],
            ));
        }
        let kinds: Vec<Kind> = match (entry, record) {
            (Some(entry), _) => entry.provides.clone(),
            (None, Some(record)) => record.provides.iter().map(|p| p.kind).collect(),
            (None, None) => Vec::new(),
        };
        let mut provides: Vec<String> = kinds.iter().map(|kind| kind_label(*kind)).collect();
        let ids: Vec<String> = record
            .map(|r| r.provides_ids.clone())
            .or_else(|| entry.map(|e| e.provides_ids.clone()))
            .unwrap_or_default();
        if !ids.is_empty() {
            provides.push(tf("extras.providesIds", &[&few(&ids)]));
        }
        let requires: Vec<String> = entry
            .map(|e| e.requires.clone())
            .or_else(|| record.map(|r| r.requires.clone()))
            .unwrap_or_default()
            .iter()
            .map(|requirement| {
                let have = registry
                    .get(&requirement.id)
                    .map(|held| held.version.clone());
                let what = format!("{} {}", requirement.id, requirement.version)
                    .trim()
                    .to_owned();
                let state = match have {
                    Some(version) => tf("extras.requireHere", &[&version]),
                    None => t("extras.requireMissing"),
                };
                if requirement.reason.is_empty() {
                    format!("{what} - {state}")
                } else {
                    format!("{what} - {state}. {}", requirement.reason)
                }
            })
            .collect();
        let origin = match record.map(|r| &r.origin) {
            Some(concat_extras::registry::Origin::Repository { name }) => {
                tf("extras.fromRepository", &[name])
            }
            Some(concat_extras::registry::Origin::File { verified: true }) => {
                t("extras.fromVerifiedFile")
            }
            Some(concat_extras::registry::Origin::File { verified: false }) => {
                t("extras.fromUnverifiedFile")
            }
            None => self
                .entry(id)
                .map(|(fetched, _)| tf("extras.offeredBy", &[&fetched.repository.name]))
                .unwrap_or_default(),
        };
        let compatible = card.state != 5;
        ExtraDetailData {
            open: true,
            description: entry
                .map(|e| e.description.clone())
                .unwrap_or_default()
                .into(),
            facts: facts.join("\n").into(),
            provides: provides.join("\n").into(),
            requires: requires.join("\n").into(),
            origin: origin.into(),
            changelog: entry
                .map(|e| e.changelog.clone())
                .unwrap_or_default()
                .into(),
            homepage: entry.map(|e| e.homepage.clone()).unwrap_or_default().into(),
            can_install: !working && record.is_none() && entry.is_some() && compatible,
            can_update: !working && card.state == 2,
            can_remove: !working && record.is_some(),
            can_enable: !working && record.is_some_and(|r| !r.enabled),
            can_disable: !working && record.is_some_and(|r| r.enabled),
            can_cancel: working,
            card,
        }
    }

    /// The sheet's state, as Slint shows it.
    pub fn data(&self, studio: &Studio) -> ExtrasData {
        let registry = host_extras::registry(&studio.host.dirs);
        let mut status: Vec<String> = Vec::new();
        for one in &self.fetched {
            let mut line = tf(
                "extras.statusRepository",
                &[
                    &one.repository.name,
                    &one.index.extras.len(),
                    &one.index.revision,
                ],
            );
            if let Some(why) = &one.stale {
                line.push_str(&tf("extras.statusStale", &[why]));
            }
            status.push(line);
        }
        status.extend(self.problems.iter().cloned());
        if self.refreshing {
            status.insert(0, t("extras.reading"));
        }
        let trouble = !self.refreshing && self.loaded && self.fetched.is_empty();
        let empty = if !self.query.trim().is_empty() {
            tf("extras.emptySearch", &[&self.query.trim()])
        } else {
            match self.page {
                page::INSTALLED => t("extras.emptyInstalled"),
                page::UPDATES => t("extras.emptyUpdates"),
                page::DOWNLOADS => t("extras.emptyDownloads"),
                _ if self.refreshing => t("extras.reading"),
                _ if self.fetched.is_empty() => t("extras.emptyNoRepository"),
                _ => t("extras.emptyShelf"),
            }
        };
        ExtrasData {
            open: self.open,
            page: self.page,
            query: self.query.as_str().into(),
            status: status.join(" · ").into(),
            trouble,
            refreshing: self.refreshing,
            updates: self.updatable(&registry).len() as i32,
            working: (self.jobs.len() + self.queue.len()) as i32,
            empty: empty.into(),
            detail: self.detail_data(studio),
        }
    }
}

/// Reloads everything that reads extras: the effects catalogue (and so its
/// cards), the fonts the picker offers, the title presets.
fn reload(studio: &mut Studio) {
    studio.reload_packages(false);
    studio.refresh_art();
    studio.installed_fonts = crate::presets::installed_fonts(&studio.host.dirs);
    studio.text_presets = crate::presets::all(&studio.host.dirs);
    refresh_extra_content(studio);
}

/// The picture types the Stickers page offers: what the media pipeline
/// imports as a still and a card can draw.
pub const STICKER_TYPES: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// Reads what the enabled extras bring to the Stickers page and the
/// Audio page again: the pictures at once, the sounds' lengths on a worker,
/// since each is a file to open. At start, and after anything changes what
/// is installed or switched on.
pub fn refresh_extra_content(studio: &mut Studio) {
    let dirs = studio.host.dirs.clone();
    studio.extra_stickers = host_extras::extra_files(&dirs, Kind::Stickers, &STICKER_TYPES);
    // Every card is drawn again: an update keeps a picture's id and can
    // change the picture.
    studio.sticker_art.borrow_mut().clear();
    // Only the latest reading lands: two changes in quick succession start
    // two workers, and the first to finish may be the older.
    let asked = SOUNDS_ASKED.fetch_add(1, Ordering::Relaxed) + 1;
    spawn(
        move || {
            host_extras::extra_files(&dirs, Kind::Sounds, &concat_host::sounds::EXTENSIONS)
                .into_iter()
                .map(|file| {
                    let duration = concat_host::media::probe(&file.path.to_string_lossy())
                        .ok()
                        .and_then(|summary| summary.duration)
                        .unwrap_or(0.0);
                    (file, duration)
                })
                .collect::<Vec<_>>()
        },
        move |studio, _, _, sounds| {
            if SOUNDS_ASKED.load(Ordering::Relaxed) == asked {
                studio.sounds.extras = sounds;
            }
        },
    );
}

/// How many times the extras' sounds have been asked for; see
/// [`refresh_extra_content`].
static SOUNDS_ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl Studio {
    /// The Stickers page's pictures, each drawn once and kept.
    pub fn sticker_cards(&self) -> Vec<crate::ui::StickerCardData> {
        let mut art = self.sticker_art.borrow_mut();
        self.extra_stickers
            .iter()
            .map(|file| {
                let picture = art
                    .entry(file.id.clone())
                    .or_insert_with(|| {
                        slint::Image::load_from_path(&file.path).unwrap_or_else(|_| {
                            log::warn!(
                                "extras: the sticker {} could not be drawn; its card is blank",
                                file.path.display()
                            );
                            slint::Image::default()
                        })
                    })
                    .clone();
                crate::ui::StickerCardData {
                    id: file.id.as_str().into(),
                    name: file.name.as_str().into(),
                    extra: file.extra.as_str().into(),
                    picture,
                }
            })
            .collect()
    }

    /// A picture from the Stickers page, placed at the playhead on the
    /// first lane free over it: copied into the project's `stickers`
    /// folder and imported first, so the project keeps it when the extra
    /// is removed. Selects what it made.
    pub fn place_sticker(&mut self, id: &str) {
        let Some(file) = self
            .extra_stickers
            .iter()
            .find(|file| file.id == id)
            .cloned()
        else {
            return;
        };
        let media_id = match self.adopt_file(&file.name, &file.path, "stickers") {
            Ok(media_id) => media_id,
            Err(error) => {
                self.notify(&error, true);
                return;
            }
        };
        self.close_source();
        let start = f64::from(self.playhead.max(0.0));
        if let Some(clip) =
            self.apply(concat_project::Command::AddClipAtFirstFree { media_id, start })
        {
            log::info!("edit: sticker {id} placed at {start:.2}s");
            self.selection = vec![clip];
        }
    }
}

/// What the open project uses from `record`, named: its effects, and the
/// fonts its titles read from the extra's folder.
fn in_use(studio: &Studio, record: &Record) -> String {
    let Some(session) = studio.session.as_ref() else {
        return String::new();
    };
    let project = session.project();
    let used = used_effect_ids(project);
    let mut names: Vec<String> = record
        .provides_ids
        .iter()
        .filter(|id| used.contains(id.as_str()))
        .cloned()
        .collect();
    let folder = host_extras::registry(&studio.host.dirs).package_dir(&record.id);
    for font in &project.fonts {
        if std::path::Path::new(&font.path).starts_with(&folder) && !names.contains(&font.family) {
            names.push(font.family.clone());
        }
    }
    few(&names)
}

/// Every effect and transition id a project names, anywhere.
pub fn used_effect_ids(project: &concat_project::model::Project) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for timeline in &project.timelines {
        for clip in &timeline.clips {
            for link in clip.filters.iter().chain(&clip.video_effects) {
                ids.insert(link.id.clone());
            }
            if let Some(transition) = &clip.transition_in {
                ids.insert(transition.id.clone());
            }
        }
    }
    ids
}

/// For a project that opens: the extras that bring effects it uses and the
/// catalogue does not have - one installed but switched off, or one a
/// repository's cached index offers - by name, with the ids they would
/// bring. No network: only the install database and the cache are read.
pub fn missing_extras(
    project: &concat_project::model::Project,
    dirs: &concat_host::AppDirs,
) -> Vec<(String, Vec<String>)> {
    let catalogue = concat_effects::Catalogue::builtin();
    let missing: Vec<String> = used_effect_ids(project)
        .into_iter()
        .filter(|id| catalogue.get(id).is_none())
        .collect();
    if missing.is_empty() {
        return Vec::new();
    }
    let registry = host_extras::registry(dirs);
    let cached = host_extras::cached_indexes(dirs);
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for id in missing {
        let name = registry
            .provider_of(&id)
            .map(|record| record.name.clone())
            .or_else(|| {
                cached
                    .iter()
                    .find_map(|one| one.index.provider_of(&id).map(|entry| entry.name.clone()))
            });
        if let Some(name) = name {
            match out.iter_mut().find(|(held, _)| *held == name) {
                Some((_, ids)) => ids.push(id),
                None => out.push((name, vec![id])),
            }
        }
    }
    out
}

/// The shelf a page shows.
fn shelf_category(page: i32) -> Option<Category> {
    let index = usize::try_from(page - page::FIRST_SHELF).ok()?;
    Category::ALL.get(index).copied()
}

/// A shelf's name in the reader's language.
pub fn category_label(category: Category) -> String {
    // A literal call each, so scripts/locales.py sees every key.
    match category {
        Category::Effects => t("extras.shelfEffects"),
        Category::Transitions => t("extras.shelfTransitions"),
        Category::Fonts => t("extras.shelfFonts"),
        Category::Titles => t("extras.shelfTitles"),
        Category::Stickers => t("extras.shelfStickers"),
        Category::Audio => t("extras.shelfAudio"),
        Category::Other => t("extras.shelfOther"),
    }
}

fn kind_label(kind: Kind) -> String {
    match kind {
        Kind::Effects => t("extras.kindEffects"),
        Kind::Fonts => t("extras.kindFonts"),
        Kind::TitlePresets => t("extras.kindTitlePresets"),
        Kind::Stickers => t("extras.kindStickers"),
        Kind::Sounds => t("extras.kindSounds"),
    }
}

/// What a running install's badge says, and how far its bar is.
fn describe(progress: Progress) -> (String, f32) {
    match progress {
        Progress::Downloading(done, all) => {
            let fraction = (done as f64 / all.max(1) as f64).clamp(0.0, 1.0) as f32;
            (
                tf(
                    "extras.downloading",
                    &[&((fraction * 100.0).round() as u32)],
                ),
                fraction * 0.8,
            )
        }
        Progress::Verifying => (t("extras.verifying"), 0.82),
        Progress::Unpacking(done, all) => {
            let fraction = (done as f64 / all.max(1) as f64).clamp(0.0, 1.0) as f32;
            (t("extras.unpacking"), 0.84 + fraction * 0.12)
        }
        Progress::Checking => (t("extras.checking"), 0.97),
        Progress::Finishing => (t("extras.finishing"), 0.99),
    }
}

/// A key that changes at most once a percent and once a phase, so a
/// worker's reports are not one per chunk.
fn progress_key(progress: Progress) -> (u8, u32) {
    let percent = |done: u64, all: u64| ((done as f64 / all.max(1) as f64) * 100.0) as u32;
    match progress {
        Progress::Downloading(done, all) => (0, percent(done, all)),
        Progress::Verifying => (1, 0),
        Progress::Unpacking(done, all) => (2, percent(done, all)),
        Progress::Checking => (3, 0),
        Progress::Finishing => (4, 0),
    }
}

/// The first three, and how many more.
fn few(names: &[String]) -> String {
    match names.len() {
        0..=3 => names.join(", "),
        count => format!("{} +{}", names[..3].join(", "), count - 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shelves_follow_the_categories_in_order() {
        assert_eq!(shelf_category(page::FIRST_SHELF), Some(Category::Effects));
        assert_eq!(shelf_category(page::FIRST_SHELF + 6), Some(Category::Other));
        assert_eq!(shelf_category(page::AI_MODELS), None);
        assert_eq!(shelf_category(page::DISCOVER), None);
    }

    #[test]
    fn progress_reports_once_a_percent_and_a_phase() {
        assert_eq!(progress_key(Progress::Downloading(10, 1000)), (0, 1));
        assert_eq!(progress_key(Progress::Downloading(14, 1000)), (0, 1));
        assert_ne!(progress_key(Progress::Downloading(20, 1000)), (0, 1));
        assert_ne!(
            progress_key(Progress::Verifying),
            progress_key(Progress::Checking)
        );
        let (_, start) = describe(Progress::Downloading(0, 10));
        let (_, end) = describe(Progress::Finishing);
        assert!(start < end && end < 1.0);
    }

    #[test]
    fn a_long_list_is_named_by_its_first_three() {
        let names: Vec<String> = ["a", "b", "c", "d", "e"].map(str::to_owned).to_vec();
        assert_eq!(few(&names), "a, b, c +2");
        assert_eq!(few(&names[..2]), "a, b");
    }
}
