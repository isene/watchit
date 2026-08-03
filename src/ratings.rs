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

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Entry {
    /// 1-10, or 0 for "no rating" (tombstone).
    pub score: u8,
    /// Unix seconds. Newest wins when devices disagree.
    pub ts: i64,
    /// The title as this device knew it. Carried because the two ends do
    /// NOT share an id space: this catalog still holds IMDB tconsts from
    /// the old import while the phone keys everything by TMDB id. Title +
    /// year is the only key both ends can agree on, so it rides along and
    /// backs the id up.
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: i32,
}

/// Key for the title fallback — see `data::normalize_title`.
fn title_key(title: &str) -> String {
    crate::data::normalize_title(title)
}

/// Do these two refer to the same title? A year of 0 means "unknown" —
/// half this catalog came from an import that carried no years — and a
/// series is filed under its first-air year on one side and its release
/// year on the other often enough to allow a year of slack.
fn same_title(a: i32, b: i32) -> bool {
    a == 0 || b == 0 || (a - b).abs() <= 1
}

pub type Map = HashMap<String, Entry>;

#[derive(Default)]
pub struct Ratings {
    map: Map,
    /// Normalised title → the ids rated under it, so a rating made under
    /// the other end's id scheme still finds its title here. A title can
    /// hold more than one id — a remake, or the same show imported twice
    /// — so the year picks between them. Rebuilt whenever `map` changes.
    by_title: HashMap<String, Vec<String>>,
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
        let mut r = Self { map, by_title: HashMap::new(), mine };
        r.reindex();
        r
    }

    fn reindex(&mut self) {
        self.by_title.clear();
        for (id, e) in &self.map {
            if e.title.is_empty() { continue; }
            self.by_title.entry(title_key(&e.title)).or_default().push(id.clone());
        }
    }

    /// The id rated under this title, if any. Prefers an exact year, then
    /// anything close enough to be the same thing.
    fn id_by_title(&self, title: &str, year: i32) -> Option<&String> {
        let ids = self.by_title.get(&title_key(title))?;
        ids.iter()
            .find(|id| self.map.get(*id).map(|e| e.year == year).unwrap_or(false))
            .or_else(|| ids.iter()
                .find(|id| self.map.get(*id).map(|e| same_title(e.year, year)).unwrap_or(false)))
    }

    /// My score for a title, or None when unrated. Falls back to the
    /// title so a rating made on the phone (TMDB ids) still lands on the
    /// same film here (tconsts).
    pub fn get(&self, id: &str, title: &str, year: i32) -> Option<u8> {
        if let Some(e) = self.map.get(id) {
            return Some(e.score).filter(|s| *s > 0);
        }
        let other = self.id_by_title(title, year)?;
        self.map.get(other).map(|e| e.score).filter(|s| *s > 0)
    }

    /// Set (1-10) or clear (0) my score, and write this device's file.
    /// Writing through the id the OTHER end used (when the title matches)
    /// keeps one film to one entry instead of two rival ones.
    pub fn set(&mut self, id: &str, title: &str, year: i32, score: u8) {
        let key = match self.map.contains_key(id) {
            true => id.to_string(),
            false => self.id_by_title(title, year).cloned()
                .unwrap_or_else(|| id.to_string()),
        };
        self.map.insert(key, Entry {
            score: score.min(10), ts: now(),
            title: title.to_string(), year,
        });
        self.reindex();
        self.save();
    }

    /// Move a rating from one id to another, keeping its score and its
    /// timestamp. Used when the catalog migrates a title from its IMDB
    /// id to its TMDB one: the rating has to follow, or it is orphaned
    /// under an id nothing refers to any more.
    pub fn rekey(&mut self, old: &str, new: &str, title: &str, year: i32) {
        if old == new { return; }
        let Some(mut e) = self.map.remove(old) else { return };
        if !title.is_empty() { e.title = title.to_string(); }
        if year > 0 { e.year = year; }
        // A rating already on the new id wins only if it is newer.
        let keep = match self.map.get(new) {
            Some(existing) if existing.ts >= e.ts => true,
            _ => false,
        };
        if !keep { self.map.insert(new.to_string(), e); }
        self.reindex();
        self.save();
    }

    /// Every title I have actually scored, highest first.
    pub fn rated(&self) -> Vec<(String, Entry)> {
        let mut v: Vec<(String, Entry)> = self.map.iter()
            .filter(|(_, e)| e.score > 0)
            .map(|(id, e)| (id.clone(), e.clone()))
            .collect();
        v.sort_by(|a, b| b.1.score.cmp(&a.1.score));
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

    fn e(score: u8, ts: i64, title: &str, year: i32) -> Entry {
        Entry { score, ts, title: title.into(), year }
    }

    #[test]
    fn newest_timestamp_wins() {
        let mut dst: Map = HashMap::new();
        dst.insert("tt1".into(), e(5, 100, "A", 1990));
        dst.insert("tt2".into(), e(7, 300, "B", 1991));
        let mut src: Map = HashMap::new();
        src.insert("tt1".into(), e(9, 200, "A", 1990)); // newer, wins
        src.insert("tt2".into(), e(3, 200, "B", 1991)); // older, loses
        merge_into(&mut dst, src);
        assert_eq!(dst["tt1"].score, 9);
        assert_eq!(dst["tt2"].score, 7);
    }

    #[test]
    fn a_clear_beats_an_older_rating() {
        // The phone clears what the desktop rated yesterday: the
        // tombstone has to win, or the rating comes back on merge.
        let mut dst: Map = HashMap::new();
        dst.insert("tt1".into(), e(8, 100, "A", 1990));
        let mut src: Map = HashMap::new();
        src.insert("tt1".into(), e(0, 200, "A", 1990));
        merge_into(&mut dst, src);
        assert_eq!(dst["tt1"].score, 0);
    }

    #[test]
    fn a_missing_year_still_matches_the_title() {
        // Half this catalog came from an import that carried no years, so
        // insisting on an exact year match means the imported copy of a
        // show never shows the rating given to its TMDB-keyed twin.
        let dir = std::env::temp_dir().join(format!("watchit-noyear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m: Map = HashMap::new();
        m.insert("71914".into(), e(8, 100, "The Wheel of Time", 2021));
        std::fs::write(dir.join("ratings-x.json"), serde_json::to_string(&m).unwrap()).unwrap();

        let r = Ratings::load(&dir);
        assert_eq!(r.get("tt7462410", "The Wheel of Time", 0), Some(8), "year 0 = unknown");
        assert_eq!(r.get("tt7462410", "The Wheel of Time", 2022), Some(8), "a year of slack");
        assert_eq!(r.get("tt0000001", "Something Else", 0), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_title_bridges_two_id_schemes() {
        // The phone rates by TMDB id, this catalog still holds tconsts.
        // Same film, so the score has to show up here anyway — and
        // re-rating it must land on the SAME entry, not fork a second.
        let dir = std::env::temp_dir().join(format!("watchit-ratings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut phone: Map = HashMap::new();
        phone.insert("278".into(), e(9, 100, "The Shawshank Redemption", 1994));
        std::fs::write(dir.join("ratings-phone.json"),
                       serde_json::to_string(&phone).unwrap()).unwrap();

        let mut r = Ratings::load(&dir);
        assert_eq!(r.get("tt0111161", "The Shawshank Redemption", 1994), Some(9));
        r.set("tt0111161", "The Shawshank Redemption", 1994, 7);
        assert_eq!(r.rated().len(), 1, "one film, one entry");
        assert_eq!(r.get("tt0111161", "The Shawshank Redemption", 1994), Some(7));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
