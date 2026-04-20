# Watchit

Rust feature clone of [IMDB-terminal](https://github.com/isene/IMDB), a terminal browser for movies and series. Built on crust.

## Build

```bash
PATH="/usr/bin:$PATH" cargo build --release
```

## Modules

- `config.rs` - YAML config (~/.watchit/config.yml)
- `data.rs` - JSON data models: Database (list), Details cache
- `scrape.rs` - IMDb Top 250 JSON-LD scrape + details + autocomplete search
- `tmdb.rs` - Optional TMDb streaming providers lookup
- `main.rs` - TUI app with 5 panes (list/genres/wish/dump/detail)
