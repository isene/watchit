# Watchit - Terminal Movie & Series Browser

<img src="img/watchit.svg" align="left" width="150" height="150">

![Rust](https://img.shields.io/badge/language-Rust-f74c00) ![License](https://img.shields.io/badge/license-Unlicense-green) ![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-blue) ![Stay Amazing](https://img.shields.io/badge/Stay-Amazing-important)

Terminal browser for IMDb Top 250 movies and series. Cut down the time spent searching in favor of time spent watching. Rust feature port of [IMDB-terminal](https://github.com/isene/IMDB), built on [crust](https://github.com/isene/crust).

<br clear="left"/>

## Quick Start

```bash
# Build from source
git clone https://github.com/isene/watchit
cd watchit
cargo build --release

# Run — press I on first launch to scrape Top 250
./target/release/watchit
```

## Features

- **IMDb Top 250 movies + series** via embedded JSON-LD scrape
- **5-pane TUI**: main list, genre filter, wish list, dump list, detail view with poster
- **Detail cache** (plot, cast, directors, runtime, genres, poster URL)
- **TMDb streaming integration** — see where to watch each title in your region
- **Genre filtering** with include / exclude / clear per-genre
- **Wish and dump lists** separate for movies and series
- **Rating + year filters**
- **Sort by rating or alphabetically**
- **Search IMDb** to add new titles beyond the Top 250
- **Poster images** rendered inline via [glow](https://github.com/isene/glow) (kitty/sixel/w3m)
- **Async scrape + fetch** — background threads, no UI blocking

## Keys

| Key | Action |
|---|---|
| `TAB` / `S-TAB` | Switch focus between panes |
| `UP` / `DOWN`, `j` / `k` | Move within pane |
| `PgUP` / `PgDOWN` | Page |
| `HOME` / `END` | First / last |
| `+` | Wish (list pane) / Include (genres) |
| `-` | Dump (list) / Exclude (genres) / Remove (wish/dump) |
| `Space` | Clear genre filter |
| `l` | Toggle Movies / Series |
| `o` | Toggle sort (rating / alphabetical) |
| `r` | Set minimum rating |
| `y` / `Y` | Min / max year |
| `/` | Search IMDb |
| `I` | Full scrape of Top 250 (background) |
| `i` | Incremental fetch of missing details |
| `f` | Re-fetch current item |
| `D` | Remove duplicates |
| `k` | Set TMDb API key |
| `R` | Set streaming region |
| `W` | Save config |
| `?` / `q` | Help / Quit |

## Configuration

`~/.watchit/config.yml` (auto-created on first run):

```yaml
tmdb_key: ""
region: US
rating_min: 0.0
year_min: 0
year_max: 0
sort: rating
view: movies
show_posters: true
movie_limit: 250
series_limit: 250
wish_movies: []
wish_series: []
dump_movies: []
dump_series: []
genres_include: []
genres_exclude: []
```

- `tmdb_key`: Optional TMDb v3 API key for streaming info. Get one at [themoviedb.org](https://www.themoviedb.org/settings/api).
- `region`: ISO country code for streaming availability (e.g. `US`, `NO`, `GB`).
- Data cache: `~/.watchit/data/list.json` + `details.json` + `*.jpg`.

## TMDb Streaming Info (optional)

1. Sign up at [themoviedb.org](https://www.themoviedb.org)
2. API → Create → Developer; copy your v3 API key
3. Press `k` in watchit and paste the key
4. Press `R` to set your region
5. Press `i` to refetch details, including streaming providers

## Part of the Rust Terminal Suite (Fe2O3)

- [rush](https://github.com/isene/rush) — shell
- [pointer](https://github.com/isene/pointer) — file manager
- [kastrup](https://github.com/isene/kastrup) — messaging hub
- [scroll](https://github.com/isene/scroll) — web browser
- [tock](https://github.com/isene/tock) — calendar
- [nova](https://github.com/isene/nova) — astronomy panel
- **watchit** — IMDb browser

## License

Unlicense (public domain).
