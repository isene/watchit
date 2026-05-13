//! TMDB API integration. The previous version of this module scraped
//! IMDB's HTML/JSON-LD pages directly. IMDB has since moved every
//! public page behind a CloudFront WAF that returns a JavaScript
//! challenge to any plain HTTP request — the scrape returned 0
//! results, see #1. This module replaces the scrape with first-class
//! API calls against The Movie Database (TMDB).
//!
//! Requires a TMDB v3 API key in config (`tmdb_key`). Without one,
//! all data operations return empty / error. Keys are free —
//! <https://www.themoviedb.org/settings/api>.
//!
//! ID format note: pre-TMDB versions of watchit stored IMDB tconsts
//! (`tt1234567`) as the primary identifier. This version stores TMDB
//! numeric IDs (e.g. `550`) instead. Legacy entries with `tt*` ids
//! are silently ignored by the new list/fetch code paths — re-scrape
//! to repopulate after upgrading.
//!
//! Public API kept compatible with the previous shape so main.rs's
//! call sites only need a small `api_key` plumb-through.

use crate::data::{Details, ListItem};
use serde_json::Value as JsonValue;

const TMDB_BASE: &str = "https://api.themoviedb.org/3";
const POSTER_BASE: &str = "https://image.tmdb.org/t/p/w500";

fn http_get(url: &str) -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(20))
        .build();
    agent.get(url)
        .set("Accept", "application/json")
        .call()
        .ok()?
        .into_string()
        .ok()
}

/// Map the legacy IMDB chart paths used by main.rs to TMDB endpoints
/// plus a "kind" tag (movie / tv). Returns None for unknown charts.
fn chart_to_tmdb(chart: &str) -> Option<(&'static str, &'static str)> {
    Some(match chart {
        "chart/top"        | "top_rated_movies" => ("/movie/top_rated", "movie"),
        "chart/toptv"      | "top_rated_tv"     => ("/tv/top_rated",    "tv"),
        "chart/moviemeter" | "popular_movies"   => ("/movie/popular",   "movie"),
        "chart/tvmeter"    | "popular_tv"       => ("/tv/popular",      "tv"),
        _ => return None,
    })
}

/// Fetch a TMDB "chart" — top-rated or popular, movies or TV.
/// Caller passes the same string names main.rs has historically used
/// (`chart/top`, `chart/toptv`, `chart/moviemeter`, `chart/tvmeter`)
/// for source compatibility; they map to TMDB endpoints internally.
///
/// `limit` is the number of items to return; TMDB serves 20 per page,
/// so up to `ceil(limit/20)` HTTP requests are made.
pub fn scrape_chart_keyed(chart: &str, limit: usize, api_key: &str) -> Vec<ListItem> {
    if api_key.is_empty() { return Vec::new(); }
    let Some((endpoint, kind)) = chart_to_tmdb(chart) else { return Vec::new() };

    let mut out: Vec<ListItem> = Vec::new();
    let pages_needed = (limit.saturating_sub(1) / 20) + 1;
    for page in 1..=pages_needed {
        let url = format!("{}{}?api_key={}&page={}", TMDB_BASE, endpoint, api_key, page);
        let Some(body) = http_get(&url) else { break };
        let Ok(v) = serde_json::from_str::<JsonValue>(&body) else { break };
        let Some(results) = v.get("results").and_then(|a| a.as_array()) else { break };
        if results.is_empty() { break; }
        for r in results {
            let Some(id) = r.get("id").and_then(|x| x.as_i64()) else { continue };
            let title_field = if kind == "movie" { "title" } else { "name" };
            let title = r.get(title_field).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let rating = r.get("vote_average").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let date_field = if kind == "movie" { "release_date" } else { "first_air_date" };
            let year = r.get(date_field).and_then(|x| x.as_str())
                .and_then(|s| s.get(..4))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            out.push(ListItem {
                id: id.to_string(),
                title, rating, year,
                genres: Vec::new(),  // genres come on details fetch
            });
            if out.len() >= limit { return out; }
        }
    }
    out
}

/// Fetch full details for a TMDB id. `kind_hint` (`"movie"` or `"tv"`)
/// short-circuits the movie-then-tv probing; pass `None` if unknown.
/// When `region` is non-empty, streaming providers are populated via
/// `append_to_response=watch/providers`.
pub fn fetch_details_keyed(id: &str, kind_hint: Option<&str>, region: &str, api_key: &str) -> Details {
    if api_key.is_empty() {
        return Details { id: id.into(), error: true, ..Details::default() };
    }
    // Skip legacy IMDB tconsts — the user upgraded and their old data
    // file is stale. Returning error lets main.rs skip these entries.
    if id.starts_with("tt") {
        return Details { id: id.into(), error: true, ..Details::default() };
    }
    let kinds: &[&str] = match kind_hint {
        Some("movie") | Some("Movie") => &["movie"],
        Some("tv") | Some("TVSeries") => &["tv"],
        _ => &["movie", "tv"],
    };
    for k in kinds {
        let url = format!(
            "{}/{}/{}?api_key={}&append_to_response=credits,external_ids,watch/providers,release_dates",
            TMDB_BASE, k, id, api_key
        );
        let Some(body) = http_get(&url) else { continue };
        let Ok(v) = serde_json::from_str::<JsonValue>(&body) else { continue };
        if v.get("status_code").is_some() { continue; } // TMDB 404 envelope
        return parse_details(id, k, region, &v);
    }
    Details { id: id.into(), error: true, ..Details::default() }
}

fn parse_details(id: &str, kind: &str, region: &str, v: &JsonValue) -> Details {
    let is_movie = kind == "movie";
    let title = v.get(if is_movie { "title" } else { "name" })
        .and_then(|x| x.as_str()).unwrap_or("").to_string();
    let plot = v.get("overview").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let rating = v.get("vote_average").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let votes = v.get("vote_count").and_then(|x| x.as_i64()).unwrap_or(0);
    let popularity = v.get("popularity").and_then(|x| x.as_f64()).unwrap_or(0.0);

    let runtime = if is_movie {
        v.get("runtime").and_then(|x| x.as_i64())
            .filter(|&m| m > 0)
            .map(|m| if m >= 60 {
                format!("{}h {}m", m / 60, m % 60)
            } else {
                format!("{}m", m)
            })
            .unwrap_or_default()
    } else {
        v.get("episode_run_time").and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.as_i64())
            .filter(|&m| m > 0)
            .map(|m| format!("{}m", m))
            .unwrap_or_default()
    };

    let release_date = v.get(if is_movie { "release_date" } else { "first_air_date" })
        .and_then(|x| x.as_str()).unwrap_or("").to_string();
    let year = release_date.get(..4).and_then(|s| s.parse().ok()).unwrap_or(0);

    let (start_date, end_date) = if is_movie {
        (String::new(), String::new())
    } else {
        (release_date.clone(),
         v.get("last_air_date").and_then(|x| x.as_str()).unwrap_or("").to_string())
    };

    let genres = v.get("genres").and_then(|x| x.as_array())
        .map(|arr| arr.iter()
            .filter_map(|g| g.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect())
        .unwrap_or_default();

    let directors = if is_movie {
        v.pointer("/credits/crew").and_then(|x| x.as_array())
            .map(|arr| arr.iter()
                .filter(|c| c.get("job").and_then(|j| j.as_str()) == Some("Director"))
                .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect())
            .unwrap_or_default()
    } else {
        // For TV shows, "created_by" is the closest equivalent.
        v.get("created_by").and_then(|x| x.as_array())
            .map(|arr| arr.iter()
                .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect())
            .unwrap_or_default()
    };

    let writers = v.pointer("/credits/crew").and_then(|x| x.as_array())
        .map(|arr| arr.iter()
            .filter(|c| {
                c.get("department").and_then(|d| d.as_str()) == Some("Writing")
            })
            .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect::<Vec<_>>())
        .map(|mut v| { v.dedup(); v })
        .unwrap_or_default();

    let stars = v.pointer("/credits/cast").and_then(|x| x.as_array())
        .map(|arr| arr.iter().take(10)
            .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect())
        .unwrap_or_default();

    let poster_path = v.get("poster_path").and_then(|x| x.as_str()).unwrap_or("");
    let poster_url = if poster_path.is_empty() {
        String::new()
    } else {
        format!("{}{}", POSTER_BASE, poster_path)
    };

    let seasons = if !is_movie {
        v.get("number_of_seasons").and_then(|x| x.as_i64()).map(|n| n as i32)
    } else { None };
    let episodes = if !is_movie {
        v.get("number_of_episodes").and_then(|x| x.as_i64()).map(|n| n as i32)
    } else { None };

    let country = v.get("origin_country").and_then(|x| x.as_array())
        .map(|arr| arr.iter().filter_map(|c| c.as_str()).collect::<Vec<_>>().join(","))
        .unwrap_or_else(|| v.get("production_countries").and_then(|x| x.as_array())
            .map(|arr| arr.iter()
                .filter_map(|c| c.get("iso_3166_1").and_then(|s| s.as_str()))
                .collect::<Vec<_>>()
                .join(","))
            .unwrap_or_default());

    // Streaming providers from append_to_response=watch/providers.
    // TMDB groups by region; we pull only the configured one.
    let streaming = if region.is_empty() {
        Vec::new()
    } else {
        let path = format!("/watch/providers/results/{}", region);
        let region_data = v.pointer(&path).cloned().unwrap_or(JsonValue::Null);
        let mut out = Vec::new();
        for bucket in ["flatrate", "free", "ads"] {
            if let Some(arr) = region_data.get(bucket).and_then(|x| x.as_array()) {
                for p in arr {
                    if let Some(n) = p.get("provider_name").and_then(|x| x.as_str()) {
                        if !out.iter().any(|s: &String| s == n) {
                            out.push(n.to_string());
                        }
                    }
                }
            }
        }
        out
    };

    // Content rating from release_dates (movies) / content_ratings (tv).
    // Best-effort: prefer the configured region, fall back to US.
    let content_rating = extract_content_rating(v, is_movie, region);

    Details {
        id: id.into(),
        title, year, rating, votes,
        runtime, plot, genres,
        directors, writers, stars,
        poster_url,
        streaming,
        content_rating,
        country,
        kind: if is_movie { "Movie".into() } else { "TVSeries".into() },
        release_date,
        start_date, end_date,
        seasons, episodes,
        popularity,
        error: false,
    }
}

fn extract_content_rating(v: &JsonValue, is_movie: bool, region: &str) -> String {
    let regions: Vec<&str> = if region.is_empty() {
        vec!["US"]
    } else {
        vec![region, "US"]
    };
    if is_movie {
        // /release_dates/results is array of { iso_3166_1, release_dates: [{certification}] }
        let Some(results) = v.pointer("/release_dates/results").and_then(|x| x.as_array()) else { return String::new() };
        for want in &regions {
            for r in results {
                if r.get("iso_3166_1").and_then(|x| x.as_str()) != Some(*want) { continue; }
                if let Some(dates) = r.get("release_dates").and_then(|x| x.as_array()) {
                    for d in dates {
                        let cert = d.get("certification").and_then(|x| x.as_str()).unwrap_or("");
                        if !cert.is_empty() { return cert.to_string(); }
                    }
                }
            }
        }
    } else {
        let Some(results) = v.pointer("/content_ratings/results").and_then(|x| x.as_array()) else { return String::new() };
        for want in &regions {
            for r in results {
                if r.get("iso_3166_1").and_then(|x| x.as_str()) != Some(*want) { continue; }
                let cert = r.get("rating").and_then(|x| x.as_str()).unwrap_or("");
                if !cert.is_empty() { return cert.to_string(); }
            }
        }
    }
    String::new()
}

/// Multi-search across movies and TV. Returns up to `max` items.
pub fn search_keyed(query: &str, max: usize, api_key: &str) -> Vec<ListItem> {
    if query.is_empty() || api_key.is_empty() { return Vec::new(); }
    let q = urlencode(query);
    let url = format!(
        "{}/search/multi?api_key={}&query={}&include_adult=false",
        TMDB_BASE, api_key, q
    );
    let Some(body) = http_get(&url) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<JsonValue>(&body) else { return Vec::new() };
    let Some(results) = v.get("results").and_then(|a| a.as_array()) else { return Vec::new() };
    results.iter()
        .filter_map(|r| {
            let mt = r.get("media_type").and_then(|x| x.as_str())?;
            if mt != "movie" && mt != "tv" { return None; }
            let id = r.get("id").and_then(|x| x.as_i64())?;
            let title_field = if mt == "movie" { "title" } else { "name" };
            let title = r.get(title_field).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let date_field = if mt == "movie" { "release_date" } else { "first_air_date" };
            let year = r.get(date_field).and_then(|x| x.as_str())
                .and_then(|s| s.get(..4))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let rating = r.get("vote_average").and_then(|x| x.as_f64()).unwrap_or(0.0);
            Some(ListItem {
                id: id.to_string(),
                title, rating, year,
                genres: Vec::new(),
            })
        })
        .take(max)
        .collect()
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
            out.push(c);
        } else if c == ' ' {
            out.push('+');
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}
