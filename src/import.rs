//! One-time import of existing IMDB-terminal (Ruby) data/config.
//!
//! Runs on startup if ~/.watchit/ is empty but ~/.imdb/ and ~/.imdb.yml
//! exist. Copies list.json, details.json, all poster JPGs, and translates
//! the old config into watchit's format.

use crate::config::{self, Config};
use serde_yaml::Value as Yaml;
use std::path::PathBuf;

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
}

fn imdb_config_path() -> PathBuf { home().join(".imdb.yml") }
fn imdb_data_dir() -> PathBuf { home().join(".imdb/data") }

pub fn should_import() -> bool {
    // Only import if watchit data is fresh and IMDB data exists.
    let watchit_list = config::list_path();
    let imdb_list = imdb_data_dir().join("list.json");
    !watchit_list.exists() && imdb_list.exists()
}

/// Perform the full import. Returns a status message for the footer.
pub fn import() -> String {
    let src_data = imdb_data_dir();
    let dst_data = config::data_dir();
    let _ = std::fs::create_dir_all(&dst_data);

    // Copy list.json + details.json + posters. Skip existing to be safe.
    let mut copied = 0usize;
    if let Ok(entries) = std::fs::read_dir(&src_data) {
        for e in entries.flatten() {
            let path = e.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let dst = dst_data.join(name);
            if dst.exists() { continue; }
            if std::fs::copy(&path, &dst).is_ok() { copied += 1; }
        }
    }

    // Translate ~/.imdb.yml → ~/.watchit/config.yml.
    let cfg = import_config().unwrap_or_default();
    cfg.save();

    format!("Imported {} files + config from ~/.imdb", copied)
}

fn import_config() -> Option<Config> {
    let data = std::fs::read_to_string(imdb_config_path()).ok()?;
    let y: Yaml = serde_yaml::from_str(&data).ok()?;

    let mut cfg = Config::default();

    if let Some(v) = y.get("rating_threshold").and_then(|x| x.as_f64()) {
        cfg.rating_min = v;
    }
    if let Some(v) = y.get("movie_limit").and_then(|x| x.as_u64()) {
        cfg.movie_limit = v as usize;
    }
    if let Some(v) = y.get("series_limit").and_then(|x| x.as_u64()) {
        cfg.series_limit = v as usize;
    }
    if let Some(v) = y.get("sort_by").and_then(|x| x.as_str()) {
        cfg.sort = v.to_string();
    }
    // show_movies/show_series → view
    let show_movies = y.get("show_movies").and_then(|x| x.as_bool()).unwrap_or(true);
    cfg.view = if show_movies { "movies".into() } else { "series".into() };

    if let Some(v) = y.get("tmdb_key").and_then(|x| x.as_str()) {
        cfg.tmdb_key = v.to_string();
    }
    if let Some(v) = y.get("tmdb_region").and_then(|x| x.as_str()) {
        cfg.region = v.to_string();
    }
    cfg.year_min = y.get("year_min").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
    cfg.year_max = y.get("year_max").and_then(|x| x.as_i64()).unwrap_or(0) as i32;

    cfg.wish_movies = string_array(y.get("movie_wish_list"));
    cfg.wish_series = string_array(y.get("series_wish_list"));
    cfg.dump_movies = string_array(y.get("movie_dump_list"));
    cfg.dump_series = string_array(y.get("series_dump_list"));

    // genre_filters: { Name: 0 | 1 | -1 }
    //   1  → include
    //  -1  → exclude
    //   0  → neutral (skip)
    if let Some(filters) = y.get("genre_filters").and_then(|x| x.as_mapping()) {
        for (k, v) in filters {
            let name = k.as_str().unwrap_or("").to_string();
            if name.is_empty() { continue; }
            let val = v.as_i64().unwrap_or(0);
            if val > 0 { cfg.genres_include.push(name); }
            else if val < 0 { cfg.genres_exclude.push(name); }
        }
    }

    Some(cfg)
}

fn string_array(v: Option<&Yaml>) -> Vec<String> {
    v.and_then(|x| x.as_sequence())
        .map(|seq| seq.iter()
            .filter_map(|y| y.as_str().map(String::from))
            .collect())
        .unwrap_or_default()
}
