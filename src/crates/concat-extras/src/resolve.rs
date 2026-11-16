// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! What installing something means: the extras asked for, every extra they
//! need that is not installed at a version that will do, in an order where
//! each comes after what it needs - and what cannot be had, said before
//! anything is downloaded.
//!
//! A plan never downgrades: an installed extra newer than what a repository
//! offers is left as it is. Upgrading an extra that another installed extra
//! needs at a version range the upgrade leaves is reported as a conflict,
//! not done. A cycle is refused outright.

use std::collections::{BTreeMap, HashMap};

use crate::Error;
use crate::index::Entry;
use crate::manifest::Requirement;

/// One installed extra, as far as a plan cares.
#[derive(Clone, Debug)]
pub struct Installed {
    /// Its version.
    pub version: semver::Version,
    /// What it needs.
    pub requires: Vec<Requirement>,
    /// Whether it is switched on.
    pub enabled: bool,
}

/// What a step does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Not installed: install it.
    Install,
    /// Installed at an older version: replace it.
    Upgrade {
        /// The version installed now.
        from: semver::Version,
    },
    /// Installed at this version and asked for by name: install it again
    /// over itself, which is how a damaged one is repaired.
    Reinstall,
    /// Installed at a version that will do, but switched off.
    Enable,
}

/// One extra the plan installs, upgrades or switches on.
#[derive(Clone, Debug)]
pub struct Step {
    /// The extra.
    pub id: String,
    /// The version it will be at.
    pub version: semver::Version,
    /// What happens to it.
    pub action: Action,
    /// The extra that needs it, when it was not asked for itself.
    pub wanted_by: Option<String>,
    /// Why, in the needer's words.
    pub reason: String,
    /// Its archive's size, for the total the sheet shows; 0 for Enable.
    pub download_size: u64,
}

/// Something the plan needs and cannot get.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    /// The extra whose need it is.
    pub for_id: String,
    /// What it needs.
    pub id: String,
    /// The versions that would do.
    pub range: String,
    /// Why it cannot be had.
    pub why: String,
}

/// An install plan.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// What to do, dependencies first.
    pub steps: Vec<Step>,
    /// What cannot be had. A plan with anything here must not be run.
    pub missing: Vec<Missing>,
}

impl Plan {
    /// What the plan downloads, in bytes.
    pub fn download_size(&self) -> u64 {
        self.steps.iter().map(|step| step.download_size).sum()
    }

    /// Whether the plan can run.
    pub fn is_runnable(&self) -> bool {
        self.missing.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Visiting,
    Done,
}

struct Planner<'a> {
    available: HashMap<&'a str, &'a Entry>,
    installed: &'a BTreeMap<String, Installed>,
    marks: HashMap<String, Mark>,
    /// The version each extra will be at once the plan has run, for the
    /// ones the plan touches.
    planned: HashMap<String, semver::Version>,
    stack: Vec<String>,
    plan: Plan,
}

/// Plans installing `requested` from `available` (every repository's
/// entries, one per id) over what is `installed`.
pub fn plan(
    requested: &[&str],
    available: &[&Entry],
    installed: &BTreeMap<String, Installed>,
) -> Result<Plan, Error> {
    let mut planner = Planner {
        available: available
            .iter()
            .map(|entry| (entry.id.as_str(), *entry))
            .collect(),
        installed,
        marks: HashMap::new(),
        planned: HashMap::new(),
        stack: Vec::new(),
        plan: Plan::default(),
    };
    for id in requested {
        planner.visit(id, None, &semver::VersionReq::STAR, "", true)?;
    }
    planner.check_dependents();
    Ok(planner.plan)
}

impl Planner<'_> {
    fn visit(
        &mut self,
        id: &str,
        wanted_by: Option<&str>,
        range: &semver::VersionReq,
        reason: &str,
        explicit: bool,
    ) -> Result<(), Error> {
        match self.marks.get(id) {
            Some(Mark::Visiting) => {
                let mut cycle = self.stack.clone();
                cycle.push(id.to_owned());
                return Err(Error::Dependency(format!(
                    "these extras need each other in a circle, so none can be installed first: {}",
                    cycle.join(" → ")
                )));
            }
            Some(Mark::Done) => {
                // Planned or settled already; check it suits this need too.
                let version = self
                    .planned
                    .get(id)
                    .cloned()
                    .or_else(|| self.installed.get(id).map(|held| held.version.clone()));
                if let Some(version) = version
                    && !range.matches(&version)
                {
                    self.missing(wanted_by, id, range, format!("{id} {version} is what the plan has, and that is not a version that will do"));
                }
                return Ok(());
            }
            None => {}
        }
        let held = self.installed.get(id);
        // A dependency already installed at a version that will do is left
        // alone - or only switched on.
        if !explicit
            && let Some(held) = held
            && range.matches(&held.version)
        {
            self.marks.insert(id.to_owned(), Mark::Done);
            if !held.enabled {
                self.plan.steps.push(Step {
                    id: id.to_owned(),
                    version: held.version.clone(),
                    action: Action::Enable,
                    wanted_by: wanted_by.map(str::to_owned),
                    reason: reason.to_owned(),
                    download_size: 0,
                });
            }
            return Ok(());
        }
        let Some(entry) = self.available.get(id).copied() else {
            let why = if held.is_some() {
                format!(
                    "the installed {id} is not a version that will do, and no repository offers another"
                )
            } else {
                format!("no repository offers {id}")
            };
            self.missing(wanted_by, id, range, why);
            self.marks.insert(id.to_owned(), Mark::Done);
            return Ok(());
        };
        let version = entry.version();
        if !range.matches(&version) {
            self.missing(
                wanted_by,
                id,
                range,
                format!(
                    "the repository offers {id} {version}, which is not a version that will do"
                ),
            );
            self.marks.insert(id.to_owned(), Mark::Done);
            return Ok(());
        }
        let action = match held {
            None => Action::Install,
            Some(held) if held.version < version => Action::Upgrade {
                from: held.version.clone(),
            },
            Some(held) if held.version == version => Action::Reinstall,
            Some(held) => {
                // Newer than the repository's: never downgraded.
                self.marks.insert(id.to_owned(), Mark::Done);
                if !held.enabled {
                    self.plan.steps.push(Step {
                        id: id.to_owned(),
                        version: held.version.clone(),
                        action: Action::Enable,
                        wanted_by: wanted_by.map(str::to_owned),
                        reason: reason.to_owned(),
                        download_size: 0,
                    });
                }
                return Ok(());
            }
        };

        self.marks.insert(id.to_owned(), Mark::Visiting);
        self.stack.push(id.to_owned());
        for requirement in &entry.requires {
            let range = requirement.range()?;
            self.visit(
                &requirement.id,
                Some(id),
                &range,
                &requirement.reason,
                false,
            )?;
        }
        self.stack.pop();
        self.marks.insert(id.to_owned(), Mark::Done);
        self.planned.insert(id.to_owned(), version.clone());
        self.plan.steps.push(Step {
            id: id.to_owned(),
            version,
            action,
            wanted_by: wanted_by.map(str::to_owned),
            reason: reason.to_owned(),
            download_size: entry.download.size,
        });
        Ok(())
    }

    fn missing(
        &mut self,
        wanted_by: Option<&str>,
        id: &str,
        range: &semver::VersionReq,
        why: String,
    ) {
        self.plan.missing.push(Missing {
            for_id: wanted_by.unwrap_or(id).to_owned(),
            id: id.to_owned(),
            range: range.to_string(),
            why,
        });
    }

    /// An upgrade must not leave an installed extra the plan does not
    /// touch without a version it needs.
    fn check_dependents(&mut self) {
        let mut broken = Vec::new();
        for (dependent, held) in self.installed {
            if self.planned.contains_key(dependent) {
                continue;
            }
            for requirement in &held.requires {
                let Some(version) = self.planned.get(&requirement.id) else {
                    continue;
                };
                let Ok(range) = requirement.range() else {
                    continue;
                };
                if !range.matches(version) {
                    broken.push(Missing {
                        for_id: dependent.clone(),
                        id: requirement.id.clone(),
                        range: range.to_string(),
                        why: format!(
                            "upgrading {} to {version} would break {dependent}, which needs {range}",
                            requirement.id
                        ),
                    });
                }
            }
        }
        self.plan.missing.extend(broken);
    }
}

/// The installed, enabled extras that need `id`: what removing or
/// switching it off would break.
pub fn dependents(id: &str, installed: &BTreeMap<String, Installed>) -> Vec<String> {
    installed
        .iter()
        .filter(|(other, held)| {
            other.as_str() != id && held.enabled && held.requires.iter().any(|r| r.id == id)
        })
        .map(|(other, _)| other.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Download;
    use crate::manifest::{Category, Kind};

    fn entry(id: &str, version: &str, requires: &[(&str, &str)]) -> Entry {
        Entry {
            id: id.to_owned(),
            name: id.to_owned(),
            version: version.to_owned(),
            summary: String::new(),
            description: String::new(),
            author: "A".to_owned(),
            licence: "MIT".to_owned(),
            homepage: String::new(),
            category: Category::Other,
            min_app: String::new(),
            platforms: Vec::new(),
            keywords: Vec::new(),
            provides: vec![Kind::Effects],
            provides_ids: Vec::new(),
            requires: requires
                .iter()
                .map(|(id, version)| Requirement {
                    id: (*id).to_owned(),
                    version: (*version).to_owned(),
                    reason: format!("{id} is needed"),
                })
                .collect(),
            installed_size: 0,
            featured: false,
            changelog: String::new(),
            download: Download {
                url: "x.concatpkg".to_owned(),
                size: 100,
                sha256: String::new(),
            },
        }
    }

    fn held(version: &str, requires: &[(&str, &str)], enabled: bool) -> Installed {
        Installed {
            version: semver::Version::parse(version).unwrap(),
            requires: requires
                .iter()
                .map(|(id, version)| Requirement {
                    id: (*id).to_owned(),
                    version: (*version).to_owned(),
                    reason: String::new(),
                })
                .collect(),
            enabled,
        }
    }

    fn ids(plan: &Plan) -> Vec<&str> {
        plan.steps.iter().map(|step| step.id.as_str()).collect()
    }

    #[test]
    fn no_dependencies_is_one_step() {
        let a = entry("a.a", "1.0.0", &[]);
        let plan = super::plan(&["a.a"], &[&a], &BTreeMap::new()).unwrap();
        assert_eq!(ids(&plan), ["a.a"]);
        assert_eq!(plan.steps[0].action, Action::Install);
        assert!(plan.is_runnable());
        assert_eq!(plan.download_size(), 100);
    }

    #[test]
    fn dependencies_come_first_once_each() {
        let base = entry("a.base", "1.2.0", &[]);
        let mid = entry("a.mid", "1.0.0", &[("a.base", "^1.1")]);
        let top = entry("a.top", "1.0.0", &[("a.mid", ""), ("a.base", ">=1.0")]);
        let plan = super::plan(&["a.top"], &[&top, &mid, &base], &BTreeMap::new()).unwrap();
        assert_eq!(ids(&plan), ["a.base", "a.mid", "a.top"]);
        assert_eq!(plan.steps[0].wanted_by.as_deref(), Some("a.mid"));
        assert_eq!(plan.steps[0].reason, "a.base is needed");
    }

    #[test]
    fn an_installed_dependency_that_will_do_is_left_alone() {
        let base = entry("a.base", "2.0.0", &[]);
        let top = entry("a.top", "1.0.0", &[("a.base", "^1")]);
        let installed = BTreeMap::from([("a.base".to_owned(), held("1.4.0", &[], true))]);
        let plan = super::plan(&["a.top"], &[&top, &base], &installed).unwrap();
        assert_eq!(ids(&plan), ["a.top"], "1.4.0 satisfies ^1; no upgrade to 2");

        let installed = BTreeMap::from([("a.base".to_owned(), held("1.4.0", &[], false))]);
        let plan = super::plan(&["a.top"], &[&top, &base], &installed).unwrap();
        assert_eq!(ids(&plan), ["a.base", "a.top"]);
        assert_eq!(plan.steps[0].action, Action::Enable);
    }

    #[test]
    fn a_missing_dependency_is_reported_not_guessed() {
        let top = entry("a.top", "1.0.0", &[("a.gone", "")]);
        let plan = super::plan(&["a.top"], &[&top], &BTreeMap::new()).unwrap();
        assert!(!plan.is_runnable());
        assert_eq!(plan.missing[0].id, "a.gone");
        assert_eq!(plan.missing[0].for_id, "a.top");
        assert!(plan.missing[0].why.contains("no repository"));

        let old = entry("a.base", "1.0.0", &[]);
        let top = entry("a.top", "1.0.0", &[("a.base", ">=2")]);
        let plan = super::plan(&["a.top"], &[&top, &old], &BTreeMap::new()).unwrap();
        assert!(plan.missing[0].why.contains("not a version that will do"));
    }

    #[test]
    fn a_cycle_is_refused() {
        let a = entry("a.a", "1.0.0", &[("a.b", "")]);
        let b = entry("a.b", "1.0.0", &[("a.c", "")]);
        let c = entry("a.c", "1.0.0", &[("a.a", "")]);
        let error = super::plan(&["a.a"], &[&a, &b, &c], &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains("a.a → a.b → a.c → a.a"), "{error}");
    }

    #[test]
    fn upgrades_reinstalls_and_never_a_downgrade() {
        let a = entry("a.a", "2.0.0", &[]);
        let installed = BTreeMap::from([("a.a".to_owned(), held("1.0.0", &[], true))]);
        let plan1 = super::plan(&["a.a"], &[&a], &installed).unwrap();
        assert!(matches!(plan1.steps[0].action, Action::Upgrade { .. }));

        let installed = BTreeMap::from([("a.a".to_owned(), held("2.0.0", &[], true))]);
        let plan2 = super::plan(&["a.a"], &[&a], &installed).unwrap();
        assert_eq!(plan2.steps[0].action, Action::Reinstall);

        let installed = BTreeMap::from([("a.a".to_owned(), held("3.0.0", &[], true))]);
        let plan3 = super::plan(&["a.a"], &[&a], &installed).unwrap();
        assert!(plan3.steps.is_empty(), "the newer one installed stays");
    }

    #[test]
    fn an_upgrade_that_breaks_an_installed_dependent_is_a_conflict() {
        let base = entry("a.base", "2.0.0", &[]);
        let installed = BTreeMap::from([
            ("a.base".to_owned(), held("1.0.0", &[], true)),
            (
                "a.user".to_owned(),
                held("1.0.0", &[("a.base", "^1")], true),
            ),
        ]);
        let plan = super::plan(&["a.base"], &[&base], &installed).unwrap();
        assert!(!plan.is_runnable());
        assert_eq!(plan.missing[0].for_id, "a.user");
        assert!(plan.missing[0].why.contains("would break a.user"));
    }

    #[test]
    fn dependents_are_the_enabled_extras_that_need_it() {
        let installed = BTreeMap::from([
            ("a.base".to_owned(), held("1.0.0", &[], true)),
            ("a.one".to_owned(), held("1.0.0", &[("a.base", "")], true)),
            ("a.two".to_owned(), held("1.0.0", &[("a.base", "")], false)),
        ]);
        assert_eq!(dependents("a.base", &installed), ["a.one"]);
        assert!(dependents("a.one", &installed).is_empty());
    }
}
