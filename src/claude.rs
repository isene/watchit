//! `claude -p` recommendations built from my own ratings.
//!
//! Two ways in: `ask` runs one non-interactive turn on a background
//! thread (the TUI stays live), and `taste` builds the context both
//! that and the interactive session are seeded with. The interactive
//! side hands the terminal over to `claude` itself — see `discuss` in
//! main.rs — so a conversation costs no protocol of our own.

use std::io::Write;
use std::process::{Command, Stdio};

/// The alias, not a pinned id: the CLI resolves it to the current
/// Sonnet, so watchit doesn't rot when a new one ships.
const MODEL: &str = "sonnet";

/// One rated / wished / dumped title as the model should see it.
pub struct Seen {
    pub title: String,
    pub year: i32,
    pub kind: String,
    pub score: Option<u8>,
}

impl Seen {
    fn line(&self) -> String {
        let year = if self.year > 0 { format!(" ({})", self.year) } else { String::new() };
        match self.score {
            Some(s) => format!("{}{} — {}/10", self.title, year, s),
            None => format!("{}{}", self.title, year),
        }
    }
}

/// The taste profile every prompt starts from. Ratings first (that is
/// the signal), then what I already want to see and what I have thrown
/// out — both are "do not recommend this" lists as well as evidence.
pub fn taste(rated: &[Seen], wish: &[Seen], dump: &[Seen]) -> String {
    let mut s = String::new();
    s.push_str("Here is my taste in movies and series.\n\nRATED (my own 1-10):\n");
    if rated.is_empty() {
        s.push_str("(nothing rated yet)\n");
    } else {
        for r in rated { s.push_str(&format!("- {} [{}]\n", r.line(), r.kind)); }
    }
    if !wish.is_empty() {
        s.push_str("\nON MY WISH LIST (already want to see these — do not recommend them):\n");
        for w in wish { s.push_str(&format!("- {}\n", w.line())); }
    }
    if !dump.is_empty() {
        s.push_str("\nTHROWN OUT (not interested — do not recommend these, and read them as negative signal):\n");
        for d in dump { s.push_str(&format!("- {}\n", d.line())); }
    }
    s
}

/// The genre filter as it is set right now. The list on screen obeys it,
/// so a recommendation that ignores it does not belong to the session
/// the user is actually in.
pub fn genre_rule(include: &[String], exclude: &[String]) -> String {
    let mut s = String::new();
    if !include.is_empty() {
        s.push_str(&format!(
            "\n\nSTAY INSIDE THESE GENRES: {}. Every single recommendation has to be one \
             of them — I am browsing that shelf right now, so anything else is no use to me.",
            include.join(", ")));
    }
    if !exclude.is_empty() {
        s.push_str(&format!(
            "\n\nAVOID THESE GENRES ENTIRELY: {}. I have filtered them out.",
            exclude.join(", ")));
    }
    s
}

/// One line naming the filter, for the footer and the popup header.
pub fn genre_label(include: &[String], exclude: &[String]) -> String {
    let mut parts = Vec::new();
    if !include.is_empty() { parts.push(include.join(", ")); }
    if !exclude.is_empty() { parts.push(format!("not {}", exclude.join(", "))); }
    parts.join("  ·  ")
}

/// Prompt for the one-shot `c` recommendation.
pub fn recommend_prompt(taste: &str, n: usize, want: &str, genres: &str) -> String {
    let focus = if want.trim().is_empty() {
        String::new()
    } else {
        format!("\n\nWhat I am in the mood for right now: {}", want.trim())
    };
    format!(
        "{taste}{genres}{focus}\n\n\
         Recommend {n} movies or series I would probably love, based on what my \
         ratings actually show — the patterns in what I rate high and low, not \
         the obvious crowd-pleasers. Skip anything listed above. Lean towards \
         things I am unlikely to have already found on my own.\n\n\
         Format: one entry per title, exactly like this, with a blank line \
         between entries and NO other prose, headings or numbering:\n\n\
         Title (year) — Movie or Series\n\
         One or two sentences on why THIS fits what I rate highly, naming the \
         titles of mine it follows from.\n",
        taste = taste, genres = genres, focus = focus, n = n
    )
}

/// Opening turn for the interactive `C` session.
pub fn discuss_prompt(taste: &str, genres: &str) -> String {
    format!(
        "{taste}{genres}\n\n\
         You are my film and television companion. Use my ratings above as the \
         evidence for what I actually like. I want to talk about what to watch \
         next: suggest, argue, push back, ask me what I am in the mood for. Be \
         concrete and name titles. Keep it conversational and short unless I \
         ask you to go deep.\n\n\
         Open by telling me, in two or three sentences, what my ratings say \
         about my taste — including anything that surprises you. Then ask me \
         one question. /exit when we are done, and that returns me to watchit.\n",
        taste = taste, genres = genres
    )
}

/// Run one non-interactive turn. Prompt on stdin, answer on stdout —
/// the same shape library and the kastrup triage use.
pub fn ask(prompt: &str) -> Result<String, String> {
    let mut child = Command::new("claude")
        .arg("-p")
        .arg("--model").arg(MODEL)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn claude: {} — is the `claude` CLI on PATH?", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).map_err(|e| format!("write prompt: {}", e))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("wait: {}", e))?;
    if !out.status.success() {
        // The CLI prints API/policy errors on stdout, so an empty stderr
        // is not the same as no explanation.
        let err = String::from_utf8_lossy(&out.stderr);
        let so = String::from_utf8_lossy(&out.stdout);
        let detail = if !err.trim().is_empty() {
            err.trim().to_string()
        } else {
            so.trim().chars().take(300).collect::<String>()
        };
        return Err(format!("claude exited: {}", detail));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Is the CLI even installed? Checked before promising an answer.
pub fn available() -> bool {
    Command::new("claude").arg("--version")
        .stdout(Stdio::null()).stderr(Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}
