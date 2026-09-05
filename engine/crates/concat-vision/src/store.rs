// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where masks live between runs.
//!
//! One directory per media file per model, inside the project's `cache/`
//! folder so it travels with the project and vanishes with it, holding one
//! PNG per analysed source instant, named by that instant in milliseconds.
//! The host writes them as it analyses; the renderer opens the directory,
//! reads the names, and asks for the mask nearest a source time. Nothing
//! is ever indexed: the file names are the index.
//!
//! Decoded masks are kept in a process-wide cache, since a preview asks for
//! the same handful over and over while the playhead sits still, and an
//! export asks for each one several frames running.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use concat_project::model::{Cutout, CutoutMode};

use crate::{MASK_RATE, MODEL_ID, Mask, strokes};

/// A mask further than this from the instant asked for is nobody's answer:
/// two and a half analysis steps, so a gap in the cache reads as "not yet"
/// rather than as the wrong frame.
const REACH_MS: u64 = 2500 / MASK_RATE as u64;

/// How many decoded masks and how many resolved ones stay in memory. A
/// mask is 64 KB; both together stay under forty megabytes.
const CACHED: usize = 300;

/// The directory a media file's masks live in under `project`.
///
/// Named by a hash of the path rather than the path, like the peaks cache
/// and for the same reason: a flat, portable name that cannot escape the
/// folder. The model's id is in it too, so a different model never
/// answers for this one.
pub fn mask_dir(project: &Path, media_path: &str) -> PathBuf {
    project
        .join("cache")
        .join("masks")
        .join(format!("{:016x}-{MODEL_ID}", fnv1a(media_path.as_bytes())))
}

/// The file for the mask at `millis` of source.
pub fn mask_file(dir: &Path, millis: u64) -> PathBuf {
    dir.join(format!("{millis:09}.png"))
}

/// FNV-1a 64, for names that outlive the process.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// One media file's masks, as found on disk when it was opened.
#[derive(Clone, Debug)]
pub struct MaskStore {
    dir: PathBuf,
    /// The analysed instants, ascending, in milliseconds of source.
    times: Vec<u64>,
}

impl MaskStore {
    /// Reads the directory's names. A directory that is not there is a
    /// store with nothing in it, not an error.
    pub fn open(dir: &Path) -> MaskStore {
        let mut times: Vec<u64> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;
                name.strip_suffix(".png")?.parse::<u64>().ok()
            })
            .collect();
        times.sort_unstable();
        times.dedup();
        MaskStore {
            dir: dir.to_path_buf(),
            times,
        }
    }

    /// Where the store is.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// How many instants are analysed.
    pub fn len(&self) -> usize {
        self.times.len()
    }

    /// True with nothing analysed.
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// The analysed instant nearest `seconds` of source, when one is within
    /// reach. A store of one mask answers for every instant: a still has
    /// one frame and that frame is the answer everywhere.
    pub fn nearest_millis(&self, seconds: f64) -> Option<u64> {
        if self.times.len() == 1 {
            return self.times.first().copied();
        }
        let wanted = (seconds.max(0.0) * 1000.0).round() as u64;
        let index = self.times.partition_point(|&at| at < wanted);
        let after = self.times.get(index).copied();
        let before = index
            .checked_sub(1)
            .and_then(|i| self.times.get(i).copied());
        [before, after]
            .into_iter()
            .flatten()
            .min_by_key(|&at| at.abs_diff(wanted))
            .filter(|&at| at.abs_diff(wanted) <= REACH_MS)
    }

    /// The model's mask nearest `seconds`, decoded, or `None` when that
    /// instant is not analysed yet or its file cannot be read.
    pub fn mask_at(&self, seconds: f64) -> Option<Arc<Mask>> {
        let millis = self.nearest_millis(seconds)?;
        load(&mask_file(&self.dir, millis))
    }

    /// The mask to cut with at `seconds`: the model's, with the cutout's
    /// strokes painted on when it is custom, and softened by its feather.
    /// `aspect` is the source's width over its height, for round brushes.
    pub fn resolved(&self, seconds: f64, cutout: &Cutout, aspect: f32) -> Option<Arc<Mask>> {
        let millis = self.nearest_millis(seconds)?;
        let file = mask_file(&self.dir, millis);
        let key = ResolvedKey {
            file: file.clone(),
            settings: settings_key(cutout, aspect),
        };
        if let Some(hit) = resolved_cache()
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key))
        {
            return Some(hit);
        }
        let auto = load(&file)?;
        let painted = match cutout.mode {
            CutoutMode::Custom if !cutout.strokes.is_empty() => {
                strokes::paint(&auto, &cutout.strokes, aspect)
            }
            _ => (*auto).clone(),
        };
        let radius = cutout.feather as f32 * painted.width() as f32;
        let mask = Arc::new(painted.blurred(radius));
        if let Ok(mut cache) = resolved_cache().lock() {
            cache.put(key, Arc::clone(&mask));
        }
        Some(mask)
    }

    /// The source instants between `from` and `to` seconds, on the analysis
    /// grid, that have no mask yet. Empty when the range is covered.
    pub fn missing(&self, from: f64, to: f64) -> Vec<u64> {
        let step = 1000 / u64::from(MASK_RATE);
        let first = ((from.max(0.0) * 1000.0) as u64) / step * step;
        let last = (to.max(from).max(0.0) * 1000.0).ceil() as u64;
        let mut out = Vec::new();
        let mut at = first;
        while at <= last {
            let index = self.times.partition_point(|&t| t < at);
            let hit = self.times.get(index).copied().is_some_and(|t| t == at);
            if !hit {
                out.push(at);
            }
            at += step;
        }
        out
    }

    /// Writes a mask for `millis` and remembers it. Best effort on disk;
    /// in memory it is there either way, for the run that made it.
    pub fn put(&mut self, millis: u64, mask: &Mask) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|error| format!("could not create {}: {error}", self.dir.display()))?;
        let file = mask_file(&self.dir, millis);
        std::fs::write(&file, mask.to_png())
            .map_err(|error| format!("could not write {}: {error}", file.display()))?;
        if let Err(index) = self.times.binary_search(&millis) {
            self.times.insert(index, millis);
        }
        if let Ok(mut cache) = mask_cache().lock() {
            cache.put(file, Arc::new(mask.clone()));
        }
        Ok(())
    }

    /// Forgets every mask on disk and in memory, for a media file whose
    /// masks should be found again.
    pub fn clear(&mut self) {
        for &millis in &self.times {
            let file = mask_file(&self.dir, millis);
            let _ = std::fs::remove_file(&file);
            if let Ok(mut cache) = mask_cache().lock() {
                cache.remove(&file);
            }
        }
        if let Ok(mut cache) = resolved_cache().lock() {
            cache.retain(|key| !key.file.starts_with(&self.dir));
        }
        self.times.clear();
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// What besides the file decides a resolved mask: the mode, the feather
/// and every stroke, hashed, with the aspect the brushes were sized by.
fn settings_key(cutout: &Cutout, aspect: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (cutout.mode == CutoutMode::Custom).hash(&mut hasher);
    cutout.feather.to_bits().hash(&mut hasher);
    aspect.to_bits().hash(&mut hasher);
    if cutout.mode == CutoutMode::Custom {
        for stroke in &cutout.strokes {
            (stroke.tool as u8).hash(&mut hasher);
            stroke.size.to_bits().hash(&mut hasher);
            for [x, y] in &stroke.points {
                x.to_bits().hash(&mut hasher);
                y.to_bits().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ResolvedKey {
    file: PathBuf,
    settings: u64,
}

/// A bounded map that forgets its oldest entry first.
struct Recent<K, V> {
    map: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> Recent<K, V> {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &K) -> Option<V> {
        self.map.get(key).cloned()
    }

    fn put(&mut self, key: K, value: V) {
        if self.map.insert(key.clone(), value).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > CACHED {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }

    fn remove(&mut self, key: &K) {
        if self.map.remove(key).is_some() {
            self.order.retain(|held| held != key);
        }
    }

    fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.map.retain(|key, _| keep(key));
        self.order.retain(|key| keep(key));
    }
}

fn mask_cache() -> &'static Mutex<Recent<PathBuf, Arc<Mask>>> {
    static CACHE: OnceLock<Mutex<Recent<PathBuf, Arc<Mask>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Recent::new()))
}

fn resolved_cache() -> &'static Mutex<Recent<ResolvedKey, Arc<Mask>>> {
    static CACHE: OnceLock<Mutex<Recent<ResolvedKey, Arc<Mask>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Recent::new()))
}

/// The mask in `file`, from memory when it has been read before.
fn load(file: &Path) -> Option<Arc<Mask>> {
    if let Some(hit) = mask_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&file.to_path_buf()))
    {
        return Some(hit);
    }
    let bytes = std::fs::read(file).ok()?;
    let mask = Arc::new(Mask::from_png(&bytes)?);
    if let Ok(mut cache) = mask_cache().lock() {
        cache.put(file.to_path_buf(), Arc::clone(&mask));
    }
    Some(mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::model::{BrushTool, Stroke};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "concat-vision-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_directory_is_named_by_the_file_and_the_model() {
        let a = mask_dir(Path::new("/p"), "/footage/a.mp4");
        let b = mask_dir(Path::new("/p"), "/footage/b.mp4");
        assert_ne!(a, b);
        assert!(a.starts_with("/p/cache/masks"));
        assert!(a.to_string_lossy().ends_with(MODEL_ID));
    }

    #[test]
    fn masks_are_written_listed_and_found_by_nearest_instant() {
        let dir = scratch("store");
        let mut store = MaskStore::open(&dir);
        assert!(store.is_empty());
        store.put(0, &Mask::filled(4, 4, 10)).expect("writes");
        store.put(100, &Mask::filled(4, 4, 20)).expect("writes");
        store.put(200, &Mask::filled(4, 4, 30)).expect("writes");

        let reopened = MaskStore::open(&dir);
        assert_eq!(reopened.len(), 3);
        assert_eq!(reopened.nearest_millis(0.14), Some(100));
        assert_eq!(reopened.nearest_millis(0.16), Some(200));
        // Beyond reach of every analysed instant: not yet.
        assert_eq!(reopened.nearest_millis(9.0), None);
        assert_eq!(reopened.mask_at(0.21).map(|m| m.at(0, 0)), Some(30));

        assert_eq!(reopened.missing(0.0, 0.2), Vec::<u64>::new());
        assert_eq!(reopened.missing(0.0, 0.45), vec![300, 400]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_single_mask_answers_for_every_instant() {
        let dir = scratch("still");
        let mut store = MaskStore::open(&dir);
        store.put(0, &Mask::filled(4, 4, 200)).expect("writes");
        assert_eq!(store.nearest_millis(37.5), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_resolved_mask_carries_the_strokes_and_the_feather() {
        let dir = scratch("resolved");
        let mut store = MaskStore::open(&dir);
        store.put(0, &Mask::filled(16, 16, 0)).expect("writes");
        let plain = Cutout::auto();
        assert_eq!(
            store.resolved(0.0, &plain, 1.0).map(|m| m.at(8, 8)),
            Some(0)
        );
        let custom = Cutout {
            mode: CutoutMode::Custom,
            feather: 0.0,
            strokes: vec![Stroke {
                tool: BrushTool::Brush,
                size: 0.5,
                points: vec![[0.5, 0.5]],
            }],
        };
        assert_eq!(
            store.resolved(0.0, &custom, 1.0).map(|m| m.at(8, 8)),
            Some(255)
        );
        // Automatic ignores the strokes even when they are still stored.
        let back_to_auto = Cutout {
            mode: CutoutMode::Auto,
            ..custom.clone()
        };
        assert_eq!(
            store.resolved(0.0, &back_to_auto, 1.0).map(|m| m.at(8, 8)),
            Some(0)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clearing_forgets_disk_and_memory() {
        let dir = scratch("clear");
        let mut store = MaskStore::open(&dir);
        store.put(0, &Mask::filled(4, 4, 1)).expect("writes");
        assert!(store.mask_at(0.0).is_some());
        store.clear();
        assert!(store.is_empty());
        assert!(!dir.exists());
        assert!(MaskStore::open(&dir).mask_at(0.0).is_none());
    }
}
