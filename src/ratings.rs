//! My own 1-10 score for a title, kept apart from `ListItem::rating`
//! (which is TMDB's crowd score).
//!
//! Storage is one file PER DEVICE in `~/.watchit/sync/`, because that
//! directory is shared with the phone: two devices writing the same
//! file is exactly what makes Syncthing leave silent
//! `.sync-conflict-…` copies that nobody ever reads. Each device
//! writes only its own file and reads them all; per title, the newest
//! timestamp wins. No conflicts possible and no merge protocol to get
//! wrong.
//!
//! Clearing a rating stores score 0 with a fresh timestamp — a
//! tombstone. Dropping the entry instead would let the other device's
//! older rating win the next merge and resurrect it.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct Entry {
    /// 1-10, or 0 for "no rating" (tombstone).
    pub score: u8,
    /// Unix seconds. Newest wins when devices disagree.
    pub ts: i64,
}

pub type Map = HashMap<String, Entry>;

#[derive(Default)]
pub struct Ratings {
    map: Map,
    mine: PathBuf,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// This machine's name, reduced to what is safe in a filename. Used to
/// keep each device's file distinct in the shared folder.
fn device() -> String {
    let raw = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    let name: String = raw.trim().chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    if name.is_empty() { "desktop".to_string() } else { name }
}

/// Merge `src` into `dst`, newest timestamp per title wins.
pub fn merge_into(dst: &mut Map, src: Map) {
    for (id, e) in src {
        match dst.get(&id) {
            Some(old) if old.ts >= e.ts => {}
            _ => { dst.insert(id, e); }
        }
    }
}

fn read_file(path: &Path) -> Map {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

impl Ratings {
    /// Read every device's file in `dir` and merge them.
    pub fn load(dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(dir);
        let mine = dir.join(format!("ratings-{}.json", device()));
        let mut map = Map::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let is_ratings = p.file_name().and_then(|f| f.to_str())
                    .map(|n| n.starts_with("ratings-") && n.ends_with(".json"))
                    .unwrap_or(false);
                if is_ratings { merge_into(&mut map, read_file(&p)); }
            }
        }
        Self { map, mine }
    }

    /// My score for a title, or None when unrated.
    pub fn get(&self, id: &str) -> Option<u8> {
        self.map.get(id).map(|e| e.score).filter(|s| *s > 0)
    }

    /// Set (1-10) or clear (0) my score, and write this device's file.
    pub fn set(&mut self, id: &str, score: u8) {
        self.map.insert(id.to_string(), Entry { score: score.min(10), ts: now() });
        self.save();
    }

    /// Every title I have actually scored, highest first.
    pub fn rated(&self) -> Vec<(String, u8)> {
        let mut v: Vec<(String, u8)> = self.map.iter()
            .filter(|(_, e)| e.score > 0)
            .map(|(id, e)| (id.clone(), e.score))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v
    }

    pub fn count(&self) -> usize {
        self.map.values().filter(|e| e.score > 0).count()
    }

    /// Write only this device's file — never anyone else's.
    fn save(&self) {
        if let Some(parent) = self.mine.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = serde_json::to_string_pretty(&self.map) {
            let _ = std::fs::write(&self.mine, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_timestamp_wins() {
        let mut dst: Map = HashMap::new();
        dst.insert("tt1".into(), Entry { score: 5, ts: 100 });
        dst.insert("tt2".into(), Entry { score: 7, ts: 300 });
        let mut src: Map = HashMap::new();
        src.insert("tt1".into(), Entry { score: 9, ts: 200 }); // newer, wins
        src.insert("tt2".into(), Entry { score: 3, ts: 200 }); // older, loses
        merge_into(&mut dst, src);
        assert_eq!(dst["tt1"].score, 9);
        assert_eq!(dst["tt2"].score, 7);
    }

    #[test]
    fn a_clear_beats_an_older_rating() {
        // The phone clears what the desktop rated yesterday: the
        // tombstone has to win, or the rating comes back on merge.
        let mut dst: Map = HashMap::new();
        dst.insert("tt1".into(), Entry { score: 8, ts: 100 });
        let mut src: Map = HashMap::new();
        src.insert("tt1".into(), Entry { score: 0, ts: 200 });
        merge_into(&mut dst, src);
        assert_eq!(dst["tt1"].score, 0);
    }
}
