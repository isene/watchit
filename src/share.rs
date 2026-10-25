//! What this device puts in the folder it shares with the phone.
//!
//! Same shape as `ratings.rs`: one file per device, everyone reads them
//! all and writes only their own, so Syncthing never has two writers on
//! one file to leave a `.sync-conflict-` copy of.
//!
//! Three things live here. The catalog — a union, never a mirror: each
//! device adds titles separately and neither ever means "delete what I
//! do not have". The TMDB key, so the phone does not need its own copy
//! typed in by hand. And the wish lists, where a removal travels too.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::data::{Database, DetailsCache, ListItem};

/// This machine's name, reduced to what is safe in a filename.
fn device() -> String {
    let raw = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    let name: String = raw.trim().chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    if name.is_empty() { "desktop".to_string() } else { name }
}

fn mine(dir: &Path) -> PathBuf { dir.join(format!("catalog-{}.json", device())) }

const KEY_FILE: &str = "tmdb_key.txt";

/// Publish the TMDB key so the other end can pick it up instead of
/// having it typed in on a phone keyboard. Written only when it differs,
/// so this is a `read` and nothing else on almost every start.
pub fn publish_key(dir: &Path, key: &str) {
    if key.is_empty() { return; }
    let path = dir.join(KEY_FILE);
    if std::fs::read_to_string(&path).map(|s| s.trim() == key).unwrap_or(false) {
        return;
    }
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&path, format!("{}\n", key));
}

/// Read the key another device published, if this one has none.
pub fn published_key(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(KEY_FILE))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Write this device's catalog. Posters ride along from the details
/// cache: the phone shows thumbnails in its list, and without them a
/// title shared from here looks broken until it fetches details.
pub fn write_mine(dir: &Path, db: &Database, details: &DetailsCache) {
    let with_posters = |list: &Vec<ListItem>| -> Vec<ListItem> {
        list.iter().map(|it| {
            let mut it = it.clone();
            if it.poster_url.is_empty() {
                if let Some(d) = details.get(&it.id) {
                    it.poster_url = d.poster_url.clone();
                }
            }
            it
        }).collect()
    };
    let out = Database {
        movies: with_posters(&db.movies),
        series: with_posters(&db.series),
    };
    if let Ok(s) = serde_json::to_string_pretty(&out) {
        crate::data::write_whole(&mine(dir), &s);
    }
}

/// Merge every other device's catalog into this one. Returns how many
/// titles were new, so a silent sync can say what it did.
pub fn merge_others(dir: &Path, db: &mut Database) -> usize {
    let own = mine(dir);
    let mut added = 0;
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    for entry in entries.flatten() {
        let p = entry.path();
        if p == own { continue; }
        let is_catalog = p.file_name().and_then(|f| f.to_str())
            .map(|n| n.starts_with("catalog-") && n.ends_with(".json"))
            .unwrap_or(false);
        if !is_catalog { continue; }
        let Some(other) = std::fs::read_to_string(&p).ok()
            .and_then(|s| serde_json::from_str::<Database>(&s).ok()) else { continue };
        added += merge_list(&mut db.movies, other.movies);
        added += merge_list(&mut db.series, other.series);
    }
    added
}

/// Union by id. Where both sides know a title the one already held
/// wins, with empty fields backfilled from the other — a row that
/// arrived without a year or a poster should take them from whichever
/// device did the fetch.
fn merge_list(mine: &mut Vec<ListItem>, theirs: Vec<ListItem>) -> usize {
    let mut incoming: HashMap<String, ListItem> =
        theirs.into_iter().map(|i| (i.id.clone(), i)).collect();
    for it in mine.iter_mut() {
        let Some(other) = incoming.remove(&it.id) else { continue };
        if it.poster_url.is_empty() { it.poster_url = other.poster_url; }
        if it.kind.is_empty() { it.kind = other.kind; }
        if it.year == 0 { it.year = other.year; }
        if it.rating == 0.0 { it.rating = other.rating; }
        if it.genres.is_empty() { it.genres = other.genres; }
    }
    let added = incoming.len();
    mine.extend(incoming.into_values());
    added
}

// --- The wish lists ---
//
// Same rule again: `wish-<device>.json` per device, newest timestamp per
// title wins. Unlike the catalog this is no union: taking a title off
// the list is a mark of its own (`on: false`), or the other device's
// older "on" would put it back.

/// A title put on, or taken off, a wish list, and when.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub on: bool,
    pub ts: i64,
}

/// Per list ("movies", "series"), per catalog id.
pub type Marks = HashMap<String, HashMap<String, Mark>>;

fn wish_file(dir: &Path) -> PathBuf { dir.join(format!("wish-{}.json", device())) }

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Every device's marks, merged.
pub fn load_wish(dir: &Path) -> Marks {
    let mut out = Marks::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let p = entry.path();
        let is_wish = p.file_name().and_then(|f| f.to_str())
            .map(|n| n.starts_with("wish-") && n.ends_with(".json"))
            .unwrap_or(false);
        if !is_wish { continue; }
        let Some(theirs) = std::fs::read_to_string(&p).ok()
            .and_then(|s| serde_json::from_str::<Marks>(&s).ok()) else { continue };
        for (kind, marks) in theirs {
            let list = out.entry(kind).or_default();
            for (id, m) in marks {
                if list.get(&id).map(|old| m.ts > old.ts).unwrap_or(true) {
                    list.insert(id, m);
                }
            }
        }
    }
    out
}

/// Bring this device's wish list for `kind` in line with the marks.
/// A title on the list that no device has marked yet (this list as it
/// was before wish lists were shared) gets an "on" mark. Then the list
/// keeps its order, loses what was taken off elsewhere, and gains what
/// was put on elsewhere, oldest first. True when the marks changed and
/// this device's file needs writing.
pub fn sync_wish(marks: &mut Marks, kind: &str, list: &mut Vec<String>, now: i64) -> bool {
    let m = marks.entry(kind.to_string()).or_default();
    let mut changed = false;
    for id in list.iter() {
        if !m.contains_key(id) {
            m.insert(id.clone(), Mark { on: true, ts: now });
            changed = true;
        }
    }
    list.retain(|id| m.get(id).map(|k| k.on).unwrap_or(false));
    let mut gained: Vec<(i64, String)> = m.iter()
        .filter(|(id, k)| k.on && !list.contains(id))
        .map(|(id, k)| (k.ts, id.clone()))
        .collect();
    gained.sort();
    list.extend(gained.into_iter().map(|(_, id)| id));
    changed
}

/// Put a title on the list or take it off, and publish that.
pub fn mark_wish(dir: &Path, marks: &mut Marks, kind: &str, id: &str, on: bool) {
    marks.entry(kind.to_string()).or_default().insert(id.to_string(), Mark { on, ts: now() });
    write_wish(dir, marks);
}

/// A title that moved from its IMDB id to its TMDB one takes its marks
/// along. True when there was one to move.
pub fn rekey_wish(marks: &mut Marks, old: &str, new: &str) -> bool {
    let mut moved = false;
    for list in marks.values_mut() {
        if let Some(m) = list.remove(old) {
            list.entry(new.to_string()).or_insert(m);
            moved = true;
        }
    }
    moved
}

pub fn write_wish(dir: &Path, marks: &Marks) {
    if let Ok(s) = serde_json::to_string_pretty(marks) {
        crate::data::write_whole(&wish_file(dir), &s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str, year: i32) -> ListItem {
        ListItem { id: id.into(), title: title.into(), year, ..Default::default() }
    }

    #[test]
    fn a_shared_catalog_adds_without_removing() {
        let dir = std::env::temp_dir().join(format!("watchit-share-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let theirs = Database {
            movies: vec![item("1", "Shared", 1990), item("2", "Also theirs", 1991)],
            series: vec![],
        };
        std::fs::write(dir.join("catalog-phone.json"),
                       serde_json::to_string(&theirs).unwrap()).unwrap();

        let mut db = Database { movies: vec![item("1", "Shared", 0)], series: vec![] };
        let added = merge_others(&dir, &mut db);
        assert_eq!(added, 1, "one title this device did not have");
        assert_eq!(db.movies.len(), 2);
        let shared = db.movies.iter().find(|i| i.id == "1").unwrap();
        assert_eq!(shared.year, 1990, "the missing year came from the other side");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn on(ts: i64) -> Mark { Mark { on: true, ts } }

    #[test]
    fn a_wish_travels_and_so_does_its_removal() {
        let dir = std::env::temp_dir().join(format!("watchit-wish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The phone wished 2 and later took 1 off; this device knew 1 and 3.
        std::fs::write(dir.join("wish-phone.json"), r#"{"series":{
            "1":{"on":false,"ts":300}, "2":{"on":true,"ts":200}}}"#).unwrap();
        std::fs::write(dir.join("wish-here.json"), r#"{"series":{
            "1":{"on":true,"ts":100}, "3":{"on":true,"ts":100}}}"#).unwrap();
        let mut marks = load_wish(&dir);
        let mut list = vec!["1".to_string(), "3".to_string()];
        assert!(!sync_wish(&mut marks, "series", &mut list, 999), "nothing new to publish");
        assert_eq!(list, vec!["3", "2"], "1 taken off, 2 added at the end");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_sync_publishes_the_old_list() {
        let mut marks = Marks::new();
        let mut list = vec!["7".to_string()];
        assert!(sync_wish(&mut marks, "movies", &mut list, 50));
        assert_eq!(marks["movies"]["7"], on(50));
        assert_eq!(list, vec!["7"]);
    }

    #[test]
    fn the_key_round_trips() {
        let dir = std::env::temp_dir().join(format!("watchit-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(published_key(&dir), None);
        publish_key(&dir, "abc123");
        assert_eq!(published_key(&dir).as_deref(), Some("abc123"));
        publish_key(&dir, "");
        assert_eq!(published_key(&dir).as_deref(), Some("abc123"), "an empty key erases nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
