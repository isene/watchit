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
    pub fn save(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
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

        out.insert(id.clone(), Details {
            id: id.clone(),
            title, year, rating, votes, runtime, plot, genres,
            directors, writers: Vec::new(), stars,
            poster_url: String::new(), streaming,
            content_rating, country, kind, release_date, start_date, end_date,
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

pub fn save_details_cache(path: &std::path::Path, cache: &DetailsCache) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(path, s);
    }
}
