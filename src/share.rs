//! What this device puts in the folder it shares with the phone.
//!
//! Same shape as `ratings.rs`: one file per device, everyone reads them
//! all and writes only their own, so Syncthing never has two writers on
//! one file to leave a `.sync-conflict-` copy of.
//!
//! Two things live here. The catalog — a union, never a mirror: each
//! device adds titles separately and neither ever means "delete what I
//! do not have". And the TMDB key, so the phone does not need its own
//! copy typed in by hand.

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
    let _ = std::fs::create_dir_all(dir);
    if let Ok(s) = serde_json::to_string_pretty(&out) {
        let _ = std::fs::write(mine(dir), s);
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
