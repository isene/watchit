use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ListItem {
    pub id: String,        // tconst, e.g. "tt0111161"
    pub title: String,
    pub rating: f64,
    #[serde(default)]
    pub year: i32,
    #[serde(default)]
    pub genres: Vec<String>,
    /// "movie" or "tv" — which TMDB endpoint owns this id. Defaulted, so
    /// an older list.json still loads; search fills it in so a picked
    /// title lands in the right list instead of whichever view happened
    /// to be open.
    #[serde(default)]
    pub kind: String,
    /// Poster URL, carried only so a catalog shared with the phone can
    /// show thumbnails there. This app draws its posters from the
    /// details cache and leaves this empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poster_url: String,
}

/// Per-title extended details. Superset of the Ruby IMDB details format and
/// the JSON-LD data we scrape, so imports from ~/.imdb/data/details.json
/// preserve all fields (votes, content_rating, type, seasons, etc.).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Details {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub year: i32,
    #[serde(default)]
    pub rating: f64,
    #[serde(default)]
    pub votes: i64,
    #[serde(default)]
    pub runtime: String,
    #[serde(default)]
    pub plot: String,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub directors: Vec<String>,
    #[serde(default)]
    pub writers: Vec<String>,
    #[serde(default)]
    pub stars: Vec<String>,
    #[serde(default)]
    pub poster_url: String,
    #[serde(default)]
    pub streaming: Vec<String>,
    #[serde(default)]
    pub content_rating: String,
    #[serde(default)]
    pub country: String,
    /// "Movie" or "TVSeries"
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub release_date: String,
    /// IMDB tconst (`tt1234567`) if TMDB has it on file. Used to render
    /// a click-through link from the detail pane — TMDB is the data
    /// source, IMDB is one click away for reviews / trivia / etc.
    #[serde(default)]
    pub imdb_id: String,
    #[serde(default)]
    pub start_date: String,
    #[serde(default)]
    pub end_date: String,
    #[serde(default)]
    pub seasons: Option<i32>,
    #[serde(default)]
    pub episodes: Option<i32>,
    #[serde(default)]
    pub popularity: f64,
    #[serde(default)]
    pub error: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Database {
    pub movies: Vec<ListItem>,
    pub series: Vec<ListItem>,
}

impl Database {
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
}

/// The lists and the details, kept in ferrite in `~/.watchit/data/watchit.ferrite`.
/// A save writes only the titles that changed. On the first start the rows
/// are copied from list.json and details.json, which are left as they were.
pub struct Store {
    fe: ferrite::Db,
}

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS movies (id TEXT PRIMARY KEY, pos INTEGER NOT NULL,
        title TEXT NOT NULL, rating REAL NOT NULL, year INTEGER NOT NULL,
        genres TEXT NOT NULL, kind TEXT NOT NULL, poster_url TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS series (id TEXT PRIMARY KEY, pos INTEGER NOT NULL,
        title TEXT NOT NULL, rating REAL NOT NULL, year INTEGER NOT NULL,
        genres TEXT NOT NULL, kind TEXT NOT NULL, poster_url TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS details (key TEXT PRIMARY KEY, id TEXT NOT NULL,
        title TEXT NOT NULL, year INTEGER NOT NULL, rating REAL NOT NULL,
        votes INTEGER NOT NULL, runtime TEXT NOT NULL, plot TEXT NOT NULL,
        genres TEXT NOT NULL, directors TEXT NOT NULL, writers TEXT NOT NULL,
        stars TEXT NOT NULL, poster_url TEXT NOT NULL, streaming TEXT NOT NULL,
        content_rating TEXT NOT NULL, country TEXT NOT NULL, kind TEXT NOT NULL,
        release_date TEXT NOT NULL, imdb_id TEXT NOT NULL, start_date TEXT NOT NULL,
        end_date TEXT NOT NULL, seasons INTEGER, episodes INTEGER,
        popularity REAL NOT NULL, error INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)";

const ITEM_COLS: &str = "id, pos, title, rating, year, genres, kind, poster_url";
const DETAIL_COLS: &str = "key, id, title, year, rating, votes, runtime, plot, genres, \
    directors, writers, stars, poster_url, streaming, content_rating, country, kind, \
    release_date, imdb_id, start_date, end_date, seasons, episodes, popularity, error";

/// A sweep writes whole detail rows, and the log only folds into a
/// snapshot by itself after many of them. Past this size it is folded at
/// the next start, so a start never reads many old copies.
const BIG_LOG: u64 = 2 * 1024 * 1024;

use ferrite::{Row, Value};

fn text(s: &str) -> Value { Value::Text(s.to_string()) }
/// A list of names or genres is one text, a line each.
fn lines(v: &[String]) -> Value { Value::Text(v.join("\n")) }
fn opt_int(n: Option<i32>) -> Value { n.map_or(Value::Null, |n| Value::Int(n as i64)) }

fn text_at(r: &Row, i: usize) -> String { r.get(i).and_then(Value::as_text).unwrap_or("").to_string() }
fn int_at(r: &Row, i: usize) -> Option<i64> { r.get(i).and_then(Value::as_int) }
fn real_at(r: &Row, i: usize) -> f64 { r.get(i).and_then(Value::as_real).unwrap_or(0.0) }
fn lines_at(r: &Row, i: usize) -> Vec<String> {
    let s = r.get(i).and_then(Value::as_text).unwrap_or("");
    if s.is_empty() { Vec::new() } else { s.split('\n').map(String::from).collect() }
}

fn item_row(pos: usize, it: &ListItem) -> Row {
    vec![text(&it.id), Value::Int(pos as i64), text(&it.title), Value::Real(it.rating),
        Value::Int(it.year as i64), lines(&it.genres), text(&it.kind), text(&it.poster_url)]
}

fn item_of(r: &Row) -> ListItem {
    ListItem {
        id: text_at(r, 0), title: text_at(r, 2), rating: real_at(r, 3),
        year: int_at(r, 4).unwrap_or(0) as i32, genres: lines_at(r, 5),
        kind: text_at(r, 6), poster_url: text_at(r, 7),
    }
}

fn detail_row(key: &str, d: &Details) -> Row {
    vec![text(key), text(&d.id), text(&d.title), Value::Int(d.year as i64), Value::Real(d.rating),
        Value::Int(d.votes), text(&d.runtime), text(&d.plot), lines(&d.genres),
        lines(&d.directors), lines(&d.writers), lines(&d.stars), text(&d.poster_url),
        lines(&d.streaming), text(&d.content_rating), text(&d.country), text(&d.kind),
        text(&d.release_date), text(&d.imdb_id), text(&d.start_date), text(&d.end_date),
        opt_int(d.seasons), opt_int(d.episodes), Value::Real(d.popularity),
        Value::Int(d.error as i64)]
}

fn detail_of(r: &Row) -> Details {
    Details {
        id: text_at(r, 1), title: text_at(r, 2), year: int_at(r, 3).unwrap_or(0) as i32,
        rating: real_at(r, 4), votes: int_at(r, 5).unwrap_or(0), runtime: text_at(r, 6),
        plot: text_at(r, 7), genres: lines_at(r, 8), directors: lines_at(r, 9),
        writers: lines_at(r, 10), stars: lines_at(r, 11), poster_url: text_at(r, 12),
        streaming: lines_at(r, 13), content_rating: text_at(r, 14), country: text_at(r, 15),
        kind: text_at(r, 16), release_date: text_at(r, 17), imdb_id: text_at(r, 18),
        start_date: text_at(r, 19), end_date: text_at(r, 20),
        seasons: int_at(r, 21).map(|n| n as i32), episodes: int_at(r, 22).map(|n| n as i32),
        popularity: real_at(r, 23), error: int_at(r, 24).unwrap_or(0) != 0,
    }
}

impl Store {
    pub fn open(dir: &std::path::Path) -> Result<Store, String> {
        // ferrite lets one program in at a time.
        let fe = ferrite::Db::open(dir)
            .map_err(|e| format!("{e} (is watchit already running?)"))?;
        let mut s = Store { fe };
        s.fe.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        let copied = s.fe.query("SELECT value FROM meta WHERE key = 'copied'", &[])
            .map(|o| !o.rows().is_empty()).unwrap_or(false);
        if !copied { s.copy_json(dir)?; }
        if s.fe.log_len() > BIG_LOG { let _ = s.fe.checkpoint(); }
        Ok(s)
    }

    /// Copy list.json and details.json from beside the store, all of it or
    /// nothing, with the row that says it is done.
    fn copy_json(&mut self, dir: &std::path::Path) -> Result<(), String> {
        let db = Database::load(&dir.with_file_name("list.json"));
        let details = load_details_cache(&dir.with_file_name("details.json"));
        self.fe.begin();
        let done = self.put_list(&db)
            .and_then(|_| self.put_details(&details))
            .and_then(|_| self.fe.execute("INSERT INTO meta (key, value) VALUES ('copied', '1')", &[])
                .map(|_| ()).map_err(|e| e.to_string()));
        match done {
            Ok(_) => self.fe.commit().map_err(|e| e.to_string()),
            Err(e) => { self.fe.rollback(); Err(format!("copy list.json: {e}")) }
        }
    }

    /// A table's rows where they sit in ferrite, the columns in the order
    /// the schema gives them. Reading through SQL would copy each row once
    /// more, and a start does this for every title.
    fn rows(&self, table: &str) -> Vec<&Row> {
        match self.fe.table_id(table) {
            Ok(id) => self.fe.table(id).iter().map(|(_, r)| r).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn list(&self) -> Database {
        let items = |table: &str| -> Vec<ListItem> {
            let mut v: Vec<(i64, ListItem)> = self.rows(table).into_iter()
                .map(|r| (int_at(r, 1).unwrap_or(0), item_of(r))).collect();
            v.sort_unstable_by_key(|(pos, _)| *pos);
            v.into_iter().map(|(_, it)| it).collect()
        };
        Database { movies: items("movies"), series: items("series") }
    }

    pub fn details(&self) -> DetailsCache {
        self.rows("details").into_iter().map(|r| (text_at(r, 0), detail_of(r))).collect()
    }

    pub fn save_list(&mut self, db: &Database) {
        self.fe.begin();
        match self.put_list(db) {
            Ok(_) => { let _ = self.fe.commit(); }
            Err(_) => self.fe.rollback(),
        }
    }

    pub fn save_details(&mut self, cache: &DetailsCache) {
        self.fe.begin();
        match self.put_details(cache) {
            Ok(_) => { let _ = self.fe.commit(); }
            Err(_) => self.fe.rollback(),
        }
    }

    fn put_list(&mut self, db: &Database) -> Result<(), String> {
        for (table, items) in [("movies", &db.movies), ("series", &db.series)] {
            let rows = items.iter().enumerate().map(|(i, it)| item_row(i, it)).collect();
            self.put_rows(table, ITEM_COLS, rows)?;
        }
        Ok(())
    }

    fn put_details(&mut self, cache: &DetailsCache) -> Result<(), String> {
        let rows = cache.iter().map(|(k, d)| detail_row(k, d)).collect();
        self.put_rows("details", DETAIL_COLS, rows)
    }

    /// Make the table hold exactly these rows, the key first in each.
    /// A row that is already there as it is costs nothing.
    fn put_rows(&mut self, table: &str, cols: &str, rows: Vec<Row>) -> Result<(), String> {
        let key_of = |r: &Row| r.first().and_then(Value::as_text).unwrap_or("").to_string();
        let (changed, gone): (Vec<&Row>, Vec<String>) = {
            let now: std::collections::HashMap<String, &Row> =
                self.rows(table).into_iter().map(|r| (key_of(r), r)).collect();
            let keys: std::collections::HashSet<String> = rows.iter().map(key_of).collect();
            (rows.iter().filter(|r| now.get(&key_of(r)) != Some(r)).collect(),
             now.keys().filter(|k| !keys.contains(*k)).cloned().collect())
        };
        if !changed.is_empty() {
            let slots: Vec<String> = (1..=changed[0].len()).map(|i| format!("?{i}")).collect();
            let put = self.fe.prepare(&format!("INSERT OR REPLACE INTO {table} ({cols}) VALUES ({})", slots.join(", ")))
                .map_err(|e| e.to_string())?;
            for row in changed {
                put.run(&mut self.fe, row).map_err(|e| e.to_string())?;
            }
        }
        if !gone.is_empty() {
            let key = cols.split(',').next().unwrap_or("id");
            let del = self.fe.prepare(&format!("DELETE FROM {table} WHERE {key} = ?1"))
                .map_err(|e| e.to_string())?;
            for k in gone {
                del.run(&mut self.fe, &[Value::Text(k)]).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

/// A title reduced to what identifies it: case and punctuation are not
/// signal, and neither is the year range the old IMDB import baked into
/// the title itself ("Travelers (2016-2018)"). Without stripping that,
/// an imported row never matches its TMDB twin — not for deduplication,
/// and not for finding the rating the phone made under the other id.
pub fn normalize_title(title: &str) -> String {
    let mut t = title.trim();
    if t.ends_with(')') {
        if let Some(open) = t.rfind('(') {
            let inner = &t[open + 1..t.len() - 1];
            let only_years = !inner.is_empty() && inner.chars().all(|c| {
                c.is_ascii_digit() || c == '-' || c == '\u{2013}' || c == '\u{2014}'
                    || c == ' ' || c == '/' || c == '?'
            });
            if only_years { t = t[..open].trim_end(); }
        }
    }
    t.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Key for anything cached per title: the details entry, the poster
/// file, the rating.
///
/// A TMDB id is NOT unique on its own — movie ids and tv ids are
/// separate spaces, so 745 is both "The Sixth Sense" and "Spaced". A
/// catalog holding both then serves one's details, poster and rating to
/// the other. Series get a `t` prefix; ids are numeric, so nothing else
/// can look like one. Legacy IMDB ids are left alone — IMDB has a single
/// space, so `tt0167404` already means one thing.
pub fn cache_key(id: &str, is_series: bool) -> String {
    if is_series && !id.starts_with("tt") {
        format!("t{}", id)
    } else {
        id.to_string()
    }
}

pub type DetailsCache = HashMap<String, Details>;

pub fn load_details_cache(path: &std::path::Path) -> DetailsCache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| {
            // Support both our own format and the legacy Ruby IMDB format.
            if let Ok(c) = serde_json::from_str::<DetailsCache>(&s) {
                Some(c)
            } else {
                parse_legacy_details(&s)
            }
        })
        .unwrap_or_default()
}

/// Ruby IMDB details.json had slightly different field names:
///   summary → plot, actors → stars, providers → streaming,
///   duration ("2H22M") → runtime, release_date → year parse,
///   type → kind, error: "none" → false.
fn parse_legacy_details(s: &str) -> Option<DetailsCache> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let obj = v.as_object()?;
    let mut out: DetailsCache = HashMap::new();
    for (id, entry) in obj {
        let e = entry.as_object()?;
        let title = e.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let rating = e.get("rating").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let votes = e.get("votes").and_then(|x| x.as_i64()).unwrap_or(0);
        let release_date = e.get("release_date").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let year = release_date.get(..4).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
        let plot = e.get("summary").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let runtime = e.get("duration").and_then(|x| x.as_str())
            .map(normalize_duration)
            .unwrap_or_default();
        let genres = e.get("genres").map(string_array).unwrap_or_default();
        let directors = e.get("directors").map(string_array).unwrap_or_default();
        let stars = e.get("actors").map(string_array).unwrap_or_default();
        let streaming = e.get("providers").map(string_array).unwrap_or_default();
        let content_rating = e.get("content_rating").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let country = e.get("country").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let kind = e.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let start_date = e.get("start_date").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let end_date = e.get("end_date").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let popularity = e.get("popularity").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let seasons = e.get("seasons").and_then(|x| x.as_i64()).map(|n| n as i32);
        let episodes = e.get("episodes").and_then(|x| x.as_i64()).map(|n| n as i32);
        let err_str = e.get("error").and_then(|x| x.as_str()).unwrap_or("none");
        let error = err_str != "none" && !err_str.is_empty();

        // Legacy ids ARE tconsts, so use the id itself as imdb_id.
        let imdb_id = if id.starts_with("tt") { id.clone() } else { String::new() };
        out.insert(id.clone(), Details {
            id: id.clone(),
            title, year, rating, votes, runtime, plot, genres,
            directors, writers: Vec::new(), stars,
            poster_url: String::new(), streaming,
            content_rating, country, kind, release_date, imdb_id, start_date, end_date,
            seasons, episodes, popularity, error,
        });
    }
    Some(out)
}

fn string_array(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn normalize_duration(s: &str) -> String {
    // "2H22M" → "2h 22m"; "1H05M" → "1h 05m"; "45M" → "45m".
    let mut out = String::new();
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() { num.push(c); }
        else {
            if !num.is_empty() {
                match c {
                    'H' | 'h' => { out.push_str(&num); out.push_str("h "); }
                    'M' | 'm' => { out.push_str(&num); out.push_str("m "); }
                    'S' | 's' => { out.push_str(&num); out.push('s'); }
                    _ => {}
                }
                num.clear();
            }
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same<T: Serialize>(a: &T, b: &T) -> bool {
        serde_json::to_value(a).unwrap() == serde_json::to_value(b).unwrap()
    }

    #[test]
    fn the_store_gives_back_what_was_saved_and_writes_only_changes() {
        let dir = std::env::temp_dir().join(format!("watchit-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let item = |id: &str, title: &str| ListItem {
            id: id.into(), title: title.into(), rating: 7.5, year: 1999,
            genres: vec!["Drama".into(), "Sci-Fi".into()], kind: "movie".into(), poster_url: String::new(),
        };
        let db = Database { movies: vec![item("745", "The Sixth Sense"), item("603", "The Matrix")],
            series: vec![item("745", "Spaced")] };
        std::fs::write(dir.join("list.json"), serde_json::to_string(&db).unwrap()).unwrap();
        let mut details = DetailsCache::new();
        details.insert("t745".into(), Details { id: "745".into(), title: "Spaced".into(),
            seasons: Some(2), stars: vec!["Alice".into(), "Bob".into()], ..Default::default() });
        details.insert("603".into(), Details { id: "603".into(), title: "The Matrix".into(),
            rating: 8.2, error: true, ..Default::default() });
        std::fs::write(dir.join("details.json"), serde_json::to_string(&details).unwrap()).unwrap();

        // The first open copies the JSON files in.
        let store_dir = dir.join("watchit.ferrite");
        let mut s = Store::open(&store_dir).unwrap();
        assert!(same(&s.list(), &db));
        assert!(same(&s.details(), &details));

        // Saving what is already there writes nothing.
        let before = s.fe.log_len();
        s.save_list(&db);
        s.save_details(&details);
        assert_eq!(s.fe.log_len(), before);

        // A change, a removal and a new title, and they survive a reopen.
        let mut db2 = db.clone();
        db2.movies[0].rating = 8.1;
        db2.series.clear();
        db2.series.push(item("1396", "Breaking Bad"));
        details.remove("t745");
        s.save_list(&db2);
        s.save_details(&details);
        drop(s);
        let s = Store::open(&store_dir).unwrap();
        assert!(same(&s.list(), &db2));
        assert!(same(&s.details(), &details));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_series_and_a_film_can_share_a_tmdb_id() {
        // movie/745 is The Sixth Sense, tv/745 is Spaced.
        assert_ne!(cache_key("745", true), cache_key("745", false));
        assert_eq!(cache_key("745", false), "745");
        assert_eq!(cache_key("745", true), "t745");
        // IMDB ids are already unambiguous; leave them be.
        assert_eq!(cache_key("tt0167404", true), "tt0167404");
    }

    #[test]
    fn the_imports_year_range_is_not_part_of_the_title() {
        assert_eq!(normalize_title("Travelers (2016\u{2013}2018)"), normalize_title("Travelers"));
        assert_eq!(normalize_title("Altered Carbon (2018\u{2013}2020)"), "alteredcarbon");
        // A parenthetical that is not a year range stays: it distinguishes.
        assert_ne!(normalize_title("Alien (Director's Cut)"), normalize_title("Alien"));
        assert_eq!(normalize_title("M*A*S*H"), "mash");
    }
}
