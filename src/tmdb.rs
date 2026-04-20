//! Optional TMDb integration: maps an IMDb tconst → available streaming
//! providers in a given region via the /find and /watch/providers APIs.

use serde_json::Value as JsonValue;

fn http_get(url: &str) -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_secs(10))
        .build();
    agent.get(url)
        .set("Accept", "application/json")
        .call()
        .ok()?
        .into_string()
        .ok()
}

/// Return list of streaming provider names for the given IMDb id in the
/// given region (e.g. "US", "NO", "GB"). Requires a TMDb API v3 key.
pub fn streaming_providers(imdb_id: &str, region: &str, api_key: &str) -> Vec<String> {
    if api_key.is_empty() { return Vec::new(); }

    // First: find the TMDb id and media type from the IMDb id.
    let find_url = format!(
        "https://api.themoviedb.org/3/find/{}?external_source=imdb_id&api_key={}",
        imdb_id, api_key
    );
    let Some(body) = http_get(&find_url) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<JsonValue>(&body) else { return Vec::new() };

    let (media_type, tmdb_id) = if let Some(arr) = v.get("movie_results").and_then(|a| a.as_array()) {
        if let Some(first) = arr.first() {
            if let Some(id) = first.get("id").and_then(|x| x.as_i64()) {
                ("movie", id)
            } else { return Vec::new(); }
        } else if let Some(arr) = v.get("tv_results").and_then(|a| a.as_array()) {
            if let Some(first) = arr.first() {
                if let Some(id) = first.get("id").and_then(|x| x.as_i64()) {
                    ("tv", id)
                } else { return Vec::new(); }
            } else { return Vec::new(); }
        } else { return Vec::new(); }
    } else { return Vec::new(); };

    // Then: fetch providers for that title.
    let prov_url = format!(
        "https://api.themoviedb.org/3/{}/{}/watch/providers?api_key={}",
        media_type, tmdb_id, api_key
    );
    let Some(body) = http_get(&prov_url) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<JsonValue>(&body) else { return Vec::new() };

    let region_data = v.pointer(&format!("/results/{}", region)).cloned()
        .unwrap_or(JsonValue::Null);
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
}

/// Fetch list of ISO 3166-1 region codes TMDb supports. Uses a static list
/// for simplicity; full list would come from /configuration/countries.
pub fn known_regions() -> Vec<(&'static str, &'static str)> {
    vec![
        ("US", "United States"), ("GB", "United Kingdom"),
        ("CA", "Canada"), ("AU", "Australia"), ("NZ", "New Zealand"),
        ("NO", "Norway"), ("SE", "Sweden"), ("DK", "Denmark"),
        ("FI", "Finland"), ("IS", "Iceland"),
        ("DE", "Germany"), ("FR", "France"), ("ES", "Spain"),
        ("IT", "Italy"), ("NL", "Netherlands"), ("BE", "Belgium"),
        ("IE", "Ireland"), ("PL", "Poland"), ("PT", "Portugal"),
        ("AT", "Austria"), ("CH", "Switzerland"),
        ("JP", "Japan"), ("KR", "Korea"), ("HK", "Hong Kong"),
        ("SG", "Singapore"), ("IN", "India"),
        ("BR", "Brazil"), ("MX", "Mexico"), ("AR", "Argentina"),
        ("ZA", "South Africa"),
    ]
}
