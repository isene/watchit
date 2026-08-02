mod claude;
mod config;
mod data;
mod import;
mod ratings;
mod scrape;

use config::Config;
use crust::{Crust, Input, Pane, Popup};
use crust::style;
use data::{Database, Details, DetailsCache, ListItem};
use std::collections::HashSet;
use std::sync::mpsc;

fn main() {
    // --help and --version answer before the TUI touches the terminal.
    // A tool that asks what this is — the fe2o3 launcher's ? popup, a
    // packaging script, a curious shell — should get an answer, not a
    // screen paint.
    if std::env::args().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("watchit — Movie and series browser (Fe2O3 suite)");
        println!();
        println!("Usage: watchit");
        println!();
        println!("IMDb Top 250 with inline posters, TMDb streaming info, wish and dump");
        println!("lists, genre filters, search-to-add. Data in ~/.watchit/.");
        return;
    }
    if std::env::args().skip(1).any(|a| a == "-v" || a == "--version") {
        println!("watchit {}", env!("CARGO_PKG_VERSION"));
        return;
    }

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
    Crust::set_app_identity("Watchit");
    let mut app = App::new(cfg);
    app.load_all();

    if let Some(msg) = import_msg {
        app.footer_say(&format!(" {}", msg), 46);
    } else if app.db.movies.is_empty() && app.db.series.is_empty() {
        if app.cfg.tmdb_key.is_empty() {
            app.footer_say(" No data + no TMDB key: press K to set the key (free at themoviedb.org/settings/api), then I to fetch", 196);
        } else {
            app.footer_say(" No data: press I to fetch top-rated lists from TMDB (takes ~10s)", 226);
        }
    }

    app.render_all();

    loop {
        let Some(key) = Input::getchr(Some(1)) else {
            if app.poll_async() { app.render_all(); }
            continue;
        };
        if app.poll_async() { app.render_all(); }

        // A status line reports on the key you just pressed; by the time
        // you press the next one it is stale, and it was sitting on top
        // of the prompt line every prompt uses.
        if app.status_msg.is_some() {
            app.status_msg = None;
            app.render_footer();
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
            // Rate the highlighted title outright: a digit IS the score,
            // 0 stands for 10 (no other key gives a one-press rating).
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                app.rate(key.parse::<u8>().unwrap_or(0));
                app.render_all();
            }
            "0" => { app.rate(10); app.render_all(); }
            "DEL" | "BACK" => { app.rate(0); app.render_all(); }
            "c" => { app.recommend(); }
            "C" => { app.discuss(); }
            "r" => { app.set_rating_min(); app.render_all(); }
            "y" => { app.set_year_min(); app.render_all(); }
            "Y" => { app.set_year_max(); app.render_all(); }
            "/" => { app.search_titles(); }
            "m" => { app.toggle_only_rated(); app.render_all(); }
            "I" => { app.start_full_scrape(); app.render_all(); }
            "i" => { app.start_incremental(); app.render_all(); }
            "f" => { app.refetch_current(); app.render_all(); }
            "K" => { app.set_tmdb_key(); app.render_all(); }
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

    /// Show only what I have rated, highest first. A session toggle —
    /// it is a way of looking at the list, not a saved preference.
    only_rated: bool,
    /// First visible row of the list pane. Kept between renders (rather
    /// than recomputed from the cursor) so the view scrolls the way
    /// pointer's does instead of jumping to re-centre.
    list_scroll: usize,

    // Async: background scrape/fetch tasks.
    scrape_rx: Option<mpsc::Receiver<ScrapeResult>>,
    detail_rx: Option<mpsc::Receiver<Details>>,

    status_msg: Option<(String, u8)>,

    image_display: Option<glow::Display>,
    current_poster: Option<String>,

    /// Adjacent poster prefetch state. Keeps us from spawning a new thread
    /// when one is already running for the same neighborhood.
    prefetch_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Poster downloads still in flight. When they finish and the user is
    /// still looking at that id, we trigger a re-render.
    poster_rx: Option<mpsc::Receiver<String>>,

    /// My own 1-10 scores, merged across every device that writes into
    /// the shared sync folder.
    ratings: ratings::Ratings,
    /// A `claude -p` recommendation in flight. The TUI stays live while
    /// it runs; the answer opens in a popup when it lands.
    recs_rx: Option<mpsc::Receiver<Result<String, String>>>,
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
            only_rated: false,
            list_scroll: 0,
            scrape_rx: None,
            detail_rx: None,
            status_msg: None,
            image_display: None,
            current_poster: None,
            prefetch_busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            poster_rx: None,
            ratings: ratings::Ratings::default(),
            recs_rx: None,
        }
    }

    fn build_panes(cols: u16, rows: u16)
        -> (Pane, Pane, Pane, Pane, Pane, Pane, Pane)
    {
        let mut header = Pane::new(1, 1, cols, 1, 255, 236);
        header.wrap = false;
        // Leave 4 rows outside the main content area: 1 header + 1 border top
        // + 1 border bottom + 1 footer. Panes at y=3 with h=rows-4.
        let content_h = rows.saturating_sub(4);
        let mut list = Pane::new(2, 3, 50, content_h, 252, 0);
        list.wrap = false;
        list.border = true; // active on startup
        list.border_fg = Some(81);
        let mut genres = Pane::new(53, 3, 16, content_h, 248, 232);
        genres.wrap = false;
        genres.border_fg = Some(81);
        let wish_h = rows / 2 - 1;
        let mut wish = Pane::new(70, 3, 30, wish_h, 64, 232);
        wish.wrap = false;
        wish.border_fg = Some(81);
        let dump_y = 3 + wish_h + 1;
        let mut dump = Pane::new(70, dump_y, 30, content_h.saturating_sub(wish_h + 1), 130, 232);
        dump.wrap = false;
        dump.border_fg = Some(81);
        let detail_x = 102;
        let mut detail = Pane::new(detail_x, 3, cols.saturating_sub(detail_x), content_h, 255, 0);
        // Wrap is fine — crust now handles OSC 8 across visual wraps.
        detail.wrap = true;
        let mut footer = Pane::new(1, rows, cols, 1, 255, 236);
        footer.wrap = false;
        (header, list, genres, wish, dump, detail, footer)
    }

    /// Set border only on the currently focused pane (IMDB pattern).
    /// Explicitly erases the border glyphs on newly-unfocused panes so the
    /// old frame doesn't linger on screen.
    fn apply_focus_border(&mut self) {
        // Clear any pane that currently has a border but shouldn't.
        if self.list.border && self.focus != Focus::List { self.list.border_clear(); self.list.border = false; }
        if self.genres.border && self.focus != Focus::Genres { self.genres.border_clear(); self.genres.border = false; }
        if self.wish.border && self.focus != Focus::Wish { self.wish.border_clear(); self.wish.border = false; }
        if self.dump.border && self.focus != Focus::Dump { self.dump.border_clear(); self.dump.border = false; }
        // Enable on the new focus.
        match self.focus {
            Focus::List => self.list.border = true,
            Focus::Genres => self.genres.border = true,
            Focus::Wish => self.wish.border = true,
            Focus::Dump => self.dump.border = true,
        }
    }

    fn load_all(&mut self) {
        self.db = Database::load(&config::list_path());
        self.details = data::load_details_cache(&config::details_path());
        self.drop_mismatched_details();
        self.ratings = ratings::Ratings::load(&config::sync_dir());
        self.rebuild_genres();
        self.rebuild_filtered();
    }

    /// Throw away any cached details that describe the wrong thing.
    ///
    /// TMDB movie and tv ids are separate spaces, so a details fetch that
    /// probed movie-then-tv could answer for a completely different film
    /// (tv/67683 is Travelers; movie/67683 is a 1969 Soviet comedy). Such
    /// an entry looks perfectly valid — it has a title and no error flag
    /// — so nothing would ever refetch it. Catch it by the one thing that
    /// gives it away: it disagrees with the catalog about what kind of
    /// thing it is.
    fn drop_mismatched_details(&mut self) {
        let mut bad: Vec<String> = Vec::new();
        for (id, d) in &self.details {
            let Some(want) = self.kind_of(id) else { continue };
            let has = if d.kind == "TVSeries" { "tv" } else if d.kind == "Movie" { "movie" } else { continue };
            if has != want { bad.push(id.clone()); }
        }
        if bad.is_empty() { return; }
        for id in &bad { self.details.remove(id); }
        data::save_details_cache(&config::details_path(), &self.details);
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
            .filter(|it| self.only_rated
                || it.rating >= self.cfg.rating_min)
            .filter(|it| !self.only_rated
                || self.ratings.get(&it.id, &it.title, self.item_year(it)).is_some())
            .filter(|it| self.cfg.year_min == 0 || self.item_year(it) >= self.cfg.year_min)
            .filter(|it| self.cfg.year_max == 0 || self.item_year(it) <= self.cfg.year_max)
            .filter(|it| self.matches_genres(it))
            .map(|it| (it.id.clone(), it.rating, it.title.clone()))
            .collect();
        match self.cfg.sort.as_str() {
            "alpha" => ids.sort_by(|a, b| a.2.to_lowercase().cmp(&b.2.to_lowercase())),
            // My own score first, unrated last, TMDB rating breaking ties.
            "mine" => ids.sort_by(|a, b| {
                let (ma, mb) = (self.ratings.get(&a.0, &a.2, 0).unwrap_or(0),
                                self.ratings.get(&b.0, &b.2, 0).unwrap_or(0));
                mb.cmp(&ma).then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
            }),
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
        let scope = if self.only_rated { "  \u{2605} rated only" } else { "" };
        let text = format!(" watchit  [{}{}]  {}  •  {}", view, scope, filter, counts);
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
            let focused = i == self.list_idx && self.focus == Focus::List;
            // Underline only the title (+ year), not the rating or the ">" marker.
            let title_part = format!("{}{}", title, year_s);
            let title_styled = if focused {
                style::bold(&style::underline(&title_part))
            } else {
                title_part
            };
            let marker = if focused { "\u{2192} " } else { "  " };
            // My score next to the crowd's, padded to a fixed 3 columns
            // BEFORE colouring — SGR bytes would break the alignment.
            let mine = match self.ratings.get(id, &title, year) {
                Some(s) => style::fg(&format!("{:>3}", format!("\u{2605}{}", s)), 214),
                None => "   ".to_string(),
            };
            lines.push(format!("{}{:>4.1} {} {}", marker, rating, mine, title_styled));
        }
        self.list.set_text(&lines.join("\n"));
        self.scroll_list();
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
            // Underline only the name itself, not the leading marker.
            let name = if focused { style::underline(&style::bold(g)) } else { g.clone() };
            lines.push(format!("{}{}", mark, name));
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
            let name = if focused { style::bold(&style::underline(&title)) } else { title };
            let marker = if focused { "\u{2192} " } else { "  " };
            lines.push(format!("{}{}", marker, name));
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
            let name = if focused { style::bold(&style::underline(&title)) } else { title };
            let marker = if focused { "\u{2192} " } else { "  " };
            lines.push(format!("{}{}", marker, name));
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
        // OSC 8 hyperlinks: TMDB is the data source so always present;
        // IMDB is shown alongside when TMDB has the external id on file
        // (Details.imdb_id from external_ids.imdb_id in fetch_details).
        // Clickable in kitty/foot/wezterm/iTerm2.
        // Which endpoint the link points at: the details cache first, then
        // the catalog row, then which list it lives in — a legacy row has
        // no kind, and a series linked as /movie/<id> goes to a different
        // film entirely (the ids are separate spaces).
        let is_tv = det.as_ref().map(|d| d.kind == "TVSeries").unwrap_or(false)
            || item.as_ref().map(|it| it.kind == "tv").unwrap_or(false)
            || self.db.series.iter().any(|it| it.id == id);
        let tmdb_path = if is_tv { "tv" } else { "movie" };
        let tmdb_url = format!("https://www.themoviedb.org/{}/{}", tmdb_path, id);
        let tmdb_link = style::hyperlink(
            &tmdb_url,
            &format!("themoviedb.org/{}/{}", tmdb_path, id),
        );
        let mut link_line = style::fg(&style::underline(&tmdb_link), 240);
        if let Some(tconst) = det.as_ref().map(|d| d.imdb_id.clone()).filter(|s| !s.is_empty()) {
            let imdb_url = format!("https://www.imdb.com/title/{}/", tconst);
            let imdb_link = style::hyperlink(
                &imdb_url,
                &format!("imdb.com/title/{}/", tconst),
            );
            link_line.push_str(&style::fg("  ·  ", 240));
            link_line.push_str(&style::fg(&style::underline(&imdb_link), 240));
        }
        lines.push(link_line);
        lines.push(String::new());

        // My own score sits above TMDB's, and says so when it is missing
        // — an empty line here would read as "rated 0".
        let seen_year = self.seen_of(&id, None).map(|s| s.year).unwrap_or(0);
        let mine = match self.ratings.get(&id, &title, seen_year) {
            Some(s) => style::bold(&style::fg(&format!("My rating: {}/10", s), 214)),
            None => style::fg("My rating: – (press 1-9, 0 for 10)", 240),
        };
        lines.push(mine);

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
            // Background-prefetch posters for nearby list entries so
            // cursor navigation feels snappy.
            self.prefetch_adjacent_posters();
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
        // Cap it. Unbounded, the box was the whole detail pane, and on a
        // tall terminal that is a poster the size of the screen with the
        // plot squeezed above it. A poster reads fine at postcard size.
        let img_w = self.detail.w.saturating_sub(2).min(34);
        let img_h = self.detail.h.saturating_sub(top + 1).min(24);
        if img_h < 4 { return; }
        self.image_display = Some(display);
        if let Some(ref mut disp) = self.image_display {
            disp.show(path.to_string_lossy().as_ref(), img_x, img_y, img_w, img_h);
        }
        self.current_poster = Some(id.to_string());
    }

    fn show_poster(&mut self, id: &str, url: &str) {
        // Disk cache at ~/.watchit/data/<id>.jpg; if present, show instantly.
        // If missing, spawn background download and clear the poster area so
        // the UI stays responsive. Poll in poll_async() will render once it
        // lands if the user is still on the same item.
        let path = config::data_dir().join(format!("{}.jpg", id));
        if path.exists() {
            self.show_poster_path(id, &path);
            return;
        }
        self.clear_poster();
        self.spawn_poster_download(id.to_string(), url.to_string(), path);
    }

    fn spawn_poster_download(&mut self, id: String, url: String, path: std::path::PathBuf) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(5))
                .timeout_read(std::time::Duration::from_secs(20))
                .redirects(5)
                .build();
            if let Ok(resp) = agent.get(&url).call() {
                let mut bytes = Vec::new();
                if std::io::Read::read_to_end(&mut resp.into_reader(), &mut bytes).is_ok()
                    && bytes.len() > 100
                {
                    let _ = std::fs::write(&path, &bytes);
                }
            }
            let _ = tx.send(id);
        });
        self.poster_rx = Some(rx);
    }

    /// Prefetch poster JPGs for adjacent items in the filtered list so that
    /// when the user moves the cursor the next few rows already have their
    /// posters on disk. Non-blocking; skips entries that already have a
    /// cached JPG and coalesces overlapping prefetch waves.
    fn prefetch_adjacent_posters(&mut self) {
        use std::sync::atomic::Ordering;
        if self.prefetch_busy.load(Ordering::Relaxed) { return; }

        // Build the job list on the main thread (no borrow of self from the
        // worker).
        let mut jobs: Vec<(String, String, std::path::PathBuf)> = Vec::new();
        let offsets = [1i32, 2, 3, -1, -2];
        let len = self.filtered.len() as i32;
        for off in offsets {
            let idx = self.list_idx as i32 + off;
            if idx < 0 || idx >= len { continue; }
            let id = self.filtered[idx as usize].clone();
            let path = config::data_dir().join(format!("{}.jpg", id));
            if path.exists() { continue; }
            let url = self.details.get(&id)
                .map(|d| d.poster_url.clone())
                .unwrap_or_default();
            if !url.is_empty() { jobs.push((id, url, path)); }
        }
        if jobs.is_empty() { return; }

        let busy = self.prefetch_busy.clone();
        busy.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(5))
                .timeout_read(std::time::Duration::from_secs(20))
                .redirects(5)
                .build();
            for (_id, url, path) in jobs {
                if path.exists() { continue; }
                if let Ok(resp) = agent.get(&url).call() {
                    let mut bytes = Vec::new();
                    if std::io::Read::read_to_end(&mut resp.into_reader(), &mut bytes).is_ok()
                        && bytes.len() > 100
                    {
                        let _ = std::fs::write(&path, &bytes);
                    }
                }
            }
            busy.store(false, Ordering::Relaxed);
        });
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
            let hint = " ?:Help  TAB:Focus  j/k:Move  1-9/0:Rate  m:Mine  +/-:Wish/Dump  /:Search  c/C:Claude  l:Movies/Series  o:Sort  q:Quit";
            self.footer.say(&style::fg(hint, 245));
        }
    }

    fn footer_say(&mut self, msg: &str, color: u8) {
        self.status_msg = Some((msg.to_string(), color));
        self.render_footer();
    }

    /// Scrolloff of 3, the way pointer does it: the cursor never comes
    /// closer than three rows to either edge, and the offset persists
    /// between renders so the view slides instead of jumping.
    ///
    /// The `+ HEAD` matters. The pane's title and its blank line are text
    /// rows like any other, so the cursor sits two rows lower than its
    /// index — without accounting for that, the last two titles lived
    /// below the fold and the pane simply refused to scroll to them.
    fn scroll_list(&mut self) {
        const HEAD: usize = 2;
        const OFF: usize = 3;
        let h = self.list.h as usize;
        let total = self.filtered.len() + HEAD;
        let cursor = self.list_idx + HEAD;
        if total <= h {
            self.list_scroll = 0;
        } else if cursor < self.list_scroll + OFF {
            self.list_scroll = cursor.saturating_sub(OFF);
        } else if cursor + OFF >= self.list_scroll + h {
            let max_ix = total.saturating_sub(h);
            self.list_scroll = (cursor + OFF + 1).saturating_sub(h).min(max_ix);
        }
        self.list.ix = self.list_scroll;
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
        self.apply_focus_border();
    }
    fn prev_focus(&mut self) {
        self.focus = match self.focus {
            Focus::List => Focus::Dump,
            Focus::Genres => Focus::List,
            Focus::Wish => Focus::Genres,
            Focus::Dump => Focus::Wish,
        };
        self.apply_focus_border();
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
        self.cfg.sort = match self.cfg.sort.as_str() {
            "rating" => "alpha".into(),
            "alpha" => "mine".into(),
            _ => "rating".into(),
        };
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
            if needs { missing.push((it.id.clone(), self.kind_of(&it.id))); }
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
            for (id, kind) in missing {
                let d = scrape::fetch_details_keyed(&id, kind.as_deref(), &region, &key);
                let _ = tx.send(d);
            }
        });
        self.detail_rx = Some(rx);
    }

    /// Load additional TMDB lists ("popular" movies + TV)
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
        let key = self.cfg.tmdb_key.clone();
        let movie_limit = self.cfg.movie_limit;
        let series_limit = self.cfg.series_limit;
        std::thread::spawn(move || {
            let _ = tx.send(ScrapeResult::Progress("Popular movies...".into()));
            for it in scrape::scrape_chart_keyed("chart/moviemeter", movie_limit, &key) {
                if !existing_movies.contains(&it.id) { new_movies.push(it); }
            }
            let _ = tx.send(ScrapeResult::Progress("Popular series...".into()));
            for it in scrape::scrape_chart_keyed("chart/tvmeter", series_limit, &key) {
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
        if self.cfg.tmdb_key.is_empty() {
            self.footer_say(" No TMDB API key — set tmdb_key in ~/.watchit/config.yml (free key at themoviedb.org/settings/api)", 196);
            return;
        }
        self.footer_say(" Fetching TMDB top-rated lists...", 226);
        self.render_footer();
        let (tx, rx) = mpsc::channel();
        let movie_limit = self.cfg.movie_limit;
        let series_limit = self.cfg.series_limit;
        let key = self.cfg.tmdb_key.clone();
        std::thread::spawn(move || {
            let key1 = key.clone();
            let _ = tx.send(ScrapeResult::Progress("Fetching movies...".into()));
            let movies = scrape::scrape_chart_keyed("chart/top", movie_limit, &key1);
            let _ = tx.send(ScrapeResult::Progress("Fetching series...".into()));
            let series = scrape::scrape_chart_keyed("chart/toptv", series_limit, &key);
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
        let missing: Vec<(String, Option<String>)> = self.filtered.iter()
            .filter(|id| self.details.get(id.as_str()).map(|d| d.error || d.title.is_empty()).unwrap_or(true))
            .take(5)
            .map(|id| (id.clone(), self.kind_of(id)))
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
            for (id, kind) in missing {
                let d = scrape::fetch_details_keyed(&id, kind.as_deref(), &region, &key);
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
        let kind = self.kind_of(&id);
        let key = self.cfg.tmdb_key.clone();
        let region = self.cfg.region.clone();
        std::thread::spawn(move || {
            let d = scrape::fetch_details_keyed(&id_clone, kind.as_deref(), &region, &key);
            let _ = tx.send(d);
        });
        self.detail_rx = Some(rx);
    }

    fn poll_async(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = self.recs_rx.take() {
            match rx.try_recv() {
                Ok(Ok(text)) => {
                    self.offer_recs(&text);
                    changed = true;
                }
                Ok(Err(e)) => {
                    self.footer_say(&format!(" Claude: {}", e), 196);
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => { self.recs_rx = Some(rx); }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.footer_say(" Claude call died", 196);
                    changed = true;
                }
            }
        }
        if let Some(rx) = self.poster_rx.take() {
            match rx.try_recv() {
                Ok(finished_id) => {
                    // If user is still viewing this id, show the new poster.
                    if self.current_id().as_deref() == Some(finished_id.as_str()) {
                        let path = config::data_dir().join(format!("{}.jpg", finished_id));
                        if path.exists() {
                            self.show_poster_path(&finished_id, &path);
                        }
                    }
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => { self.poster_rx = Some(rx); }
                Err(mpsc::TryRecvError::Disconnected) => {}
            }
        }
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

    /// Search TMDB and pick from the hits.
    ///
    /// This used to draw the query and the results into the detail pane,
    /// which the poster is painted over and which `render_all` rewrites
    /// on the next keypress — so the typing was invisible and the hits
    /// flickered away. Now the query goes in the footer prompt and the
    /// hits get their own picker, with the highlighted one's blurb below
    /// the list: four films called "Travellers" are otherwise impossible
    /// to tell apart.
    fn search_titles(&mut self) {
        if self.cfg.tmdb_key.is_empty() {
            self.footer_say(" Search needs a TMDB key — press K", 196);
            return;
        }
        let query = self.footer.ask(" Search TMDB: ", "");
        let query = query.trim().to_string();
        if query.is_empty() { self.render_footer(); return; }

        self.footer_say(&format!(" Searching TMDB for \u{201c}{}\u{201d}…", query), 81);
        let hits = scrape::search_keyed(&query, 30, &self.cfg.tmdb_key);
        if hits.is_empty() {
            self.footer_say(&format!(" Nothing found for \u{201c}{}\u{201d}", query), 226);
            return;
        }

        let Some(pick) = self.pick_search_hit(&query, &hits) else { return };
        let hit = hits[pick].item.clone();

        // Route by what TMDB says it is, NOT by the view that happens to
        // be open — searching for a series from the movie list used to
        // file it under movies, where it never showed up again.
        let is_series = hit.kind == "tv";
        let known = {
            let list = if is_series { &self.db.series } else { &self.db.movies };
            list.iter().any(|it| it.id == hit.id)
        };
        if known {
            // Go to it. "Already in your list" while the list refuses to
            // show it is not an answer — that is exactly the case where
            // you searched because you could not find it.
            let why = self.reveal(&hit.id);
            self.render_all();
            self.footer_say(&format!(" {} is already in your list{}", hit.title, why), 226);
            return;
        }
        let list = if is_series { &mut self.db.series } else { &mut self.db.movies };
        list.push(hit.clone());
        self.db.save(&config::list_path());
        // Show it, and fetch its details so the pane has something to say.
        let why = self.reveal(&hit.id);
        self.render_all();
        // Pull its details in the background, then say what happened —
        // refetch_current posts its own status, so saying it first would
        // just be overwritten.
        self.refetch_current();
        self.footer_say(&format!(" Added {} to {}{} — fetching details", hit.title,
            if is_series { "Series" } else { "Movies" }, why), 46);
    }

    /// Put the cursor on a title, whatever it takes, and report what had
    /// to give way.
    ///
    /// Switching to its list is not enough: a filter can hide it — 3 Body
    /// Problem sits at 7.485 under a 7.5 minimum — and then the cursor
    /// has nowhere to land. Anything hiding it is relaxed rather than
    /// silently obeyed, because you only ask for a title you want to see.
    fn reveal(&mut self, id: &str) -> String {
        let Some((item, _)) = self.any_lookup(id) else { return String::new() };
        let item = item.clone();
        let is_series = self.db.series.iter().any(|it| it.id == id);
        self.cfg.view = if is_series { "series".into() } else { "movies".into() };
        let mut relaxed: Vec<String> = Vec::new();
        if self.only_rated && self.ratings.get(id, &item.title, self.item_year(&item)).is_none() {
            self.only_rated = false;
            relaxed.push("left the rated-only view".into());
        }
        self.rebuild_filtered();

        if !self.filtered.iter().any(|f| f == id) {
            let year = self.item_year(&item);
            if item.rating < self.cfg.rating_min {
                self.cfg.rating_min = (item.rating * 10.0).floor() / 10.0;
                relaxed.push(format!("dropped the rating filter to {:.1}", self.cfg.rating_min));
            }
            if self.cfg.year_min > 0 && year > 0 && year < self.cfg.year_min {
                self.cfg.year_min = year;
                relaxed.push(format!("dropped the min year to {}", year));
            }
            if self.cfg.year_max > 0 && year > 0 && year > self.cfg.year_max {
                self.cfg.year_max = year;
                relaxed.push(format!("raised the max year to {}", year));
            }
            if !self.matches_genres(&item) {
                self.cfg.genres_include.clear();
                self.cfg.genres_exclude.clear();
                relaxed.push("cleared the genre filter".into());
            }
            let dump = if is_series { &mut self.cfg.dump_series } else { &mut self.cfg.dump_movies };
            if let Some(pos) = dump.iter().position(|d| d == id) {
                dump.remove(pos);
                relaxed.push("took it off your dump list".into());
            }
            self.rebuild_filtered();
        }

        if let Some(pos) = self.filtered.iter().position(|f| f == id) {
            self.list_idx = pos;
            self.focus = Focus::List;
            self.apply_focus_border();
        }
        if relaxed.is_empty() { String::new() } else { format!(" — {}", relaxed.join(", ")) }
    }

    /// A list with the highlighted row's detail underneath, and its own
    /// input loop. `Popup::modal` is one line per item and returns only
    /// the final choice — there is nowhere to hang the panel that makes
    /// the choice possible, and no way to offer more than one action.
    ///
    /// `rows` are redrawn from the closure each pass, so a caller can
    /// mark them as it goes. Returns on any key in `act` with the row it
    /// was pressed on; ESC / q returns None.
    fn pick_from(
        &mut self,
        header: &str,
        rows: &[String],
        infos: &[String],
        act: &[&str],
        start: usize,
    ) -> Option<(usize, String)> {
        if rows.is_empty() { return None; }
        let w = self.cols.saturating_sub(8).min(96).max(46);
        let x = (self.cols.saturating_sub(w)) / 2;
        let info_h = 8u16;
        let list_h = (rows.len() as u16 + 1).min(self.rows.saturating_sub(info_h + 8)).max(4);
        let total_h = list_h + info_h + 3;
        let y = (self.rows.saturating_sub(total_h)) / 2 + 1;

        let mut list = Pane::new(x.max(1), y, w, list_h, 255, 234);
        list.border = true;
        list.border_fg = Some(81);
        list.wrap = false;
        let mut info = Pane::new(x.max(1), y + list_h + 2, w, info_h, 252, 233);
        info.border = true;
        info.border_fg = Some(240);
        info.wrap = true;

        let mut idx = start.min(rows.len() - 1);
        let mut out = None;
        loop {
            let drawn: Vec<String> = rows.iter().enumerate().map(|(i, r)| {
                if i == idx { style::bold(&style::fg(r, 226)) } else { r.clone() }
            }).collect();
            list.set_text(&format!("{}\n{}", style::bold(&style::fg(header, 81)), drawn.join("\n")));
            list.ix = self.compute_scroll(idx + 1, rows.len() + 1, list_h as usize);
            list.full_refresh();
            info.set_text(infos.get(idx).map(String::as_str).unwrap_or(""));
            info.ix = 0;
            info.full_refresh();

            let Some(key) = Input::getchr(None) else { continue };
            if act.contains(&key.as_str()) { out = Some((idx, key)); break; }
            match key.as_str() {
                "ESC" | "q" => break,
                "j" | "DOWN" | "TAB" => { idx = (idx + 1) % rows.len(); }
                "k" | "UP" | "S-TAB" | "BACKTAB" => {
                    idx = if idx == 0 { rows.len() - 1 } else { idx - 1 };
                }
                "PgDOWN" => { idx = (idx + list_h as usize).min(rows.len() - 1); }
                "PgUP" => { idx = idx.saturating_sub(list_h as usize); }
                "HOME" => { idx = 0; }
                "END" => { idx = rows.len() - 1; }
                _ => {}
            }
        }
        out
    }

    fn pick_search_hit(&mut self, query: &str, hits: &[scrape::SearchHit]) -> Option<usize> {
        let rows: Vec<String> = hits.iter().map(|h| {
            let it = &h.item;
            let kind = if it.kind == "tv" { "Series" } else { "Movie " };
            let year = if it.year > 0 { it.year.to_string() } else { "----".into() };
            let have = if self.any_lookup(&it.id).is_some() { "\u{2713}" } else { " " };
            format!("{} {}  {}  {:>4.1}  {}", have, kind, year, it.rating, it.title)
        }).collect();
        let infos: Vec<String> = hits.iter().map(|h| {
            let year = if h.item.year > 0 { h.item.year.to_string() } else { "year unknown".into() };
            let kind = if h.item.kind == "tv" { "Series" } else { "Movie" };
            let blurb = if h.overview.trim().is_empty() {
                style::fg("(TMDB has no description for this one)", 245)
            } else {
                h.overview.clone()
            };
            format!("{}\n{}", style::bold(&style::fg(
                &format!("{} \u{b7} {} \u{b7} {} \u{b7} TMDB {:.1}",
                    h.item.title, year, kind, h.item.rating), 226)), blurb)
        }).collect();
        let header = format!("Search: {}   ({} hits)   ENTER adds \u{b7} ESC cancels", query, hits.len());
        self.clear_poster();
        let pick = self.pick_from(&header, &rows, &infos, &["ENTER"], 0).map(|(i, _)| i);
        self.repaint_screen();
        pick
    }

    /// Offer Claude's recommendations as something to act on, not just
    /// read. ENTER marks a title for the list, `w` marks it for the list
    /// AND the wish list; ESC applies whatever is marked.
    ///
    /// Nothing is fetched while marking. Each pick costs a TMDB lookup to
    /// turn a title into a real catalog row, and doing that per keypress
    /// would make the picker crawl for titles you then unmark.
    fn offer_recs(&mut self, text: &str) {
        let recs = claude::parse_recs(text);
        if recs.is_empty() {
            // The model ignored the format. Better to read it than to
            // pretend there is nothing there.
            let head = style::bold(&style::fg("What to watch next", 226));
            self.show_text(&format!("{}\n\n{}", head, text));
            return;
        }

        const NONE: u8 = 0;
        const LIST: u8 = 1;
        const WISH: u8 = 2;
        let mut marks: Vec<u8> = vec![NONE; recs.len()];
        let mut idx = 0usize;
        let scope = claude::genre_label(&self.cfg.genres_include, &self.cfg.genres_exclude);
        let header = format!(
            "What to watch next{}{}   ENTER: add \u{b7} w: add + wish \u{b7} ESC: done",
            if scope.is_empty() { "" } else { " \u{2014} " }, scope);

        self.clear_poster();
        loop {
            let rows: Vec<String> = recs.iter().zip(marks.iter()).map(|(r, m)| {
                let mark = match *m {
                    LIST => style::fg("[+]", 46),
                    WISH => style::fg("[\u{2665}]", 82),
                    _ => "[ ]".to_string(),
                };
                let kind = if r.kind == "tv" { "Series" } else { "Movie " };
                let year = if r.year > 0 { r.year.to_string() } else { "----".into() };
                let have = if self.find_by_title(&r.title, r.year).is_some() { "\u{2713}" } else { " " };
                format!("{} {} {}  {}  {}", mark, have, kind, year, r.title)
            }).collect();
            let infos: Vec<String> = recs.iter().map(|r| {
                let year = if r.year > 0 { r.year.to_string() } else { "year unknown".into() };
                let kind = if r.kind == "tv" { "Series" } else { "Movie" };
                let known = match self.find_by_title(&r.title, r.year) {
                    Some(_) => style::fg("\n\nAlready in your list.", 245),
                    None => String::new(),
                };
                format!("{}\n{}{}", style::bold(&style::fg(
                    &format!("{} \u{b7} {} \u{b7} {}", r.title, year, kind), 226)), r.why, known)
            }).collect();

            match self.pick_from(&header, &rows, &infos, &["ENTER", "w"], idx) {
                Some((i, key)) => {
                    idx = i;
                    let want = if key == "w" { WISH } else { LIST };
                    marks[i] = if marks[i] == want { NONE } else { want };
                }
                None => break,
            }
        }
        self.repaint_screen();

        let picked: Vec<(usize, u8)> = marks.iter().enumerate()
            .filter(|(_, m)| **m != NONE).map(|(i, m)| (i, *m)).collect();
        if picked.is_empty() {
            self.footer_say(" Nothing added", 245);
            return;
        }
        let (mut added, mut wished, mut missed) = (0usize, 0usize, 0usize);
        for (i, mark) in picked {
            let rec = &recs[i];
            self.footer_say(&format!(" Looking up {}…", rec.title), 81);
            let Some(item) = self.resolve_rec(rec) else { missed += 1; continue };
            let is_series = item.kind == "tv";
            let list = if is_series { &mut self.db.series } else { &mut self.db.movies };
            if !list.iter().any(|it| it.id == item.id) {
                list.push(item.clone());
                added += 1;
            }
            if mark == WISH {
                let wish = if is_series { &mut self.cfg.wish_series } else { &mut self.cfg.wish_movies };
                if !wish.contains(&item.id) { wish.push(item.id.clone()); wished += 1; }
            }
        }
        self.db.save(&config::list_path());
        self.cfg.save();
        self.rebuild_genres();
        self.rebuild_filtered();
        self.render_all();
        let mut msg = format!(" Added {} to the list", added);
        if wished > 0 { msg.push_str(&format!(", {} to the wish list", wished)); }
        if missed > 0 { msg.push_str(&format!(" \u{b7} {} not found on TMDB", missed)); }
        self.footer_say(&msg, 46);
    }

    /// Is this title already in the catalog? Recommendations come back as
    /// text, so a title match is all there is to go on until it has been
    /// looked up.
    fn find_by_title(&self, title: &str, year: i32) -> Option<&ListItem> {
        let norm = |s: &str| -> String {
            s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
        };
        let want = norm(title);
        self.db.movies.iter().chain(self.db.series.iter())
            .find(|it| norm(&it.title) == want && (year == 0 || it.year == 0 || (it.year - year).abs() <= 1))
    }

    /// Turn a recommended title into a real catalog row via TMDB. Prefers
    /// a hit of the right kind whose year matches; a recommendation is a
    /// title and a year, and TMDB will happily return a remake.
    fn resolve_rec(&self, rec: &claude::Rec) -> Option<ListItem> {
        let hits = scrape::search_keyed(&rec.title, 10, &self.cfg.tmdb_key);
        if hits.is_empty() { return None; }
        let same_kind: Vec<&scrape::SearchHit> =
            hits.iter().filter(|h| h.item.kind == rec.kind).collect();
        let pool = if same_kind.is_empty() { hits.iter().collect::<Vec<_>>() } else { same_kind };
        let exact = pool.iter().find(|h| rec.year > 0 && h.item.year == rec.year);
        let close = pool.iter().find(|h| rec.year > 0 && (h.item.year - rec.year).abs() <= 1);
        Some(exact.or(close).unwrap_or(&pool[0]).item.clone())
    }

    /// Show only the titles I have scored, best first. Turning it on
    /// switches the sort too — a rated-only list ordered by TMDB's score
    /// would bury my own ranking.
    fn toggle_only_rated(&mut self) {
        self.only_rated = !self.only_rated;
        if self.only_rated {
            self.cfg.sort = "mine".into();
        }
        self.list_idx = 0;
        self.rebuild_filtered();
        if self.only_rated {
            self.footer_say(&format!(" My ratings only — {} titles", self.filtered.len()), 214);
        } else {
            self.footer_say(" Showing everything again", 245);
        }
    }

    // --- My ratings ---

    /// Which TMDB endpoint owns an id, when we know. Movie and tv ids are
    /// SEPARATE spaces — tv/67683 is Travelers, movie/67683 is a 1969
    /// Soviet comedy — so a details fetch that probes movie-then-tv can
    /// come back with an unrelated film. Older catalog rows have no kind;
    /// those still have to probe.
    fn kind_of(&self, id: &str) -> Option<String> {
        if let Some((it, _)) = self.any_lookup(id) {
            if !it.kind.is_empty() { return Some(it.kind.clone()); }
        }
        if self.db.series.iter().any(|it| it.id == id) { return Some("tv".into()); }
        if self.db.movies.iter().any(|it| it.id == id) { return Some("movie".into()); }
        None
    }

    /// Look a title up across BOTH lists. The rated set spans movies and
    /// series, so the view-scoped `list_lookup` would lose half of it.
    fn any_lookup(&self, id: &str) -> Option<(&ListItem, &'static str)> {
        self.db.movies.iter().find(|it| it.id == id).map(|it| (it, "Movie"))
            .or_else(|| self.db.series.iter().find(|it| it.id == id).map(|it| (it, "Series")))
    }

    fn seen_of(&self, id: &str, score: Option<u8>) -> Option<claude::Seen> {
        if let Some((it, kind)) = self.any_lookup(id) {
            return Some(claude::Seen {
                title: it.title.clone(), year: self.item_year(it),
                kind: kind.to_string(), score,
            });
        }
        // Dropped from the catalog but still rated / wished: the details
        // cache remembers it.
        self.details.get(id).map(|d| claude::Seen {
            title: d.title.clone(), year: d.year,
            kind: if d.kind == "TVSeries" { "Series".into() } else { "Movie".into() },
            score,
        })
    }

    /// Score the highlighted title 1-10, or clear it with 0. Works from
    /// the list, the wish pane and the dump pane alike.
    fn rate(&mut self, score: u8) {
        let Some(id) = self.current_id() else { return };
        let seen = self.seen_of(&id, None);
        let title = seen.as_ref().map(|s| s.title.clone()).unwrap_or_else(|| id.clone());
        let year = seen.as_ref().map(|s| s.year).unwrap_or(0);
        self.ratings.set(&id, &title, year, score);
        if score == 0 {
            self.footer_say(&format!(" Cleared my rating for {}", title), 244);
        } else {
            self.footer_say(&format!(" {} — my rating: {}/10", title, score), 46);
        }
        if self.cfg.sort == "mine" { self.rebuild_filtered(); }
    }

    // --- Claude ---

    /// Everything Claude needs to know about my taste: what I scored,
    /// what I already want, what I threw out.
    fn taste(&self) -> String {
        // Fall back to the title stored in the rating itself: a rating
        // that came from the phone carries an id this catalog never had.
        let rated: Vec<claude::Seen> = self.ratings.rated().into_iter()
            .map(|(id, e)| self.seen_of(&id, Some(e.score)).unwrap_or(claude::Seen {
                title: e.title.clone(), year: e.year,
                kind: "Movie or series".into(), score: Some(e.score),
            }))
            .collect();
        let ids = |v: &Vec<String>| -> Vec<claude::Seen> {
            v.iter().filter_map(|id| {
                let seen = self.seen_of(id, None)?;
                let score = self.ratings.get(id, &seen.title, seen.year);
                Some(claude::Seen { score, ..seen })
            }).collect()
        };
        let mut wish = ids(&self.cfg.wish_movies);
        wish.extend(ids(&self.cfg.wish_series));
        let mut dump = ids(&self.cfg.dump_movies);
        dump.extend(ids(&self.cfg.dump_series));
        // The dump list is a bin, not a profile — a few dozen is plenty
        // of negative signal without swamping the prompt.
        dump.truncate(60);
        claude::taste(&rated, &wish, &dump)
    }

    /// One-shot recommendations. Runs off the UI thread so the TUI stays
    /// live for the ~20 s the model takes; the answer opens in a popup.
    fn recommend(&mut self) {
        if self.recs_rx.is_some() {
            self.footer_say(" Already asking Claude — hang on", 226);
            return;
        }
        if !claude::available() {
            self.footer_say(" Recommendations need the `claude` CLI on PATH", 196);
            return;
        }
        if self.ratings.count() == 0 {
            self.footer_say(" Rate a few titles first (1-9, 0 = 10)", 226);
            return;
        }
        let want = self.footer.ask(" In the mood for (blank = anything): ", "");
        // The genre filter scopes the recommendations too — the list on
        // screen obeys it, so the suggestions have to as well.
        let genres = format!("{}{}",
            claude::view_rule(&self.cfg.view),
            claude::genre_rule(&self.cfg.genres_include, &self.cfg.genres_exclude));
        let prompt = claude::recommend_prompt(&self.taste(), 8, &want, &genres);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || { let _ = tx.send(claude::ask(&prompt)); });
        self.recs_rx = Some(rx);
        let scope = claude::genre_label(&self.cfg.genres_include, &self.cfg.genres_exclude);
        if scope.is_empty() {
            self.footer_say(" Asking Claude for recommendations…", 81);
        } else {
            self.footer_say(&format!(" Asking Claude for recommendations — {}", scope), 81);
        }
    }

    /// Hand the terminal to an interactive `claude`, seeded with my
    /// taste, so recommendations can be argued about. `/exit` there
    /// comes back here.
    fn discuss(&mut self) {
        if !claude::available() {
            self.footer_say(" Discussion needs the `claude` CLI on PATH", 196);
            return;
        }
        let genres = format!("{}{}",
            claude::view_rule(&self.cfg.view),
            claude::genre_rule(&self.cfg.genres_include, &self.cfg.genres_exclude));
        let prompt = claude::discuss_prompt(&self.taste(), &genres);
        self.clear_poster();
        // Give claude a clean terminal: same handshake amar and kastrup use.
        use std::io::Write as _;
        Crust::disable_bracketed_paste();
        let _ = std::io::stdout().flush();
        Crust::cleanup();
        Crust::clear_screen();

        let status = std::process::Command::new("claude").arg(&prompt).status();

        Crust::init();
        Crust::enable_bracketed_paste();
        let _ = std::io::stdout().flush();
        Crust::clear_screen();
        // The terminal may have been resized during the session.
        self.rebuild_panes();
        self.render_all();
        match status {
            Ok(s) if s.success() => self.footer_say(" Back from Claude", 46),
            Ok(s) => self.footer_say(&format!(" Claude exited with {}", s), 226),
            Err(e) => self.footer_say(&format!(" Could not launch claude: {}", e), 196),
        }
    }

    fn rebuild_panes(&mut self) {
        let (cols, rows) = Crust::terminal_size();
        self.cols = cols;
        self.rows = rows;
        let (header, list, genres, wish, dump, detail, footer) = Self::build_panes(cols, rows);
        self.header = header;
        self.list = list;
        self.genres = genres;
        self.wish = wish;
        self.dump = dump;
        self.detail = detail;
        self.footer = footer;
        self.apply_focus_border();
    }

    /// Long text in the middle of the screen. The poster is drawn OVER
    /// the detail pane, so anything that has to be readable goes here
    /// instead of into that pane.
    fn show_text(&mut self, text: &str) {
        self.clear_poster();
        let w = self.cols.saturating_sub(8).min(100).max(40);
        let h = self.rows.saturating_sub(6).max(10);
        let mut popup = Popup::centered(w, h, 255, 234);
        popup.pane.wrap = true;
        popup.view(text);
        self.repaint_screen();
    }

    /// Wipe and repaint the whole layout. A popup covers more than the
    /// panes do — its own border, the gutters between panes, the pane
    /// borders — and none of that belongs to a pane, so `render_all`
    /// alone leaves the popup's crumbs on screen. The wipe takes care of
    /// that, and the two `full_refresh` calls take care of the other
    /// half: header and footer draw through `say`, which diffs against
    /// the previous frame and would skip the very rows the wipe blanked.
    fn repaint_screen(&mut self) {
        Crust::clear_screen();
        self.render_all();
        self.header.full_refresh();
        self.footer.full_refresh();
    }

    fn show_help(&mut self) {
        let help = format!("\
{}

  TAB / S-TAB    Switch focus between panes
  j/k  UP/DOWN   Move within the focused pane
  PgUP / PgDOWN  Page

{}
  1-9            Rate the highlighted title (1-9)
  0              Rate it 10
  DEL            Clear my rating
  m              Show only what I have rated, best first
  c              Ask Claude what to watch next, then pick from the
                 answer: ENTER adds to the list, w adds + wishlists
  C              Discuss recommendations with Claude
                 (both stay in the current view and genre filter)

{}
  +              Wish list (list) / Include genre (genres)
  -              Dump (list) / Exclude genre / Remove (wish+dump)
  Space          Clear genre filter on highlighted genre
  l              Toggle Movies/Series view
  o              Cycle sort (TMDB rating / alphabetical / my rating)
  r              Set minimum rating
  y / Y          Set min / max year

{}
  /              Search TMDB — pick from the hits, blurb below the list
  I              Full fetch of top-rated lists (background)
  i              Incremental fetch of missing details
  f              Re-fetch current item
  v              Verify data (fetch first 20 missing)
  L              Load additional lists (popular movies + TV)
  D              Remove duplicate entries
  K              Set TMDB API key (required — see README)
  R              Set streaming region
  W              Save config now

  ? / q          This help / Quit

{}",
            style::bold(&style::fg("watchit — movies and series, rated my way", 226)),
            style::bold(&style::fg("MY RATINGS", 81)),
            style::bold(&style::fg("BROWSING", 81)),
            style::bold(&style::fg("DATA", 81)),
            style::fg("  ESC / q / ENTER closes  ·  j/k scrolls", 240),
        );
        self.show_text(&help);
    }
}

fn move_bounded(idx: &mut usize, n: i32, total: usize) {
    if total == 0 { *idx = 0; return; }
    let new = (*idx as i32 + n).clamp(0, total as i32 - 1);
    *idx = new as usize;
}
