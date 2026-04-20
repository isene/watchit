use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn watchit_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".watchit")
}

pub fn config_path() -> PathBuf { watchit_dir().join("config.yml") }
pub fn data_dir() -> PathBuf { watchit_dir().join("data") }
pub fn list_path() -> PathBuf { data_dir().join("list.json") }
pub fn details_path() -> PathBuf { data_dir().join("details.json") }

pub fn ensure_dirs() {
    let _ = std::fs::create_dir_all(watchit_dir());
    let _ = std::fs::create_dir_all(data_dir());
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(default)]
    pub tmdb_key: String,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default = "default_rating")]
    pub rating_min: f64,
    #[serde(default)]
    pub year_min: i32,
    #[serde(default)]
    pub year_max: i32,
    #[serde(default = "default_sort")]
    pub sort: String,
    #[serde(default = "default_view")]
    pub view: String,           // "movies" or "series"
    #[serde(default = "default_true")]
    pub show_posters: bool,
    #[serde(default = "default_movie_limit")]
    pub movie_limit: usize,
    #[serde(default = "default_series_limit")]
    pub series_limit: usize,
    #[serde(default)]
    pub wish_movies: Vec<String>,
    #[serde(default)]
    pub wish_series: Vec<String>,
    #[serde(default)]
    pub dump_movies: Vec<String>,
    #[serde(default)]
    pub dump_series: Vec<String>,
    #[serde(default)]
    pub genres_include: Vec<String>,
    #[serde(default)]
    pub genres_exclude: Vec<String>,
}

fn default_region() -> String { "US".into() }
fn default_rating() -> f64 { 0.0 }
fn default_sort() -> String { "rating".into() }
fn default_view() -> String { "movies".into() }
fn default_true() -> bool { true }
fn default_movie_limit() -> usize { 250 }
fn default_series_limit() -> usize { 250 }

impl Default for Config {
    fn default() -> Self {
        Self {
            tmdb_key: String::new(),
            region: default_region(),
            rating_min: default_rating(),
            year_min: 0,
            year_max: 0,
            sort: default_sort(),
            view: default_view(),
            show_posters: true,
            movie_limit: default_movie_limit(),
            series_limit: default_series_limit(),
            wish_movies: Vec::new(),
            wish_series: Vec::new(),
            dump_movies: Vec::new(),
            dump_series: Vec::new(),
            genres_include: Vec::new(),
            genres_exclude: Vec::new(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        if let Ok(data) = std::fs::read_to_string(config_path()) {
            serde_yaml::from_str(&data).unwrap_or_default()
        } else {
            let cfg = Self::default();
            cfg.save();
            cfg
        }
    }
    pub fn save(&self) {
        ensure_dirs();
        if let Ok(yaml) = serde_yaml::to_string(self) {
            let _ = std::fs::write(config_path(), yaml);
        }
    }
}
