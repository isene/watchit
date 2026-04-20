mod config;
mod data;
mod import;
mod scrape;
mod tmdb;

use config::Config;
use crust::{Crust, Input, Pane};
use crust::style;
use data::{Database, Details, DetailsCache, ListItem};
use std::collections::HashSet;
use std::sync::mpsc;

fn main() {
    config::ensure_dirs();

    // First run: import existing Ruby IMDB data if present.
    let import_msg = if import::should_import() {
        let msg = import::import();
        eprintln!("watchit: {}", msg);
        Some(msg)
    } else {
        None
    };

    let cfg = Config::load();

    Crust::init();
    let mut app = App::new(cfg);
    app.load_all();

    if let Some(msg) = import_msg {
        app.footer_say(&format!(" {}", msg), 46);
    } else if app.db.movies.is_empty() && app.db.series.is_empty() {
        app.footer_say(" No data: press I to scrape Top 250 (takes 1-2 min)", 226);
    }

    app.render_all();

    loop {
        let Some(key) = Input::getchr(Some(1)) else {
            if app.poll_async() { app.render_all(); }
            continue;
        };
        if app.poll_async() { app.render_all(); }

        if app.search_mode {
            app.handle_search_key(&key);
            app.render_all();
            continue;
        }

        match key.as_str() {
            "q" => break,
            "?" => app.show_help(),
            "TAB" => { app.next_focus(); app.render_all(); }
            "S-TAB" | "BACKTAB" => { app.prev_focus(); app.render_all(); }
            "j" | "DOWN" => { app.move_focus(1); app.render_all(); }
            "k" | "UP" => { app.move_focus(-1); app.render_all(); }
            "PgDOWN" => { app.page_focus(1); app.render_all(); }
            "PgUP" => { app.page_focus(-1); app.render_all(); }
            "HOME" => { app.first_in_focus(); app.render_all(); }
            "END" => { app.last_in_focus(); app.render_all(); }
            "+" => { app.action_plus(); app.render_all(); }
            "-" => { app.action_minus(); app.render_all(); }
            " " | "SPACE" => { app.clear_genre_filter(); app.render_all(); }
            "l" => { app.toggle_view(); app.render_all(); }
            "o" => { app.toggle_sort(); app.render_all(); }
            "r" => { app.set_rating_min(); app.render_all(); }
            "y" => { app.set_year_min(); app.render_all(); }
            "Y" => { app.set_year_max(); app.render_all(); }
            "/" => { app.begin_search(); app.render_all(); }
            "I" => { app.start_full_scrape(); app.render_all(); }
            "i" => { app.start_incremental(); app.render_all(); }
            "f" => { app.refetch_current(); app.render_all(); }
            "k" => { app.set_tmdb_key(); app.render_all(); }
            "R" => { app.set_region(); app.render_all(); }
            "D" => { app.remove_duplicates(); app.render_all(); }
            "v" => { app.verify_data(); app.render_all(); }
            "L" => { app.load_additional_lists(); app.render_all(); }
            "W" => { app.cfg.save(); app.footer_say(" Config saved", 46); }
            "ENTER" => { app.render_all(); }
            _ => {}
        }
    }

    app.cfg.save();
    Crust::cleanup();
    Crust::clear_screen();
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Focus { List, Genres, Wish, Dump }

struct App {
    cfg: Config,
    db: Database,
    details: DetailsCache,
    cols: u16,
    rows: u16,
    header: Pane,
    list: Pane,
    genres: Pane,
    wish: Pane,
    dump: Pane,
    detail: Pane,
    footer: Pane,
    focus: Focus,

    list_idx: usize,
    genre_idx: usize,
    wish_idx: usize,
    dump_idx: usize,

    all_genres: Vec<String>,

    // Filtered ids for current view (keeps ordering stable between renders).
    filtered: Vec<String>,

    search_mode: bool,
    search_buf: String,
    search_results: Vec<ListItem>,
    search_idx: usize,

    // Async: background scrape/fetch tasks.
    scrape_rx: Option<mpsc::Receiver<ScrapeResult>>,
    detail_rx: Option<mpsc::Receiver<Details>>,

    status_msg: Option<(String, u8)>,

    image_display: Option<glow::Display>,
    current_poster: Option<String>,
}

enum ScrapeResult {
    Full(Vec<ListItem>, Vec<ListItem>),
    Progress(String),
}

impl App {
    fn new(cfg: Config) -> Self {
        let (cols, rows) = Crust::terminal_size();
        let (header, list, genres, wish, dump, detail, footer) = Self::build_panes(cols, rows);
        Self {
            cfg,
            db: Database::default(),
            details: DetailsCache::new(),
            cols, rows,
            header, list, genres, wish, dump, detail, footer,
            focus: Focus::List,
            list_idx: 0, genre_idx: 0, wish_idx: 0, dump_idx: 0,
            all_genres: Vec::new(),
            filtered: Vec::new(),
            search_mode: false,
            search_buf: String::new(),
            search_results: Vec::new(),
            search_idx: 0,
            scrape_rx: None,
            detail_rx: None,
            status_msg: None,
            image_display: None,
            current_poster: None,
        }
    }

    fn build_panes(cols: u16, rows: u16)
        -> (Pane, Pane, Pane, Pane, Pane, Pane, Pane)
    {
        let mut header = Pane::new(1, 1, cols, 1, 255, 236);
        header.wrap = false;
        let content_h = rows.saturating_sub(3);
        let mut list = Pane::new(2, 3, 50, content_h, 252, 0);
        list.wrap = false;
        list.border = true;
        let mut genres = Pane::new(53, 3, 16, content_h, 248, 232);
        genres.wrap = false;
        let wish_h = rows / 2 - 1;
        let mut wish = Pane::new(70, 3, 30, wish_h, 64, 232);
        wish.wrap = false;
        let dump_y = 3 + wish_h + 1;
        let mut dump = Pane::new(70, dump_y, 30, content_h.saturating_sub(wish_h + 1), 130, 232);
        dump.wrap = false;
        let detail_x = 102;
        let mut detail = Pane::new(detail_x, 3, cols.saturating_sub(detail_x), content_h, 255, 0);
        detail.wrap = true;
        let mut footer = Pane::new(1, rows, cols, 1, 255, 236);
        footer.wrap = false;
        (header, list, genres, wish, dump, detail, footer)
    }

    fn load_all(&mut self) {
        self.db = Database::load(&config::list_path());
        self.details = data::load_details_cache(&config::details_path());
        self.rebuild_genres();
        self.rebuild_filtered();
    }

    fn rebuild_genres(&mut self) {
        let mut set: HashSet<String> = HashSet::new();
        for d in self.details.values() {
            for g in &d.genres { set.insert(g.clone()); }
        }
        for it in self.db.movies.iter().chain(self.db.series.iter()) {
            for g in &it.genres { set.insert(g.clone()); }
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        self.all_genres = v;
    }

    fn rebuild_filtered(&mut self) {
        let source: &[ListItem] = if self.cfg.view == "movies" { &self.db.movies } else { &self.db.series };
        let dump_set: HashSet<&String> = if self.cfg.view == "movies" {
            self.cfg.dump_movies.iter().collect()
        } else {
            self.cfg.dump_series.iter().collect()
        };
        let mut ids: Vec<(String, f64, String)> = source.iter()
            .filter(|it| !dump_set.contains(&it.id))
            .filter(|it| it.rating >= self.cfg.rating_min)
            .filter(|it| self.cfg.year_min == 0 || self.item_year(it) >= self.cfg.year_min)
            .filter(|it| self.cfg.year_max == 0 || self.item_year(it) <= self.cfg.year_max)
            .filter(|it| self.matches_genres(it))
            .map(|it| (it.id.clone(), it.rating, it.title.clone()))
            .collect();
        match self.cfg.sort.as_str() {
            "alpha" => ids.sort_by(|a, b| a.2.to_lowercase().cmp(&b.2.to_lowercase())),
            _ => ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)),
        }
        self.filtered = ids.into_iter().map(|(i, _, _)| i).collect();
        if self.list_idx >= self.filtered.len() {
            self.list_idx = self.filtered.len().saturating_sub(1);
        }
    }

    fn item_year(&self, it: &ListItem) -> i32 {
        if it.year > 0 { return it.year; }
        self.details.get(&it.id).map(|d| d.year).unwrap_or(0)
    }

    fn matches_genres(&self, it: &ListItem) -> bool {
        let genres: Vec<&String> = if !it.genres.is_empty() {
            it.genres.iter().collect()
        } else if let Some(d) = self.details.get(&it.id) {
            d.genres.iter().collect()
        } else {
            Vec::new()
        };
        for inc in &self.cfg.genres_include {
            if !genres.iter().any(|g| *g == inc) { return false; }
        }
        for exc in &self.cfg.genres_exclude {
            if genres.iter().any(|g| *g == exc) { return false; }
        }
        true
    }

    fn list_lookup(&self, id: &str) -> Option<&ListItem> {
        let src = if self.cfg.view == "movies" { &self.db.movies } else { &self.db.series };
        src.iter().find(|it| it.id == id)
    }

    fn current_id(&self) -> Option<String> {
        match self.focus {
            Focus::List => self.filtered.get(self.list_idx).cloned(),
            Focus::Wish => {
                let wish = if self.cfg.view == "movies" { &self.cfg.wish_movies } else { &self.cfg.wish_series };
                wish.get(self.wish_idx).cloned()
            }
            Focus::Dump => {
                let dump = if self.cfg.view == "movies" { &self.cfg.dump_movies } else { &self.cfg.dump_series };
                dump.get(self.dump_idx).cloned()
            }
            Focus::Genres => None,
        }
    }

    fn render_all(&mut self) {
        self.render_header();
        self.render_list();
        self.render_genres();
        self.render_wish();
        self.render_dump();
        self.render_detail();
        self.render_footer();
    }

    fn render_header(&mut self) {
        let view = if self.cfg.view == "movies" { "Movies" } else { "Series" };
        let filter = format!(
            "rating>={:.1}  year>={}  year<={}  sort={}",
            self.cfg.rating_min,
            if self.cfg.year_min == 0 { "*".into() } else { self.cfg.year_min.to_string() },
            if self.cfg.year_max == 0 { "*".into() } else { self.cfg.year_max.to_string() },
            self.cfg.sort,
        );
        let counts = format!("M:{}  S:{}", self.db.movies.len(), self.db.series.len());
        let text = format!(" watchit  [{}]  {}  •  {}", view, filter, counts);
        self.header.say(&style::bold(&text));
    }

    fn render_list(&mut self) {
        let mut lines = Vec::new();
        let title = format!("▸ {}  ({} shown)", if self.cfg.view == "movies" { "Movies" } else { "Series" }, self.filtered.len());
        lines.push(style::bold(&style::fg(&title, 81)));
        lines.push(String::new());
        for (i, id) in self.filtered.iter().enumerate() {
            let item = self.list_lookup(id);
            let (title, rating) = item.map(|it| (it.title.clone(), it.rating)).unwrap_or_default();
            let year = item.map(|it| self.item_year(it)).unwrap_or(0);
            let year_s = if year > 0 { format!(" ({})", year) } else { String::new() };
            let row = format!("{:>4.1}  {}{}", rating, title, year_s);
            let marker = if i == self.list_idx && self.focus == Focus::List { "→ " } else { "  " };
            let line = format!("{}{}", marker, row);
            if i == self.list_idx && self.focus == Focus::List {
                lines.push(style::bold(&style::underline(&line)));
            } else {
                lines.push(line);
            }
        }
        self.list.set_text(&lines.join("\n"));
        self.list.ix = self.compute_scroll(self.list_idx, self.filtered.len(), self.list.h as usize);
        self.list.full_refresh();
        if self.list.border { self.list.border_refresh(); }
    }

    fn render_genres(&mut self) {
        let mut lines = Vec::new();
        lines.push(style::bold(&style::fg("Genres", 81)));
        lines.push(String::new());
        for (i, g) in self.all_genres.iter().enumerate() {
            let mark = if self.cfg.genres_include.contains(g) { style::fg("+ ", 46) }
                else if self.cfg.genres_exclude.contains(g) { style::fg("- ", 196) }
                else { "  ".into() };
            let focused = self.focus == Focus::Genres && i == self.genre_idx;
            let text = format!("{}{}", mark, g);
            if focused {
                lines.push(style::underline(&style::bold(&text)));
            } else {
                lines.push(text);
            }
        }
        self.genres.set_text(&lines.join("\n"));
        self.genres.ix = self.compute_scroll(self.genre_idx, self.all_genres.len(), self.genres.h as usize);
        self.genres.full_refresh();
    }

    fn render_wish(&mut self) {
        let ids = if self.cfg.view == "movies" { self.cfg.wish_movies.clone() } else { self.cfg.wish_series.clone() };
        let mut lines = vec![style::bold(&style::fg("Wish", 82)), String::new()];
        for (i, id) in ids.iter().enumerate() {
            let title = self.list_lookup(id).map(|it| it.title.clone())
                .or_else(|| self.details.get(id).map(|d| d.title.clone()))
                .unwrap_or_else(|| id.clone());
            let focused = self.focus == Focus::Wish && i == self.wish_idx;
            let line = if focused {
                style::underline(&style::bold(&format!("→ {}", title)))
            } else {
                format!("  {}", title)
            };
            lines.push(line);
        }
        self.wish.set_text(&lines.join("\n"));
        self.wish.ix = self.compute_scroll(self.wish_idx, ids.len(), self.wish.h as usize);
        self.wish.full_refresh();
    }

    fn render_dump(&mut self) {
        let ids = if self.cfg.view == "movies" { self.cfg.dump_movies.clone() } else { self.cfg.dump_series.clone() };
        let mut lines = vec![style::bold(&style::fg("Dump", 130)), String::new()];
        for (i, id) in ids.iter().enumerate() {
            let title = self.list_lookup(id).map(|it| it.title.clone())
                .or_else(|| self.details.get(id).map(|d| d.title.clone()))
                .unwrap_or_else(|| id.clone());
            let focused = self.focus == Focus::Dump && i == self.dump_idx;
            let line = if focused {
                style::underline(&style::bold(&format!("→ {}", title)))
            } else {
                format!("  {}", title)
            };
            lines.push(line);
        }
        self.dump.set_text(&lines.join("\n"));
        self.dump.ix = self.compute_scroll(self.dump_idx, ids.len(), self.dump.h as usize);
        self.dump.full_refresh();
    }

    fn render_detail(&mut self) {
        let Some(id) = self.current_id() else {
            self.detail.set_text("");
            self.detail.full_refresh();
            self.clear_poster();
            return;
        };
        let item = self.list_lookup(&id).cloned();
        let det = self.details.get(&id).cloned();

        let mut lines = Vec::new();
        let title = det.as_ref().map(|d| d.title.clone())
            .or_else(|| item.as_ref().map(|it| it.title.clone()))
            .unwrap_or_else(|| id.clone());
        lines.push(style::bold(&style::fg(&title, 226)));
        lines.push(style::fg(&format!("imdb.com/title/{}/", id), 240));
        lines.push(String::new());

        if let Some(d) = det.as_ref() {
            if d.year > 0 || !d.runtime.is_empty() || d.rating > 0.0 {
                lines.push(format!(
                    "{}  {}  Rating: {:.1}",
                    if d.year > 0 { d.year.to_string() } else { "-".into() },
                    if d.runtime.is_empty() { "-".into() } else { d.runtime.clone() },
                    d.rating,
                ));
            }
            if !d.genres.is_empty() {
                lines.push(style::fg(&format!("Genre: {}", d.genres.join(", ")), 117));
            }
            if !d.directors.is_empty() {
                lines.push(style::fg(&format!("Director: {}", d.directors.join(", ")), 117));
            }
            if !d.writers.is_empty() {
                lines.push(style::fg(&format!("Writer: {}", d.writers.join(", ")), 117));
            }
            if !d.stars.is_empty() {
                lines.push(style::fg(&format!("Stars: {}", d.stars.join(", ")), 117));
            }
            lines.push(String::new());
            if !d.plot.is_empty() {
                lines.push(d.plot.clone());
                lines.push(String::new());
            }
            if !d.streaming.is_empty() {
                lines.push(style::fg(&format!("Streaming ({}): {}", self.cfg.region, d.streaming.join(", ")), 82));
            }
        } else {
            lines.push(style::fg("Press f to fetch details", 245));
        }

        self.detail.set_text(&lines.join("\n"));
        self.detail.ix = 0;
        self.detail.full_refresh();

        // Poster: prefer the local cached JPG (imports + previous downloads).
        // Fall back to downloading from poster_url if only that's available.
        if self.cfg.show_posters {
            let local = config::data_dir().join(format!("{}.jpg", id));
            if local.exists() {
                self.show_poster_path(&id, &local);
            } else if let Some(url) = det.as_ref().map(|d| d.poster_url.clone()).filter(|s| !s.is_empty()) {
                self.show_poster(&id, &url);
            } else {
                self.clear_poster();
            }
        } else {
            self.clear_poster();
        }
    }

    fn show_poster_path(&mut self, id: &str, path: &std::path::Path) {
        if self.current_poster.as_deref() == Some(id) { return; }
        self.clear_poster();
        let display = glow::Display::new();
        if !display.supported() { return; }
        let top = 15u16;
        let img_x = self.detail.x;
        let img_y = self.detail.y + top;
        let img_w = self.detail.w.saturating_sub(2);
        let img_h = self.detail.h.saturating_sub(top + 1);
        if img_h < 4 { return; }
        self.image_display = Some(display);
        if let Some(ref mut disp) = self.image_display {
            disp.show(path.to_string_lossy().as_ref(), img_x, img_y, img_w, img_h);
        }
        self.current_poster = Some(id.to_string());
    }

    fn show_poster(&mut self, id: &str, url: &str) {
        // Cache to ~/.watchit/data/<id>.jpg
        let path = config::data_dir().join(format!("{}.jpg", id));
        if !path.exists() {
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(5))
                .timeout_read(std::time::Duration::from_secs(15))
                .redirects(5)
                .build();
            if let Ok(resp) = agent.get(url).call() {
                let mut bytes = Vec::new();
                if std::io::Read::read_to_end(&mut resp.into_reader(), &mut bytes).is_ok() && bytes.len() > 100 {
                    let _ = std::fs::write(&path, &bytes);
                }
            }
        }
        if !path.exists() { self.clear_poster(); return; }
        if self.current_poster.as_deref() == Some(id) { return; }

        self.clear_poster();
        let display = glow::Display::new();
        if !display.supported() { return; }

        // Put poster in bottom half of the detail pane.
        let top = 15u16;
        let img_x = self.detail.x;
        let img_y = self.detail.y + top;
        let img_w = self.detail.w.saturating_sub(2);
        let img_h = self.detail.h.saturating_sub(top + 1);
        if img_h < 4 { return; }

        self.image_display = Some(display);
        if let Some(ref mut disp) = self.image_display {
            disp.show(path.to_string_lossy().as_ref(), img_x, img_y, img_w, img_h);
        }
        self.current_poster = Some(id.to_string());
    }

    fn clear_poster(&mut self) {
        if let Some(ref mut disp) = self.image_display {
            disp.clear(self.detail.x, self.detail.y, self.detail.w, self.detail.h, self.cols, self.rows);
        }
        self.image_display = None;
        self.current_poster = None;
    }

    fn render_footer(&mut self) {
        if let Some((ref msg, color)) = self.status_msg {
            self.footer.say(&style::fg(msg, color));
        } else {
            let hint = " ?:Help  TAB:Focus  j/k:Move  +/-:Wish/Dump  /:Search  l:Movies/Series  o:Sort  r/y/Y:Filter  I:Scrape  f:Fetch  k:TMDb  R:Region  q:Quit";
            self.footer.say(&style::fg(hint, 245));
        }
    }

    fn footer_say(&mut self, msg: &str, color: u8) {
        self.status_msg = Some((msg.to_string(), color));
        self.render_footer();
    }

    fn compute_scroll(&self, idx: usize, total: usize, h: usize) -> usize {
        if total <= h { return 0; }
        let half = h / 2;
        if idx < half { 0 }
        else if idx + half >= total { total.saturating_sub(h) }
        else { idx.saturating_sub(half) }
    }

    // --- Focus/movement ---

    fn next_focus(&mut self) {
        self.focus = match self.focus {
            Focus::List => Focus::Genres,
            Focus::Genres => Focus::Wish,
            Focus::Wish => Focus::Dump,
            Focus::Dump => Focus::List,
        };
    }
    fn prev_focus(&mut self) {
        self.focus = match self.focus {
            Focus::List => Focus::Dump,
            Focus::Genres => Focus::List,
            Focus::Wish => Focus::Genres,
            Focus::Dump => Focus::Wish,
        };
    }

    fn move_focus(&mut self, n: i32) {
        match self.focus {
            Focus::List => move_bounded(&mut self.list_idx, n, self.filtered.len()),
            Focus::Genres => move_bounded(&mut self.genre_idx, n, self.all_genres.len()),
            Focus::Wish => {
                let len = if self.cfg.view == "movies" { self.cfg.wish_movies.len() } else { self.cfg.wish_series.len() };
                move_bounded(&mut self.wish_idx, n, len);
            }
            Focus::Dump => {
                let len = if self.cfg.view == "movies" { self.cfg.dump_movies.len() } else { self.cfg.dump_series.len() };
                move_bounded(&mut self.dump_idx, n, len);
            }
        }
    }
    fn page_focus(&mut self, dir: i32) {
        let h = match self.focus {
            Focus::List => self.list.h as usize,
            Focus::Genres => self.genres.h as usize,
            Focus::Wish => self.wish.h as usize,
            Focus::Dump => self.dump.h as usize,
        };
        self.move_focus(dir * h as i32);
    }
    fn first_in_focus(&mut self) {
        match self.focus {
            Focus::List => self.list_idx = 0,
            Focus::Genres => self.genre_idx = 0,
            Focus::Wish => self.wish_idx = 0,
            Focus::Dump => self.dump_idx = 0,
        }
    }
    fn last_in_focus(&mut self) {
        match self.focus {
            Focus::List => self.list_idx = self.filtered.len().saturating_sub(1),
            Focus::Genres => self.genre_idx = self.all_genres.len().saturating_sub(1),
            Focus::Wish => {
                let n = if self.cfg.view == "movies" { self.cfg.wish_movies.len() } else { self.cfg.wish_series.len() };
                self.wish_idx = n.saturating_sub(1);
            }
            Focus::Dump => {
                let n = if self.cfg.view == "movies" { self.cfg.dump_movies.len() } else { self.cfg.dump_series.len() };
                self.dump_idx = n.saturating_sub(1);
            }
        }
    }

    // --- Actions ---

    fn action_plus(&mut self) {
        match self.focus {
            Focus::List => { if let Some(id) = self.current_id() { self.add_to_wish(&id); } }
            Focus::Genres => {
                if let Some(g) = self.all_genres.get(self.genre_idx).cloned() {
                    self.cfg.genres_exclude.retain(|x| x != &g);
                    if !self.cfg.genres_include.contains(&g) {
                        self.cfg.genres_include.push(g);
                    } else {
                        self.cfg.genres_include.retain(|x| x != self.all_genres.get(self.genre_idx).unwrap());
                    }
                    self.rebuild_filtered();
                }
            }
            _ => {}
        }
    }
    fn action_minus(&mut self) {
        match self.focus {
            Focus::List => { if let Some(id) = self.current_id() { self.add_to_dump(&id); } }
            Focus::Genres => {
                if let Some(g) = self.all_genres.get(self.genre_idx).cloned() {
                    self.cfg.genres_include.retain(|x| x != &g);
                    if !self.cfg.genres_exclude.contains(&g) {
                        self.cfg.genres_exclude.push(g);
                    } else {
                        self.cfg.genres_exclude.retain(|x| x != self.all_genres.get(self.genre_idx).unwrap());
                    }
                    self.rebuild_filtered();
                }
            }
            Focus::Wish => {
                if let Some(id) = self.current_id() { self.remove_from_wish(&id); }
            }
            Focus::Dump => {
                if let Some(id) = self.current_id() { self.remove_from_dump(&id); }
            }
        }
    }
    fn clear_genre_filter(&mut self) {
        if let Some(g) = self.all_genres.get(self.genre_idx).cloned() {
            self.cfg.genres_include.retain(|x| x != &g);
            self.cfg.genres_exclude.retain(|x| x != &g);
            self.rebuild_filtered();
        }
    }

    fn add_to_wish(&mut self, id: &str) {
        let list = if self.cfg.view == "movies" { &mut self.cfg.wish_movies } else { &mut self.cfg.wish_series };
        if !list.contains(&id.to_string()) { list.push(id.to_string()); }
    }
    fn remove_from_wish(&mut self, id: &str) {
        let list = if self.cfg.view == "movies" { &mut self.cfg.wish_movies } else { &mut self.cfg.wish_series };
        list.retain(|x| x != id);
        if self.wish_idx >= list.len() { self.wish_idx = list.len().saturating_sub(1); }
    }
    fn add_to_dump(&mut self, id: &str) {
        let list = if self.cfg.view == "movies" { &mut self.cfg.dump_movies } else { &mut self.cfg.dump_series };
        if !list.contains(&id.to_string()) { list.push(id.to_string()); }
        self.rebuild_filtered();
    }
    fn remove_from_dump(&mut self, id: &str) {
        let list = if self.cfg.view == "movies" { &mut self.cfg.dump_movies } else { &mut self.cfg.dump_series };
        list.retain(|x| x != id);
        if self.dump_idx >= list.len() { self.dump_idx = list.len().saturating_sub(1); }
        self.rebuild_filtered();
    }

    fn toggle_view(&mut self) {
        self.cfg.view = if self.cfg.view == "movies" { "series".into() } else { "movies".into() };
        self.list_idx = 0;
        self.rebuild_filtered();
    }
    fn toggle_sort(&mut self) {
        self.cfg.sort = if self.cfg.sort == "rating" { "alpha".into() } else { "rating".into() };
        self.rebuild_filtered();
    }

    fn set_rating_min(&mut self) {
        let s = self.footer.ask(" Minimum rating (0-10): ", &format!("{:.1}", self.cfg.rating_min));
        if let Ok(v) = s.trim().parse::<f64>() { self.cfg.rating_min = v.clamp(0.0, 10.0); }
        self.rebuild_filtered();
    }
    fn set_year_min(&mut self) {
        let s = self.footer.ask(" Min year (0 for none): ", &self.cfg.year_min.to_string());
        if let Ok(v) = s.trim().parse::<i32>() { self.cfg.year_min = v; }
        self.rebuild_filtered();
    }
    fn set_year_max(&mut self) {
        let s = self.footer.ask(" Max year (0 for none): ", &self.cfg.year_max.to_string());
        if let Ok(v) = s.trim().parse::<i32>() { self.cfg.year_max = v; }
        self.rebuild_filtered();
    }
    fn set_tmdb_key(&mut self) {
        let s = self.footer.ask(" TMDb v3 API key: ", &self.cfg.tmdb_key);
        if !s.trim().is_empty() { self.cfg.tmdb_key = s.trim().into(); }
    }
    fn set_region(&mut self) {
        let s = self.footer.ask(" Region (ISO code, e.g. US): ", &self.cfg.region);
        if !s.trim().is_empty() { self.cfg.region = s.trim().to_uppercase(); }
    }

    /// Check every title in the database and queue a re-fetch for any whose
    /// details are missing or marked error. Same pattern as `i` but scans
    /// the full list, not just the current filtered view.
    fn verify_data(&mut self) {
        let mut missing = Vec::new();
        for it in self.db.movies.iter().chain(self.db.series.iter()) {
            let needs = self.details.get(&it.id)
                .map(|d| d.error || d.title.is_empty())
                .unwrap_or(true);
            if needs { missing.push(it.id.clone()); }
            if missing.len() >= 20 { break; }
        }
        if missing.is_empty() {
            self.footer_say(" All details valid", 46);
            return;
        }
        self.footer_say(&format!(" Verifying {} missing/stale...", missing.len()), 226);
        self.render_footer();
        let (tx, rx) = mpsc::channel();
        let key = self.cfg.tmdb_key.clone();
        let region = self.cfg.region.clone();
        std::thread::spawn(move || {
            for id in missing {
                let mut d = scrape::fetch_details(&id);
                if !d.error && !key.is_empty() {
                    d.streaming = tmdb::streaming_providers(&id, &region, &key);
                }
                let _ = tx.send(d);
            }
        });
        self.detail_rx = Some(rx);
    }

    /// Load additional IMDb chart pages ("popular" movies + TV, "trending")
    /// and merge any new titles into the database without duplicates.
    fn load_additional_lists(&mut self) {
        if self.scrape_rx.is_some() {
            self.footer_say(" Scrape already running", 226);
            return;
        }
        self.footer_say(" Fetching popular + trending lists...", 226);
        self.render_footer();
        let (tx, rx) = mpsc::channel();
        let existing_movies: std::collections::HashSet<String> =
            self.db.movies.iter().map(|i| i.id.clone()).collect();
        let existing_series: std::collections::HashSet<String> =
            self.db.series.iter().map(|i| i.id.clone()).collect();
        let mut new_movies = self.db.movies.clone();
        let mut new_series = self.db.series.clone();
        std::thread::spawn(move || {
            let _ = tx.send(ScrapeResult::Progress("Popular movies...".into()));
            for it in scrape::scrape_chart("chart/moviemeter") {
                if !existing_movies.contains(&it.id) { new_movies.push(it); }
            }
            let _ = tx.send(ScrapeResult::Progress("Popular series...".into()));
            for it in scrape::scrape_chart("chart/tvmeter") {
                if !existing_series.contains(&it.id) { new_series.push(it); }
            }
            let _ = tx.send(ScrapeResult::Full(new_movies, new_series));
        });
        self.scrape_rx = Some(rx);
    }

    fn remove_duplicates(&mut self) {
        let mut seen: HashSet<String> = HashSet::new();
        self.db.movies.retain(|it| seen.insert(it.id.clone()));
        seen.clear();
        self.db.series.retain(|it| seen.insert(it.id.clone()));
        self.db.save(&config::list_path());
        self.rebuild_filtered();
        self.footer_say(" Duplicates removed", 46);
    }

    // --- Async scrape/fetch ---

    fn start_full_scrape(&mut self) {
        if self.scrape_rx.is_some() {
            self.footer_say(" Scrape already running", 226);
            return;
        }
        self.footer_say(" Scraping IMDb Top 250...", 226);
        self.render_footer();
        let (tx, rx) = mpsc::channel();
        let movie_limit = self.cfg.movie_limit;
        let series_limit = self.cfg.series_limit;
        std::thread::spawn(move || {
            let _ = tx.send(ScrapeResult::Progress("Fetching movies...".into()));
            let mut movies = scrape::scrape_chart("chart/top");
            movies.truncate(movie_limit);
            let _ = tx.send(ScrapeResult::Progress("Fetching series...".into()));
            let mut series = scrape::scrape_chart("chart/toptv");
            series.truncate(series_limit);
            let _ = tx.send(ScrapeResult::Full(movies, series));
        });
        self.scrape_rx = Some(rx);
    }

    fn start_incremental(&mut self) {
        if self.detail_rx.is_some() {
            self.footer_say(" Fetch already running", 226);
            return;
        }
        // Find first title without details and fetch it in the background.
        let missing: Vec<String> = self.filtered.iter()
            .filter(|id| self.details.get(id.as_str()).map(|d| d.error || d.title.is_empty()).unwrap_or(true))
            .take(5)
            .cloned()
            .collect();
        if missing.is_empty() {
            self.footer_say(" All details present", 46);
            return;
        }
        self.footer_say(&format!(" Fetching {} missing...", missing.len()), 226);
        self.render_footer();
        let (tx, rx) = mpsc::channel();
        let key = self.cfg.tmdb_key.clone();
        let region = self.cfg.region.clone();
        std::thread::spawn(move || {
            for id in missing {
                let mut d = scrape::fetch_details(&id);
                if !d.error && !key.is_empty() {
                    d.streaming = tmdb::streaming_providers(&id, &region, &key);
                }
                let _ = tx.send(d);
            }
        });
        self.detail_rx = Some(rx);
    }

    fn refetch_current(&mut self) {
        let Some(id) = self.current_id() else { return };
        self.footer_say(&format!(" Re-fetching {}...", id), 226);
        self.render_footer();
        let (tx, rx) = mpsc::channel();
        let id_clone = id.clone();
        let key = self.cfg.tmdb_key.clone();
        let region = self.cfg.region.clone();
        std::thread::spawn(move || {
            let mut d = scrape::fetch_details(&id_clone);
            if !d.error && !key.is_empty() {
                d.streaming = tmdb::streaming_providers(&id_clone, &region, &key);
            }
            let _ = tx.send(d);
        });
        self.detail_rx = Some(rx);
    }

    fn poll_async(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = self.scrape_rx.take() {
            match rx.try_recv() {
                Ok(ScrapeResult::Full(movies, series)) => {
                    self.db.movies = movies;
                    self.db.series = series;
                    self.db.save(&config::list_path());
                    self.rebuild_genres();
                    self.rebuild_filtered();
                    let n = self.db.movies.len() + self.db.series.len();
                    self.footer_say(&format!(" Scraped {} titles", n), 46);
                    changed = true;
                }
                Ok(ScrapeResult::Progress(msg)) => {
                    self.footer_say(&format!(" {}", msg), 226);
                    self.scrape_rx = Some(rx);
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => { self.scrape_rx = Some(rx); }
                Err(mpsc::TryRecvError::Disconnected) => {}
            }
        }
        if let Some(rx) = self.detail_rx.take() {
            // Drain everything we can without blocking.
            loop {
                match rx.try_recv() {
                    Ok(d) => {
                        // Feed year + genres back into the corresponding
                        // ListItem so filters and sort see the richer data.
                        for list in [&mut self.db.movies, &mut self.db.series] {
                            if let Some(it) = list.iter_mut().find(|it| it.id == d.id) {
                                if it.year == 0 { it.year = d.year; }
                                if it.genres.is_empty() { it.genres = d.genres.clone(); }
                            }
                        }
                        self.details.insert(d.id.clone(), d);
                        changed = true;
                    }
                    Err(mpsc::TryRecvError::Empty) => { self.detail_rx = Some(rx); break; }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        data::save_details_cache(&config::details_path(), &self.details);
                        self.db.save(&config::list_path());
                        self.rebuild_genres();
                        self.rebuild_filtered();
                        self.footer_say(" Details fetched", 46);
                        break;
                    }
                }
            }
        }
        changed
    }

    // --- Search ---

    fn begin_search(&mut self) {
        self.search_mode = true;
        self.search_buf.clear();
        self.search_results.clear();
        self.search_idx = 0;
    }
    fn handle_search_key(&mut self, key: &str) {
        match key {
            "ESC" => { self.search_mode = false; }
            "ENTER" => {
                if self.search_results.is_empty() {
                    self.search_results = scrape::search(&self.search_buf, 10);
                } else if let Some(hit) = self.search_results.get(self.search_idx).cloned() {
                    // Add to current view's list if not present.
                    let src = if self.cfg.view == "movies" { &mut self.db.movies } else { &mut self.db.series };
                    if !src.iter().any(|it| it.id == hit.id) {
                        src.push(hit.clone());
                        self.db.save(&config::list_path());
                        self.rebuild_filtered();
                    }
                    self.search_mode = false;
                }
            }
            "TAB" | "DOWN" => {
                if !self.search_results.is_empty() {
                    self.search_idx = (self.search_idx + 1) % self.search_results.len();
                }
            }
            "S-TAB" | "BACKTAB" | "UP" => {
                if !self.search_results.is_empty() {
                    if self.search_idx == 0 { self.search_idx = self.search_results.len() - 1; }
                    else { self.search_idx -= 1; }
                }
            }
            "BACKSPACE" => { self.search_buf.pop(); self.search_results.clear(); }
            k if k.len() == 1 => { self.search_buf.push_str(k); self.search_results.clear(); }
            _ => {}
        }
        // Render search overlay in detail pane.
        let mut lines = vec![
            style::bold(&style::fg(&format!("Search: {}_", self.search_buf), 226)),
            String::new(),
        ];
        if self.search_results.is_empty() {
            lines.push(style::fg("(Enter to search, ESC to cancel)", 245));
        } else {
            for (i, r) in self.search_results.iter().enumerate() {
                let marker = if i == self.search_idx { "→ " } else { "  " };
                let year = if r.year > 0 { format!(" ({})", r.year) } else { String::new() };
                lines.push(format!("{}{}{}", marker, r.title, year));
            }
        }
        self.detail.set_text(&lines.join("\n"));
        self.detail.full_refresh();
    }

    fn show_help(&mut self) {
        let help = "\n \
            watchit — IMDb Top 250 browser\n\n \
            KEYS\n \
              TAB / S-TAB    Switch focus between panes\n \
              j/k  UP/DOWN   Move within the focused pane\n \
              PgUP / PgDOWN  Page\n \
              +              Wish list (list) / Include genre (genres)\n \
              -              Dump (list) / Exclude genre / Remove (wish+dump)\n \
              Space          Clear genre filter on highlighted genre\n \
              l              Toggle Movies/Series view\n \
              o              Toggle sort (rating / alphabetical)\n \
              r              Set minimum rating\n \
              y / Y          Set min / max year\n \
              /              Search IMDb for new titles\n \
              I              Full scrape of Top 250 (background)\n \
              i              Incremental fetch of missing details\n \
              f              Re-fetch current item\n \
              v              Verify data (fetch first 20 missing)\n \
              L              Load additional IMDb lists (popular + trending)\n \
              D              Remove duplicate entries\n \
              k              Set TMDb API key\n \
              R              Set streaming region\n \
              W              Save config now\n \
              ? / q          Help / Quit\n";
        self.detail.set_text(help);
        self.detail.full_refresh();
        let _ = Input::getchr(None);
        self.render_detail();
    }
}

fn move_bounded(idx: &mut usize, n: i32, total: usize) {
    if total == 0 { *idx = 0; return; }
    let new = (*idx as i32 + n).clamp(0, total as i32 - 1);
    *idx = new as usize;
}
