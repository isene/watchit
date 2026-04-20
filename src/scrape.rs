//! IMDb scraping via JSON-LD embedded in Top 250 pages, and per-title
//! detail pages.

use crate::data::{Details, ListItem};
use serde_json::Value as JsonValue;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:109.0) Gecko/20100101 Firefox/117.0";

fn http_get(url: &str) -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(20))
        .redirects(5)
        .build();
    agent.get(url)
        .set("User-Agent", UA)
        .set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .set("Accept-Language", "en-US,en;q=0.9")
        .call()
        .ok()?
        .into_string()
        .ok()
}

/// Scrape an IMDb chart page (e.g. "chart/top" for movies, "chart/toptv"
/// for series) via embedded JSON-LD. Returns an ordered list of entries.
pub fn scrape_chart(chart_path: &str) -> Vec<ListItem> {
    let url = format!("https://www.imdb.com/{}/", chart_path);
    let Some(html) = http_get(&url) else { return Vec::new() };
    extract_json_ld(&html)
        .and_then(|ld| parse_item_list(&ld))
        .unwrap_or_default()
}

fn extract_json_ld(html: &str) -> Option<String> {
    let tag = "<script type=\"application/ld+json\">";
    let start = html.find(tag)?;
    let rest = &html[start + tag.len()..];
    let end = rest.find("</script>")?;
    Some(rest[..end].to_string())
}

fn parse_item_list(json: &str) -> Option<Vec<ListItem>> {
    let v: JsonValue = serde_json::from_str(json).ok()?;
    let items = v.get("itemListElement")?.as_array()?;
    let mut out = Vec::with_capacity(items.len());
    for li in items {
        let item = li.get("item")?;
        let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let id = extract_tconst(url)?;
        let title = item.get("name")
            .and_then(|n| n.as_str())
            .map(decode_html_entities)
            .unwrap_or_default();
        let rating = item.pointer("/aggregateRating/ratingValue")
            .and_then(|r| r.as_f64())
            .unwrap_or(0.0);
        out.push(ListItem { id, title, rating, year: 0, genres: Vec::new() });
    }
    Some(out)
}

fn extract_tconst(url: &str) -> Option<String> {
    // Patterns like "/title/tt1234567/" somewhere in the URL.
    let idx = url.find("/title/")?;
    let tail = &url[idx + "/title/".len()..];
    let end = tail.find('/')?;
    let id = &tail[..end];
    if id.starts_with("tt") && id.len() > 3 { Some(id.to_string()) } else { None }
}

/// Fetch the details page for a title and pull out plot, year, runtime,
/// directors, cast, genres, and poster URL via embedded JSON-LD.
pub fn fetch_details(tconst: &str) -> Details {
    let url = format!("https://www.imdb.com/title/{}/", tconst);
    let html = match http_get(&url) {
        Some(h) => h,
        None => return Details { id: tconst.into(), error: true, ..Details::default() },
    };
    let Some(ld) = extract_json_ld(&html) else {
        return Details { id: tconst.into(), error: true, ..Details::default() };
    };
    let Ok(v) = serde_json::from_str::<JsonValue>(&ld) else {
        return Details { id: tconst.into(), error: true, ..Details::default() };
    };

    let title = v.get("name").and_then(|n| n.as_str()).map(decode_html_entities).unwrap_or_default();
    let plot = v.get("description").and_then(|n| n.as_str()).map(decode_html_entities).unwrap_or_default();
    let rating = v.pointer("/aggregateRating/ratingValue").and_then(|r| r.as_f64()).unwrap_or(0.0);
    let poster_url = v.get("image").and_then(|n| n.as_str()).unwrap_or("").to_string();

    let year = v.get("datePublished").and_then(|n| n.as_str())
        .and_then(|s| s.get(..4))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let runtime = v.get("duration").and_then(|n| n.as_str()).map(parse_iso_duration).unwrap_or_default();

    let genres = v.get("genre").map(extract_string_array).unwrap_or_default();
    let directors = v.get("director").map(extract_name_array).unwrap_or_default();
    let writers = v.get("creator").map(extract_name_array).unwrap_or_default();
    let stars = v.get("actor").map(extract_name_array).unwrap_or_default();

    Details {
        id: tconst.into(),
        title, year, rating, runtime, plot, genres,
        directors, writers, stars, poster_url,
        streaming: Vec::new(),
        error: false,
        ..Default::default()
    }
}

fn extract_string_array(v: &JsonValue) -> Vec<String> {
    match v {
        JsonValue::String(s) => vec![s.clone()],
        JsonValue::Array(arr) => arr.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

fn extract_name_array(v: &JsonValue) -> Vec<String> {
    match v {
        JsonValue::Object(_) => v.get("name")
            .and_then(|n| n.as_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        JsonValue::Array(arr) => arr.iter()
            .filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

/// Convert ISO 8601 duration "PT2H30M" to "2h 30m".
fn parse_iso_duration(s: &str) -> String {
    let s = s.trim_start_matches("PT");
    let mut out = String::new();
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            match c {
                'H' => { if !num.is_empty() { out.push_str(&num); out.push_str("h "); num.clear(); } }
                'M' => { if !num.is_empty() { out.push_str(&num); out.push_str("m "); num.clear(); } }
                'S' => { if !num.is_empty() { out.push_str(&num); out.push('s'); num.clear(); } }
                _ => num.clear(),
            }
        }
    }
    out.trim().to_string()
}

fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = s[i..].find(';') {
                let entity = &s[i + 1..i + semi];
                match entity {
                    "amp" => { out.push('&'); i += semi + 1; continue; }
                    "lt" => { out.push('<'); i += semi + 1; continue; }
                    "gt" => { out.push('>'); i += semi + 1; continue; }
                    "quot" => { out.push('"'); i += semi + 1; continue; }
                    "apos" | "#39" => { out.push('\''); i += semi + 1; continue; }
                    _ => {
                        if let Some(num) = entity.strip_prefix('#') {
                            if let Ok(code) = num.parse::<u32>() {
                                if let Some(c) = char::from_u32(code) {
                                    out.push(c); i += semi + 1; continue;
                                }
                            }
                        }
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Search IMDb's autocomplete endpoint. Returns up to `max` results.
pub fn search(query: &str, max: usize) -> Vec<ListItem> {
    if query.is_empty() { return Vec::new(); }
    let q = urlencode(query);
    let prefix = query.chars().next().unwrap_or('a').to_ascii_lowercase();
    let url = format!("https://v3.sg.media-imdb.com/suggestion/{}/{}.json", prefix, q);
    let Some(body) = http_get(&url) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<JsonValue>(&body) else { return Vec::new() };
    let items = v.get("d").and_then(|d| d.as_array()).cloned().unwrap_or_default();
    items.into_iter()
        .filter_map(|it| {
            let id = it.get("id").and_then(|x| x.as_str())?.to_string();
            if !id.starts_with("tt") { return None; }
            let title = it.get("l").and_then(|x| x.as_str()).map(String::from).unwrap_or_default();
            let year = it.get("y").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            Some(ListItem { id, title, rating: 0.0, year, genres: Vec::new() })
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
