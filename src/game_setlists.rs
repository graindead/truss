//! Game setlists for the Packs page: the official Guitar Hero and Rock Band setlists (and their
//! DLC and extras) converted for Clone Hero, as linked from the community's "Clone Hero Songs,
//! Setlists, and Backgrounds/Highways Master List" spreadsheet. Its first tab is read live (its
//! public HTML view, no sign-in), falling back to the last copy read and then to the copy built
//! into Truss. Only its Guitar Hero and Rock Band sections are used.

use crate::packs::percent_decode;
use serde::{Deserialize, Serialize};

/// The spreadsheet, as people open it.
pub const SHEET_URL: &str = "https://docs.google.com/spreadsheets/d/13B823ukxdVMocowo1s5XnT3tzciOfruhUVePENKc01o/edit?gid=1870223413#gid=1870223413";
/// Its setlists tab as plain HTML.
const SHEET_HTML: &str =
    "https://docs.google.com/spreadsheets/d/13B823ukxdVMocowo1s5XnT3tzciOfruhUVePENKc01o/htmlview/sheet?headers=false&gid=1870223413";
/// The copy built into Truss, for when the sheet can't be read.
pub(crate) const SNAPSHOT: &str = include_str!("../data/game-setlists.json");
/// The sections used, as the sheet names them.
const SECTIONS: [&str; 2] = ["Guitar Hero", "Rock Band"];
/// Pack page keys for games start with this (they have no page of their own).
pub const KEY_PREFIX: &str = "truss-game:";
/// Their downloads go into this folder in the pack downloads folder.
pub const FOLDER: &str = "Game Setlists";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Game {
    /// "Guitar Hero" or "Rock Band".
    pub series: String,
    pub title: String,
    pub links: Vec<GameLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameLink {
    /// "Main setlist", "DLC", "Prototype"…
    pub label: String,
    pub url: String,
}

impl Game {
    /// Its key on the Packs page.
    pub fn key(&self) -> String {
        format!("{KEY_PREFIX}{}", self.title)
    }
}

/// The folder (in the pack downloads folder) a game's download goes into.
pub fn folder(game: &str, label: &str) -> String {
    let name = if label == MAIN { game.to_string() } else { format!("{game} - {label}") };
    format!("{FOLDER}/{}", crate::download::sanitize_filename(&name))
}

pub const MAIN: &str = "Main setlist";

/// The game setlists: read from the sheet, else the last copy read, else the built-in one.
pub async fn load() -> Vec<Game> {
    match fetch().await {
        Ok(g) => g,
        Err(e) => {
            log::warn!("game setlists: {e}");
            tokio::task::spawn_blocking(load_cache).await.ok().flatten().unwrap_or_else(snapshot)
        }
    }
}

async fn fetch() -> Result<Vec<Game>, String> {
    let html = crate::api::CLIENT
        .get(SHEET_HTML)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let mut games = parse(&html);
    if games.len() < 10 {
        return Err(format!("only {} games read; the sheet may have changed", games.len()));
    }
    // Short links (DJ Hero's bit.ly) are followed once, so the right host is shown.
    for g in &mut games {
        for l in &mut g.links {
            if l.url.contains("bit.ly/") {
                l.url = crate::packs::follow_short(l.url.clone()).await;
            }
        }
    }
    let copy = games.clone();
    let _ = tokio::task::spawn_blocking(move || save_cache(&copy)).await;
    Ok(games)
}

fn snapshot() -> Vec<Game> {
    serde_json::from_str(SNAPSHOT).unwrap_or_default()
}

fn cache_path() -> Option<std::path::PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("packs").join("game-setlists.json"))
}

fn save_cache(games: &[Game]) {
    let Some(p) = cache_path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
    if let Ok(b) = serde_json::to_vec(games) {
        let _ = std::fs::write(p, b);
    }
}

fn load_cache() -> Option<Vec<Game>> {
    serde_json::from_slice(&std::fs::read(cache_path()?).ok()?).ok()
}

// ---------- Reading the sheet ----------

struct Cell {
    col: usize,
    span: usize,
    text: String,
    links: Vec<String>,
}

/// The sheet's rows, each cell at its real column (merged cells taken into account).
fn grid(html: &str) -> Vec<Vec<Cell>> {
    let mut taken: std::collections::HashSet<(usize, usize)> = Default::default();
    let mut rows = Vec::new();
    let body = html.find("<tbody").map(|i| &html[i..]).unwrap_or(html);
    for (r, tr) in body.split("<tr").skip(1).enumerate() {
        let tr = &tr[..tr.find("</tr>").unwrap_or(tr.len())];
        let mut cells = Vec::new();
        let mut col = 0;
        for chunk in tr.split("<t").skip(1) {
            if !(chunk.starts_with('d') || chunk.starts_with('h')) || chunk[1..].starts_with("ead") || chunk[1..].starts_with("body") {
                continue;
            }
            let Some(gt) = chunk.find('>') else { continue };
            let attrs = &chunk[..gt];
            let inner = &chunk[gt + 1..];
            let inner = &inner[..inner.find("</td>").or_else(|| inner.find("</th>")).unwrap_or(inner.len())];
            while taken.contains(&(r, col)) {
                col += 1;
            }
            let num = |name: &str| attr(attrs, name).and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1);
            let (span, down) = (num("colspan"), num("rowspan"));
            for dr in 0..down {
                for dc in 0..span {
                    taken.insert((r + dr, col + dc));
                }
            }
            let links = hrefs(inner);
            let text = unescape(&strip_tags(inner)).trim().to_string();
            if !text.is_empty() || !links.is_empty() {
                cells.push(Cell { col, span, text, links });
            }
            col += span;
        }
        rows.push(cells);
    }
    rows
}

/// The Guitar Hero and Rock Band sections: one game per row, its title cell linking the main
/// setlist, then DLC and extras to its right.
pub fn parse(html: &str) -> Vec<Game> {
    let mut out = Vec::new();
    // The current section and the column it starts at.
    let mut section: Option<(String, usize)> = None;
    for cells in grid(html) {
        // A section heading spans the section's columns and starts the row (after the row
        // number); headings further right belong to the sheet's other lists.
        let lead = cells.iter().find(|c| !c.text.chars().all(|ch| ch.is_ascii_digit()) || !c.links.is_empty());
        if let Some(h) = lead.filter(|c| c.span >= 3 && c.links.is_empty()) {
            let name = h.text.trim();
            section = SECTIONS.iter().find(|s| s.eq_ignore_ascii_case(name)).map(|s| (s.to_string(), h.col));
            continue;
        }
        let Some((series, start)) = &section else { continue };
        let Some(first) = cells.iter().find(|c| c.col == *start) else { continue };
        let Some(main) = first.links.first().filter(|u| usable(u)) else { continue };
        let mut links = vec![GameLink { label: MAIN.into(), url: main.clone() }];
        for c in cells.iter().filter(|c| c.col > *start && c.col < start + 4) {
            if let Some(u) = c.links.first().filter(|u| usable(u)) {
                let label = if c.text.is_empty() { "Extra".to_string() } else { c.text.clone() };
                links.push(GameLink { label, url: u.clone() });
            }
        }
        out.push(Game { series: series.clone(), title: first.text.clone(), links });
    }
    out
}

/// A download link (not a video).
fn usable(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    l.starts_with("http") && !l.contains("youtube.com") && !l.contains("youtu.be")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}=\""))? + name.len() + 2;
    Some(tag[at..at + tag[at..].find('"')?].to_string())
}

/// The links in a cell, unwrapped from Google's redirect.
fn hrefs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in s.split("href=\"").skip(1) {
        let raw = unescape(&part[..part.find('"').unwrap_or(part.len())]);
        let real = match raw.find("google.com/url?q=") {
            Some(i) => {
                let q = &raw[i + 17..];
                percent_decode(&q[..q.find('&').unwrap_or(q.len())])
            }
            None => raw,
        };
        out.push(real);
    }
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let s = s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    // Numeric references (&#8217;), then &amp; last so "&amp;lt;" stays literal.
    crate::packs::decode_numeric(&s).replace("&amp;", "&")
}

// ---------- A game's downloads ----------

/// How a Google Drive file stands right now: its size, or why it can't be downloaded.
pub enum Probe {
    Size(u64),
    OverLimit,
    Unknown,
}

/// Asks Google Drive for a file's first byte: that gives its size, or Drive's "Quota exceeded"
/// page when too many people have downloaded it recently.
pub async fn probe(url: &str) -> Probe {
    let Some(id) = drive_file_id(url) else { return Probe::Unknown };
    let u = format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t");
    let Ok(r) = crate::api::CLIENT.get(&u).header("Range", "bytes=0-0").timeout(std::time::Duration::from_secs(15)).send().await else {
        return Probe::Unknown;
    };
    let html = r.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("text/html"));
    if html {
        let body = r.text().await.unwrap_or_default();
        return if is_quota_page(&body) { Probe::OverLimit } else { Probe::Unknown };
    }
    r.headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|n| n.parse().ok())
        .map_or(Probe::Unknown, Probe::Size)
}

/// Google Drive's page for a file that's been downloaded too much lately.
pub fn is_quota_page(body: &str) -> bool {
    body.contains("Quota exceeded") || body.contains("Too many users have viewed or downloaded this file recently")
}

/// A Drive file link's id (not a folder's).
fn drive_file_id(url: &str) -> Option<String> {
    if !url.contains("drive.google.com") || url.contains("/folders/") {
        return None;
    }
    let i = url.find("/file/d/").map(|i| i + 8).or_else(|| url.find("id=").map(|i| i + 3))?;
    let id: String = url[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<table><tbody>
<tr><th>1</th><td colspan="9"><a href="https://www.google.com/url?q=https://youtu.be/x&amp;sa=D">CLICK FOR "SORRY"</a></td></tr>
<tr><th>2</th><td colspan="4">Guitar Hero</td><td>Community Setlists &amp; Packs</td></tr>
<tr><th>3</th><td>Main Setlist</td><td>DLC</td><td colspan="2">Extra Content</td><td>CSC Website</td></tr>
<tr><th>4</th><td><a href="https://www.google.com/url?q=https://drive.google.com/file/d/AAA/view&amp;sa=D">Guitar Hero III: Legends of Rock</a></td><td><a href="https://drive.google.com/file/d/BBB/view">DLC</a></td><td></td><td><a href="https://www.youtube.com/watch?v=1">Endless Setlist</a></td><td><a href="https://customsongscentral.com/">Pack</a></td></tr>
<tr><th>5</th><td colspan="4" rowspan="2">Rock Band</td><td><a href="https://customsongscentral.com/p">Revolved</a></td></tr>
<tr><th>6</th><td><a href="https://customsongscentral.com/q">December</a></td></tr>
<tr><th>7</th><td><a href="https://drive.google.com/file/d/CCC/view">Rock Band 4</a></td><td></td><td><a href="https://drive.google.com/file/d/DDD/view">Rivals</a></td></tr>
<tr><th>8</th><td colspan="4">Other</td></tr>
<tr><th>9</th><td><a href="https://drive.google.com/drive/folders/EEE">Frets on Fire</a></td></tr>
</tbody></table>"#;

    #[test]
    fn reads_sections() {
        let g = parse(SAMPLE);
        assert_eq!(g.len(), 2, "{g:#?}");
        assert_eq!(g[0].series, "Guitar Hero");
        assert_eq!(g[0].title, "Guitar Hero III: Legends of Rock");
        assert_eq!(
            g[0].links,
            vec![
                GameLink { label: MAIN.into(), url: "https://drive.google.com/file/d/AAA/view".into() },
                GameLink { label: "DLC".into(), url: "https://drive.google.com/file/d/BBB/view".into() },
            ]
        );
        assert_eq!(g[1].series, "Rock Band");
        assert_eq!(g[1].links[1].label, "Rivals");
        assert_eq!(folder("Guitar Hero III: Legends of Rock", "DLC"), "Game Setlists/Guitar Hero III꞉ Legends of Rock - DLC");
    }

    #[test]
    fn snapshot_reads() {
        let g = snapshot();
        assert!(g.len() > 20 && g.iter().any(|g| g.series == "Rock Band"), "{} games", g.len());
    }

    /// Reads the live sheet. With TRUSS_WRITE_GAME_SNAPSHOT set, also writes it as the built-in copy.
    #[test]
    #[ignore]
    fn reads_live_sheet() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let g = rt.block_on(fetch()).unwrap();
        for x in &g {
            println!("{} | {} | {:?}", x.series, x.title, x.links.iter().map(|l| format!("{}={}", l.label, l.url)).collect::<Vec<_>>());
        }
        if std::env::var_os("TRUSS_WRITE_GAME_SNAPSHOT").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/game-setlists.json");
            std::fs::write(path, serde_json::to_string_pretty(&g).unwrap() + "\n").unwrap();
        }
    }
}
