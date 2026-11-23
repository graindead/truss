//! The Packs page: chart packs, setlists and full albums posted on Custom Songs Central
//! (customsongscentral.com). The list is read from the site's own pages, newest first. Clicking
//! a pack shows its description (read from its page) and its downloads, which go through the
//! download queue into a folder in the library. Game setlists (the official Guitar Hero and Rock
//! Band games converted for Clone Hero) are listed alongside, from game_setlists.rs.

use iced::widget::{button, column, container, image, progress_bar, row, scrollable, space, text, text_input};
use iced::{Alignment, ContentFit, Element, Length, Task, Theme};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;

const SITE: &str = "https://customsongscentral.com";
/// Thumbnails are decoded at this size (a 16:9 banner), shown at half that.
const THUMB_W: u32 = 224;
const THUMB_H: u32 = 126;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// Official game setlists (game_setlists.rs), not from Custom Songs Central.
    Games,
    SongPacks,
    Setlists,
    Monthly,
    Community,
    Esports,
    Other,
}

impl Kind {
    const ALL: [Kind; 7] = [Kind::Games, Kind::SongPacks, Kind::Setlists, Kind::Monthly, Kind::Community, Kind::Esports, Kind::Other];

    fn label(self) -> &'static str {
        match self {
            Kind::Games => "Game setlists",
            Kind::SongPacks => "Song packs & albums",
            Kind::Setlists => "Setlists",
            Kind::Monthly => "Monthly packs",
            Kind::Community => "Community projects",
            Kind::Esports => "Esports",
            Kind::Other => "Other",
        }
    }

    /// The site's tag for posts of this kind (its cards carry `tag-<slug>` classes).
    fn from_tag(tag: &str) -> Option<Kind> {
        Some(match tag {
            "song-packs" => Kind::SongPacks,
            "setlists" => Kind::Setlists,
            "monthly-packs" => Kind::Monthly,
            "community-projects" => Kind::Community,
            "esports" => Kind::Esports,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    pub title: String,
    pub url: String,
    pub image: Option<String>,
    pub kind: Kind,
    /// As the site words it ("2 months ago").
    pub age: String,
}

/// Packs whose pages have no download links (a rules page, say): they're left out of the list.
/// Made by `list_packs_without_links`.
fn no_download_links(url: &str) -> bool {
    static LIST: std::sync::LazyLock<std::collections::HashSet<String>> =
        std::sync::LazyLock::new(|| serde_json::from_str(include_str!("../data/packs-without-links.json")).unwrap_or_default());
    LIST.contains(url)
}

// ---------- Reading the site ----------

use crate::api::CLIENT;

async fn get_text(url: &str) -> Result<Option<String>, String> {
    let r = CLIENT.get(url).timeout(std::time::Duration::from_secs(20)).send().await.map_err(|e| e.to_string())?;
    if r.status().as_u16() == 404 {
        return Ok(None);
    }
    let r = r.error_for_status().map_err(|e| e.to_string())?;
    r.text().await.map(Some).map_err(|e| e.to_string())
}

fn page_url(n: u32) -> String {
    if n == 1 { format!("{SITE}/") } else { format!("{SITE}/page/{n}/") }
}

/// Every pack on the site, newest first. The home page lists all posts ten to a page.
pub async fn fetch_all() -> Result<Vec<Pack>, String> {
    let first = get_text(&page_url(1)).await?.ok_or("Custom Songs Central's home page wasn't found")?;
    let pages = page_count(&first).unwrap_or(1).min(200);
    let mut html = vec![first];
    // A few pages at a time, kept in order.
    let rest: Vec<u32> = (2..=pages).collect();
    for chunk in rest.chunks(4) {
        let got = futures::future::join_all(chunk.iter().map(|&n| async move { get_text(&page_url(n)).await })).await;
        for g in got {
            if let Some(h) = g? {
                html.push(h);
            }
        }
    }
    let mut seen = HashSet::new();
    let packs: Vec<Pack> = html.iter().flat_map(|h| parse_cards(h)).filter(|p| seen.insert(p.url.clone())).collect();
    if packs.is_empty() {
        return Err("No packs found. Custom Songs Central may have changed its pages.".into());
    }
    let list = packs.clone();
    let _ = tokio::task::spawn_blocking(move || save_cache(&list)).await;
    Ok(packs)
}

/// "Page 1 of 29" → 29.
fn page_count(html: &str) -> Option<u32> {
    let i = html.find("m-pagination__text")?;
    let s = &html[i..];
    let s = &s[s.find(" of ")? + 4..];
    s[..s.find('<')?].trim().parse().ok()
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let a = s.find(start)? + start.len();
    let b = s[a..].find(end)? + a;
    Some(&s[a..b])
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

fn absolute(u: &str) -> String {
    if u.starts_with("http") { u.to_string() } else { format!("{SITE}{u}") }
}

/// The post cards on one of the site's list pages, keeping only packs (not charter profiles,
/// tools or tutorials).
fn parse_cards(html: &str) -> Vec<Pack> {
    let mut out = Vec::new();
    for chunk in html.split("<article class=\"").skip(1) {
        let card = &chunk[..chunk.find("</article>").unwrap_or(chunk.len())];
        let classes = &card[..card.find('"').unwrap_or(0)];
        if !classes.contains("m-article-card") {
            continue;
        }
        let tags: Vec<&str> =
            classes.split_whitespace().filter_map(|c| c.strip_prefix("tag-")).filter(|t| !t.starts_with("hash-")).collect();
        let kind = match tags.iter().find_map(|t| Kind::from_tag(t)) {
            Some(k) => k,
            None if tags.is_empty() => Kind::Other,
            None => continue,
        };
        // The title link: <a href="/slug/" class="m-article-card__info-link" aria-label="Title">
        let Some(link_at) = card.find("class=\"m-article-card__info-link\"") else { continue };
        let Some(href_at) = card[..link_at].rfind("href=\"") else { continue };
        let href = &card[href_at + 6..link_at];
        let href = &href[..href.find('"').unwrap_or(href.len())];
        let title = between(&card[link_at..], "aria-label=\"", "\"")
            .or_else(|| between(card, "m-article-card__title", "</h2>").and_then(|t| between(t, "title=\"", "\"")))
            .map(unescape)
            .unwrap_or_default();
        if href.is_empty() || title.is_empty() {
            continue;
        }
        let image = between(card, "m-article-card__picture-background\" src=\"", "\"").map(|s| absolute(&unescape(s)));
        let age = between(card, "m-article-card__timestamp\">", "</span>")
            .map(|s| unescape(s.trim_start().trim_start_matches("<span>").trim()))
            .unwrap_or_default();
        out.push(Pack { title, url: absolute(href), image, kind, age });
    }
    out
}

// ---------- Caches ----------

fn cache_dir() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("packs"))
}

fn save_cache(packs: &[Pack]) {
    let Some(dir) = cache_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(b) = serde_json::to_vec(packs) {
        let _ = std::fs::write(dir.join("packs.json"), b);
    }
}

fn load_cache() -> Option<Vec<Pack>> {
    serde_json::from_slice(&std::fs::read(cache_dir()?.join("packs.json")).ok()?).ok()
}

fn art_path(url: &str) -> Option<PathBuf> {
    let name: String = url.trim_start_matches(SITE).chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' }).collect();
    Some(cache_dir()?.join("art").join(name))
}

/// A pack's banner as a small thumbnail: from the disk cache, else downloaded (and cached).
async fn fetch_art(url: String) -> Option<image::Handle> {
    let path = art_path(&url);
    let cached = path.clone().and_then(|p| std::fs::read(p).ok());
    let bytes = match cached {
        Some(b) => b,
        None => {
            let r = CLIENT.get(&url).timeout(std::time::Duration::from_secs(20)).send().await.ok()?;
            let b = r.error_for_status().ok()?.bytes().await.ok()?.to_vec();
            if let Some(p) = &path {
                let _ = std::fs::create_dir_all(p.parent()?);
                let _ = std::fs::write(p, &b);
            }
            b
        }
    };
    tokio::task::spawn_blocking(move || {
        let img = ::image::load_from_memory(&bytes).ok()?;
        // Fill the 16:9 box, cropping the middle.
        let t = img.resize_to_fill(THUMB_W, THUMB_H, ::image::imageops::FilterType::Triangle).to_rgba8();
        Some(image::Handle::from_rgba(t.width(), t.height(), t.into_raw()))
    })
    .await
    .ok()?
}

// ---------- A pack's page ----------

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(String),
    Para(String),
    Item(String),
}

#[derive(Debug, Clone)]
pub struct Link {
    /// Where the link ends up (short links such as bit.ly are followed).
    pub url: String,
    pub label: String,
    pub size: Option<u64>,
    /// The folder it goes into, when it isn't named after the pack (game setlists).
    pub folder: Option<String>,
    /// Something to know before downloading it (Google Drive's download limit reached).
    pub note: Option<String>,
    /// A game setlist's download: (game title, its label), to find its charts in
    /// `game_sources`.
    pub game: Option<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Details {
    pub blocks: Vec<Block>,
    pub links: Vec<Link>,
}

/// Reads a pack's description and download links from its page.
pub async fn fetch_details(url: String) -> Result<Details, String> {
    let html = get_text(&url).await?.ok_or("This pack's page is gone.")?;
    // The post's own element (the page's stylesheet also names the class).
    let start = html.find("js-post-content\"").and_then(|i| html[i..].find('>').map(|j| i + j + 1)).ok_or("Couldn't read this pack's page.")?;
    let body = &html[start..];
    let end = ["<section class=\"m-tags", "</article>"].iter().filter_map(|m| body.find(m)).min().unwrap_or(body.len());
    let (blocks, hrefs) = html_blocks(&body[..end]);
    let links = futures::future::join_all(hrefs.into_iter().map(resolve_link)).await;
    let mut links: Vec<Link> = links.into_iter().collect();
    // Several Google Drive links have no file names to tell them apart.
    let unnamed = links.iter().filter(|l| l.label.starts_with("Google Drive")).count();
    if unnamed > 1 {
        let mut n = 0;
        for l in links.iter_mut().filter(|l| l.label.starts_with("Google Drive")) {
            n += 1;
            l.label = format!("Part {n} ({})", l.label);
        }
    }
    Ok(Details { blocks, links })
}

/// Follows short links and reads the file's name and size where the host gives them.
async fn resolve_link(href: String) -> Link {
    let url = follow_short(href).await;
    let mut size = None;
    let lower = url.to_ascii_lowercase();
    if !lower.contains("drive.google.com")
        && !lower.contains("mega.nz")
        && let Ok(r) = CLIENT.head(&url).timeout(std::time::Duration::from_secs(10)).send().await
        && r.status().is_success()
    {
        size = r.content_length().filter(|&n| n > 0);
    }
    let host = url.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or("").trim_start_matches("www.").to_string();
    let label = if url.contains("drive.google.com") {
        if url.contains("/folders/") { "Google Drive folder".to_string() } else { "Google Drive file".to_string() }
    } else {
        let file = url.split(['?', '#']).next().unwrap_or(&url).rsplit('/').next().unwrap_or("");
        let file = percent_decode(file);
        if file.contains('.') { file } else { host.clone() }
    };
    Link { url, label, size, folder: None, note: None, game: None }
}

/// A game setlist's downloads (read from the spreadsheet), with each Google Drive file's size,
/// or a note that Drive's download limit for it has been reached.
async fn game_details(game: crate::game_setlists::Game) -> Result<Details, String> {
    use crate::game_setlists::{Probe, folder, probe};
    let probes = futures::future::join_all(game.links.iter().map(|l| probe(&l.url))).await;
    let links = game
        .links
        .iter()
        .zip(probes)
        .map(|(l, p)| {
            let (size, note) = match p {
                Probe::Size(n) => (Some(n), None),
                Probe::OverLimit => (
                    None,
                    Some("Google Drive's download limit for this file has been reached. Try again later (it can take up to a day).".to_string()),
                ),
                Probe::Unknown => (None, None),
            };
            let host = if l.url.contains("drive.google.com") {
                if l.url.contains("/folders/") { "Google Drive folder".to_string() } else { "Google Drive file".to_string() }
            } else {
                l.url.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or("").trim_start_matches("www.").to_string()
            };
            Link {
                url: l.url.clone(),
                label: format!("{} ({host})", l.label),
                size,
                folder: Some(folder(&game.title, &l.label)),
                note,
                game: Some((game.title.clone(), l.label.clone())),
            }
        })
        .collect();
    let blocks = vec![
        Block::Para(format!(
            "{}: the game's own setlist converted for Clone Hero by the community, as linked from the Clone Hero Songs, Setlists, and Backgrounds/Highways Master List.",
            game.title
        )),
        Block::Para(
            "The main setlist is the game itself; its DLC and extras are separate downloads. Each goes into its own folder in Game Setlists, in your pack downloads folder. Install gets the charts one by one from Chorus (a few from RhythmVerse), skipping the ones you have; Download archive gets the spreadsheet's archive from Google Drive."
                .into(),
        ),
    ];
    Ok(Details { blocks, links })
}

/// Where a short link (bit.ly and the like) points, read one hop at a time so a destination
/// that won't answer (MEGA) is still found.
pub(crate) async fn follow_short(href: String) -> String {
    static NO_REDIRECT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .user_agent(concat!("truss/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    });
    let mut url = href;
    for _ in 0..5 {
        let host = url.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or("").to_ascii_lowercase();
        if !["bit.ly", "tinyurl.com", "t.co", "rebrand.ly", "goo.gl", "is.gd"].contains(&host.as_str()) {
            break;
        }
        let Ok(r) = NO_REDIRECT.head(&url).timeout(std::time::Duration::from_secs(10)).send().await else { break };
        let Some(next) = r.headers().get("location").and_then(|v| v.to_str().ok()) else { break };
        url = next.to_string();
    }
    url
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Download links are the page's "Download" buttons; failing those, links to file hosts.
fn is_file_host(href: &str) -> bool {
    let l = href.to_ascii_lowercase();
    ["b-cdn.net", "drive.google.com", "bit.ly", "mega.nz", "mediafire.com", "dropbox.com"].iter().any(|h| l.contains(h))
}

/// A pack page's text as headings, paragraphs and list items, and its download links.
fn html_blocks(html: &str) -> (Vec<Block>, Vec<String>) {
    #[derive(Clone, Copy, PartialEq)]
    enum Cur {
        Para,
        Heading,
        Item,
    }
    let mut blocks = Vec::new();
    let mut buttons = Vec::new();
    let mut hosts = Vec::new();
    let mut buf = String::new();
    let mut cur = Cur::Para;
    // An element being skipped, and how deep inside it (same-named elements nested in it).
    let mut skip: Option<(String, u32)> = None;
    let mut in_button = false;
    let flush = |buf: &mut String, cur: &mut Cur, blocks: &mut Vec<Block>| {
        let cells: Vec<String> = buf
            .split('\u{1}')
            .map(|c| clean_text(c))
            .filter(|c| !c.is_empty())
            .collect();
        let t = cells.join(" · ");
        buf.clear();
        let kind = std::mem::replace(cur, Cur::Para);
        if t.is_empty() || t.chars().all(|c| c == '-' || c == '*' || c == '_') {
            return;
        }
        blocks.push(match kind {
            Cur::Heading => Block::Heading(t),
            Cur::Item => Block::Item(t),
            Cur::Para => Block::Para(t),
        });
    };
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skip.is_none() && !in_button {
                buf.push_str(rest);
            }
            break;
        };
        if skip.is_none() && !in_button {
            buf.push_str(&rest[..lt]);
        }
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map(|e| &rest[e + 3..]).unwrap_or("");
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        if let Some((s, depth)) = &mut skip {
            if *s == name && !tag.ends_with('/') {
                if !closing {
                    *depth += 1;
                } else if *depth == 0 {
                    skip = None;
                } else {
                    *depth -= 1;
                }
            }
            continue;
        }
        // Share buttons and the like.
        if !closing && attr(tag, "class").is_some_and(|c| c.split_whitespace().any(|c| c == "m-share" || c.starts_with("kg-bookmark"))) {
            skip = Some((name, 0));
            continue;
        }
        match name.as_str() {
            "script" | "style" | "iframe" | "figure" | "svg" | "noscript" | "button" if !closing => skip = Some((name, 0)),
            "a" if !closing => {
                let href = attr(tag, "href").map(|h| unescape(&h)).unwrap_or_default();
                let class = attr(tag, "class").unwrap_or_default();
                if class.split_whitespace().any(|c| c == "download") && href.starts_with("http") {
                    buttons.push(href);
                    in_button = true;
                } else if is_file_host(&href) {
                    hosts.push(href);
                }
            }
            "a" => in_button = false,
            "td" | "th" if closing => buf.push('\u{1}'),
            "br" => buf.push('\n'),
            "p" | "div" | "ul" | "ol" | "table" | "tr" | "blockquote" | "center" | "hr" | "section" => flush(&mut buf, &mut cur, &mut blocks),
            "li" => {
                flush(&mut buf, &mut cur, &mut blocks);
                if !closing {
                    cur = Cur::Item;
                }
            }
            n if n.len() == 2 && n.starts_with('h') && n[1..].parse::<u8>().is_ok() => {
                flush(&mut buf, &mut cur, &mut blocks);
                if !closing {
                    cur = Cur::Heading;
                }
            }
            _ => {}
        }
    }
    flush(&mut buf, &mut cur, &mut blocks);
    let mut links = if buttons.is_empty() { hosts } else { buttons };
    let mut seen = HashSet::new();
    links.retain(|l| seen.insert(l.clone()));
    (blocks, links)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)? + key.len();
    Some(tag[i..i + tag[i..].find('"')?].to_string())
}

/// Entities decoded, invisible filler characters dropped and whitespace collapsed (line breaks
/// kept).
fn clean_text(s: &str) -> String {
    let s = unescape(s);
    let s = decode_numeric(&s);
    s.split('\n')
        .map(|line| {
            line.chars()
                .filter(|c| !matches!(c, '\u{17b4}' | '\u{17b5}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}'))
                .map(|c| if c == '\u{a0}' { ' ' } else { c })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// `&#8217;` and `&#x2019;` style entities.
fn decode_numeric(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("&#") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let end = after.find(';').filter(|&e| e <= 8);
        let ch = end.and_then(|e| {
            let n = &after[..e];
            let v = match n.strip_prefix(['x', 'X']) {
                Some(h) => u32::from_str_radix(h, 16).ok(),
                None => n.parse().ok(),
            };
            v.and_then(char::from_u32)
        });
        match (ch, end) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &after[e + 1..];
            }
            _ => {
                out.push_str("&#");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------- Downloading ----------

/// A pack download for the download queue.
#[derive(Debug, Clone)]
pub struct Job {
    pub url: String,
    /// The folder it goes into, in the library.
    pub folder: String,
    /// The pack's page, so its row can show the download's progress.
    pub page: String,
}

/// Downloads a pack into `<library>/<folder>`, unpacking its archives.
#[cfg(feature = "rhythmverse")]
pub fn download(job: Job, settings: crate::settings::Settings) -> impl futures::Stream<Item = crate::download::Progress> {
    use futures::SinkExt;
    iced::stream::channel(32, async move |mut tx: futures::channel::mpsc::Sender<crate::download::Progress>| {
        let r = run(&job, &settings, &mut tx).await;
        let _ = tx.send(crate::download::Progress::Finished(r)).await;
    })
}

#[cfg(feature = "rhythmverse")]
async fn run(
    job: &Job,
    settings: &crate::settings::Settings,
    tx: &mut futures::channel::mpsc::Sender<crate::download::Progress>,
) -> Result<PathBuf, String> {
    use crate::download::{Progress, TempGuard};
    use futures::SinkExt;
    let library = settings.pack_folder().ok_or("No songs folder set. Choose one in Settings.")?;
    if !library.is_dir() {
        if settings.pack_path.is_some() {
            return Err(format!("Pack downloads folder not found: {}. Change it in Settings > Library folders.", library.display()));
        }
        return Err(format!("Library folder not found: {}", library.display()));
    }
    let dest = library.join(&job.folder);
    if dest.exists() {
        return Err(format!("Already in library: {}", dest.display()));
    }
    // Game setlists go into a folder of their own ("Game Setlists/<game>").
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot write to library: {e}"))?;
    }
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let work = library.join(format!(".truss-tmp-pack-{n}-{}", std::process::id()));
    let _guard = TempGuard(Some(work.clone()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("Cannot write to library: {e}"))?;
    let unpacked = work.join("pack");
    crate::rhythmverse::fetch_pack(&job.url, &work, &unpacked, settings.download_videos, tx).await?;
    let _ = tx.send(Progress::Status("Unpacking...".into())).await;
    crate::rhythmverse::unpack_nested(&unpacked, settings.download_videos).await?;
    let root = {
        let u = unpacked.clone();
        tokio::task::spawn_blocking(move || {
            if !has_chart(&u, 0) {
                return Err("No Clone Hero charts were found in the download.".to_string());
            }
            Ok(single_folder(&u))
        })
        .await
        .map_err(|e| e.to_string())??
    };
    let _ = tx.send(Progress::Status("Moving into library...".into())).await;
    crate::download::move_path(&root, &dest)?;
    Ok(dest)
}

/// A pack's own top folder, if everything is in one (so it isn't nested twice). Wrappers
/// inside wrappers ("Rock Band 3/Rock Band 3/…") are unwrapped too, down to the first folder
/// that holds more than one thing, so tier folders are kept; a lone song folder is kept whole.
#[cfg(feature = "rhythmverse")]
fn single_folder(dir: &std::path::Path) -> PathBuf {
    let mut dir = dir.to_path_buf();
    for _ in 0..4 {
        let entries: Vec<_> = std::fs::read_dir(&dir).map(|rd| rd.flatten().collect()).unwrap_or_default();
        match entries.as_slice() {
            [only] if only.file_type().is_ok_and(|t| t.is_dir()) && !is_chart_folder(&only.path()) => dir = only.path(),
            [only] if only.file_type().is_ok_and(|t| t.is_dir()) => return only.path(),
            _ => break,
        }
    }
    dir
}

/// A folder that is itself one chart (song.ini, notes.chart/mid or a .sng directly inside).
#[cfg(feature = "rhythmverse")]
fn is_chart_folder(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        n == "song.ini" || n == "notes.chart" || n == "notes.mid" || n.ends_with(".sng")
    })
}

#[cfg(feature = "rhythmverse")]
fn has_chart(dir: &std::path::Path, depth: u32) -> bool {
    if depth > 8 {
        return false;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            return has_chart(&e.path(), depth + 1);
        }
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        n == "notes.chart" || n == "notes.mid" || n.ends_with(".sng")
    })
}

/// True if Truss can download this link itself.
pub fn can_download(url: &str) -> bool {
    #[cfg(feature = "rhythmverse")]
    return crate::rhythmverse::pack_host_ok(url);
    #[cfg(not(feature = "rhythmverse"))]
    {
        let _ = url;
        false
    }
}

fn human_size(n: u64) -> String {
    let gb = n as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", n as f64 / 1e6) }
}

// ---------- Page ----------

#[derive(Default)]
pub struct Page {
    packs: Vec<Pack>,
    loading: bool,
    /// Read from the site this run (else the list shown is the saved one, or nothing).
    fresh: bool,
    error: Option<String>,
    filter: String,
    kind: Option<Kind>,
    art: HashMap<String, image::Handle>,
    art_requested: HashSet<String>,
    /// The expanded pack (its page URL).
    open: Option<String>,
    /// Pack pages read so far: `None` while reading.
    details: HashMap<String, Option<Result<Details, String>>>,
    /// Which charts are in each pack (see pack_index.rs).
    index: Option<std::sync::Arc<crate::pack_index::PackIndex>>,
    index_requested: bool,
    /// Where each game setlist chart can be downloaded on its own (see game_sources.rs).
    sources: Option<std::sync::Arc<crate::game_sources::Sources>>,
    /// What the last chart-by-chart install did (or why it couldn't).
    install_note: Option<String>,
    /// Patch links (see patches.rs): applied or not, as last checked, and the one being
    /// applied or reverted.
    patched: HashMap<String, bool>,
    patch_busy: Option<String>,
    hide_installed: bool,
    /// Game setlists, as list rows and by their key.
    games: Vec<Pack>,
    game_data: HashMap<String, crate::game_setlists::Game>,
    games_requested: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Cached(Option<Vec<Pack>>),
    Loaded(Result<Vec<Pack>, String>),
    Art(String, Option<image::Handle>),
    Filter(String),
    Kind(Option<Kind>),
    Toggle(String),
    Details(String, Result<Details, String>),
    Open(String),
    Download(Job),
    Retry,
    Index(Option<crate::pack_index::PackIndex>),
    HideInstalled(bool),
    Games(Vec<crate::game_setlists::Game>),
    Sources(Option<crate::game_sources::Sources>),
    /// Install a game setlist download chart by chart: (folder, its charts).
    InstallGame(String, Vec<crate::game_sources::Source>),
    /// Apply a game setlist patch: (its link, the game).
    ApplyPatch(String, String),
    RevertPatch(String),
}

/// What the page asks of the app.
pub enum Out {
    None,
    Download(Job),
    /// Download these charts (the ones not already in the library) into this folder of the
    /// pack downloads folder.
    InstallCharts(String, Vec<crate::game_sources::Source>),
    /// Download a patch and put it into the game's charts: (its link, the game).
    ApplyPatch(String, String),
    RevertPatch(String),
}

/// A pack download's state, as the app's download queue has it.
pub enum DlState {
    Queued,
    Active { status: String, done: u64, total: Option<u64> },
    Done,
    Failed,
}

/// A pack download in the queue.
pub struct DlRow {
    /// The pack's page.
    pub page: String,
    pub state: DlState,
}

/// From the app: the pack downloads, their states by link, and the library folder.
pub struct Ctx<'a> {
    /// A pack link's download state ("Queued", "Downloading 40%", "Downloaded", "Failed: ...").
    pub status: &'a HashMap<String, String>,
    pub downloads: &'a [DlRow],
    pub library: Option<&'a std::path::Path>,
    /// True for a chart key that's in the library.
    pub in_library: &'a dyn Fn(&str) -> bool,
}

impl Page {
    /// Shows what a chart-by-chart install did, under the downloads.
    pub fn set_install_note(&mut self, note: Option<String>) {
        self.install_note = note;
    }

    /// Library keys of a game's official charts, from the game sources list (every download).
    pub fn game_source_keys(&self, game: &str) -> HashSet<String> {
        self.sources
            .as_ref()
            .and_then(|s| s.games.get(game))
            .map(|labels| labels.values().flatten().map(|s| s.key()).collect())
            .unwrap_or_default()
    }

    /// A patch finished applying or reverting: shows what it did and checks its state again.
    pub fn patch_done(&mut self, url: &str, note: String) {
        self.patch_busy = None;
        self.patched.insert(url.to_string(), crate::patches::applied(url).is_some());
        self.install_note = Some(note);
    }

    /// Called when the page is shown: reads the list once per run (showing the saved one meanwhile).
    pub fn opened(&mut self) -> Task<Msg> {
        let index = if self.index_requested {
            Task::none()
        } else {
            self.index_requested = true;
            Task::perform(crate::pack_index::load(), Msg::Index)
        };
        let index = if self.games_requested {
            index
        } else {
            self.games_requested = true;
            Task::batch([
                index,
                Task::perform(crate::game_setlists::load(), Msg::Games),
                Task::perform(crate::game_sources::load(), Msg::Sources),
            ])
        };
        if self.fresh || self.loading {
            return index;
        }
        self.loading = true;
        self.error = None;
        let cached = if self.packs.is_empty() {
            Task::perform(async { tokio::task::spawn_blocking(load_cache).await.ok().flatten() }, Msg::Cached)
        } else {
            Task::none()
        };
        Task::batch([index, cached, Task::perform(fetch_all(), Msg::Loaded)])
    }

    /// How much of a pack is in the library, if the pack index knows its charts.
    fn page_have(&self, page: &str, cx: &Ctx<'_>) -> Option<crate::pack_index::Have> {
        // A game setlist installed chart by chart: counted by the charts it installs.
        if let Some(lists) = page.strip_prefix(crate::game_setlists::KEY_PREFIX).and_then(|t| self.sources.as_ref()?.games.get(t)) {
            let keys: HashSet<String> = lists.values().flatten().map(|s| s.key()).collect();
            if !keys.is_empty() {
                return Some(crate::pack_index::Have { have: keys.iter().filter(|k| (cx.in_library)(k)).count(), total: keys.len() });
            }
        }
        self.index.as_ref()?.have_page(page, cx.in_library)
    }

    /// CSC packs, then the game setlists.
    /// Every pack shown: the site's (less those with nothing to download) and the game setlists.
    fn all(&self) -> impl Iterator<Item = &Pack> {
        self.packs.iter().filter(|p| !no_download_links(&p.url)).chain(self.games.iter())
    }

    /// A game setlist whose main setlist is in its folder in the library.
    fn game_installed(&self, page: &str, cx: &Ctx<'_>) -> bool {
        let Some(g) = self.game_data.get(page) else { return false };
        // Installed chart by chart: every chart of its main setlist is here.
        if let Some(list) = self.sources.as_ref().and_then(|s| s.get(&g.title, crate::game_setlists::MAIN)) {
            return list.iter().all(|s| (cx.in_library)(&s.key()));
        }
        let Some(lib) = cx.library else { return false };
        lib.join(crate::game_setlists::folder(&g.title, crate::game_setlists::MAIN)).exists()
    }

    pub fn update(&mut self, m: Msg) -> (Task<Msg>, Out) {
        let task = match m {
            Msg::Cached(Some(p)) if !self.fresh => {
                self.packs = p;
                self.load_art()
            }
            Msg::Cached(_) => Task::none(),
            Msg::Loaded(r) => {
                self.loading = false;
                match r {
                    Ok(p) => {
                        self.packs = p;
                        self.fresh = true;
                        self.load_art()
                    }
                    Err(e) => {
                        self.error = Some(e);
                        Task::none()
                    }
                }
            }
            Msg::Art(url, h) => {
                if let Some(h) = h {
                    self.art.insert(url, h);
                }
                Task::none()
            }
            Msg::Filter(s) => {
                self.filter = s;
                Task::none()
            }
            Msg::Kind(k) => {
                self.kind = k;
                Task::none()
            }
            Msg::Toggle(url) => {
                if self.open.as_ref() == Some(&url) {
                    self.open = None;
                    return (Task::none(), Out::None);
                }
                self.open = Some(url.clone());
                self.install_note = None;
                // A game's downloads are checked each time it's opened (Drive's download limit
                // comes and goes); while that's checked the last answer stays up.
                if let Some(g) = self.game_data.get(&url).cloned() {
                    if !matches!(self.details.get(&url), Some(Some(Ok(_)))) {
                        self.details.insert(url.clone(), None);
                    }
                    return (Task::perform(game_details(g), move |r| Msg::Details(url.clone(), r)), Out::None);
                }
                // Read once; a failed read is tried again when the pack is opened again.
                if matches!(self.details.get(&url), Some(None | Some(Ok(_)))) {
                    return (Task::none(), Out::None);
                }
                self.details.insert(url.clone(), None);
                Task::perform(fetch_details(url.clone()), move |r| Msg::Details(url.clone(), r))
            }
            Msg::Details(url, r) => {
                if let Ok(d) = &r {
                    for l in d.links.iter().filter(|l| l.game.as_ref().is_some_and(|(_, lb)| crate::patches::is_patch(lb))) {
                        self.patched.insert(l.url.clone(), crate::patches::applied(&l.url).is_some());
                    }
                }
                self.details.insert(url, Some(r));
                Task::none()
            }
            Msg::Open(url) => {
                let url = if url.starts_with(crate::game_setlists::KEY_PREFIX) { crate::game_setlists::SHEET_URL.to_string() } else { url };
                let _ = open::that_detached(url);
                Task::none()
            }
            Msg::Download(job) => return (Task::none(), Out::Download(job)),
            Msg::Sources(s) => {
                self.sources = s.map(std::sync::Arc::new);
                Task::none()
            }
            Msg::InstallGame(folder, list) => return (Task::none(), Out::InstallCharts(folder, list)),
            Msg::ApplyPatch(url, game) => {
                self.patch_busy = Some(url.clone());
                self.install_note = Some("Downloading the patch...".into());
                return (Task::none(), Out::ApplyPatch(url, game));
            }
            Msg::RevertPatch(url) => {
                self.patch_busy = Some(url.clone());
                self.install_note = None;
                return (Task::none(), Out::RevertPatch(url));
            }
            Msg::Retry => self.opened(),
            Msg::Index(i) => {
                self.index = i.map(std::sync::Arc::new);
                Task::none()
            }
            Msg::HideInstalled(b) => {
                self.hide_installed = b;
                Task::none()
            }
            Msg::Games(g) => {
                self.games = g
                    .iter()
                    .map(|g| Pack { title: g.title.clone(), url: g.key(), image: None, kind: Kind::Games, age: g.series.clone() })
                    .collect();
                self.game_data = g.into_iter().map(|g| (g.key(), g)).collect();
                Task::none()
            }
        };
        (task, Out::None)
    }

    /// Thumbnails for packs that don't have one yet, a few at a time.
    fn load_art(&mut self) -> Task<Msg> {
        let urls: Vec<String> =
            self.packs.iter().filter_map(|p| p.image.clone()).filter(|u| self.art_requested.insert(u.clone())).collect();
        if urls.is_empty() {
            return Task::none();
        }
        Task::run(
            futures::stream::StreamExt::buffered(
                futures::stream::iter(urls.into_iter().map(|u| async move {
                    let h = fetch_art(u.clone()).await;
                    (u, h)
                })),
                6,
            ),
            |(u, h)| Msg::Art(u, h),
        )
    }

    fn shown(&self, cx: &Ctx<'_>) -> Vec<&Pack> {
        let words: Vec<String> = self.filter.to_lowercase().split_whitespace().map(str::to_string).collect();
        self.all()
            .filter(|p| self.kind.is_none_or(|k| p.kind == k))
            .filter(|p| {
                !self.hide_installed || !(self.page_have(&p.url, cx).is_some_and(|h| h.have == h.total) || self.game_installed(&p.url, cx))
            })
            .filter(|p| {
                let t = p.title.to_lowercase();
                words.iter().all(|w| t.contains(w.as_str()))
            })
            .collect()
    }

    pub fn view<'a>(&'a self, cx: &Ctx<'_>) -> Element<'a, Msg> {
        let chip = |label: String, k: Option<Kind>| -> Element<'_, Msg> {
            button(text(label).size(12))
                .padding([4, 10])
                .style(crate::style::quiet_button(self.kind == k))
                .on_press(Msg::Kind(k))
                .into()
        };
        let count = |k: Kind| self.all().filter(|p| p.kind == k).count();
        let mut chips = row![chip(format!("All {}", self.all().count()), None)].spacing(6).align_y(Alignment::Center);
        for k in Kind::ALL {
            let n = count(k);
            if n > 0 {
                chips = chips.push(chip(format!("{} {n}", k.label()), Some(k)));
            }
        }
        if self.index.as_ref().is_some_and(|i| !i.links.is_empty()) {
            chips = chips.push(space().width(10)).push(
                button(text("Hide installed").size(12))
                    .padding([4, 10])
                    .style(crate::style::quiet_button(self.hide_installed))
                    .on_press(Msg::HideInstalled(!self.hide_installed)),
            );
        }
        let search = text_input("Filter packs", &self.filter)
            .on_input(Msg::Filter)
            .padding([6, 10])
            .size(13)
            .width(260)
            .style(crate::style::search_input);
        let mut status = row![].spacing(10).align_y(Alignment::Center);
        if self.loading {
            status = status.push(text("Checking for new packs…").size(12).style(muted));
        }
        if let Some(e) = &self.error {
            status = status
                .push(text(format!("Couldn't read Custom Songs Central: {e}")).size(12).style(danger))
                .push(button(text("Retry").size(12)).padding([3, 10]).style(crate::style::quiet_button(false)).on_press(Msg::Retry));
        }
        let link = |label: &'static str, url: &str| {
            button(text(label).size(12))
                .padding(0)
                .style(|t: &Theme, st| button::Style {
                    background: None,
                    text_color: if matches!(st, button::Status::Hovered | button::Status::Pressed) {
                        crate::style::fg(t)
                    } else {
                        crate::style::accent(t)
                    },
                    ..Default::default()
                })
                .on_press(Msg::Open(url.to_string()))
        };
        let header = column![
            row![text("Packs").size(16), space().width(Length::Fill), status, search].spacing(12).align_y(Alignment::Center),
            row![
                text("Packs are from").size(12).style(muted),
                link("customsongscentral.com ↗", SITE),
                text("and game setlists from the").size(12).style(muted),
                link("Clone Hero Master List ↗", crate::game_setlists::SHEET_URL),
                text("— thanks to their pack makers and charters. Click a pack to read about it and download it.").size(12).style(muted),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .wrap(),
            chips.wrap(),
        ]
        .spacing(10);

        let shown = self.shown(cx);
        let body: Element<'_, Msg> = if shown.is_empty() {
            let msg = if self.packs.is_empty() && self.games.is_empty() {
                if self.loading { "Loading packs…" } else { "No packs to show." }
            } else {
                "No packs match."
            };
            container(text(msg).size(13).style(muted)).padding(20).into()
        } else {
            let rows = shown.into_iter().fold(column![].spacing(2), |col, p| {
                let col = col.push(self.view_pack(p, cx));
                if self.open.as_ref() == Some(&p.url) { col.push(self.view_details(p, cx)) } else { col }
            });
            scrollable(container(rows).padding(list_pad())).height(Length::Fill).into()
        };
        column![container(header).padding([14, 18]), crate::style::divider(false), body].height(Length::Fill).into()
    }

    fn view_pack<'a>(&'a self, p: &'a Pack, cx: &Ctx<'_>) -> Element<'a, Msg> {
        let (w, h) = (THUMB_W as f32 / 2.0, THUMB_H as f32 / 2.0);
        let thumb: Element<'a, Msg> = match p.image.as_ref().and_then(|u| self.art.get(u)) {
            Some(handle) => image(handle.clone()).width(w).height(h).content_fit(ContentFit::Cover).into(),
            None => {
                // Game setlists have no banner: their series' initials ("GH", "RB").
                let initials: String = if p.kind == Kind::Games { p.age.split_whitespace().filter_map(|w| w.chars().next()).collect() } else { String::new() };
                container(text(initials).size(20).font(iced::Font { weight: iced::font::Weight::Bold, ..iced::Font::DEFAULT }).style(muted))
                    .width(w)
                    .height(h)
                    .center(Length::Fixed(w))
                    .style(|t: &Theme| container::Style { background: Some(crate::style::raised(t).into()), ..Default::default() })
                    .into()
            }
        };
        let games = p.kind == Kind::Games;
        let kind = if games { format!("{} · Game setlist", p.age) } else { p.kind.label().to_string() };
        let info = column![text(&p.title).size(14), text(kind).size(12).style(muted)].spacing(3).width(Length::Fill);
        let open = self.open.as_ref() == Some(&p.url);
        let badge: Element<'a, Msg> = match self.page_have(&p.url, cx) {
            Some(h) if h.have == h.total => text("Installed").size(12).style(success).into(),
            Some(h) if h.have > 0 => text(format!("{} of {} charts", h.have, h.total)).size(12).style(accent).into(),
            None if self.game_installed(&p.url, cx) => text("Installed").size(12).style(success).into(),
            _ => space().width(0).into(),
        };
        // This pack's downloads (one per link): a bar while any is waiting or running.
        let mine: Vec<&DlState> = cx.downloads.iter().filter(|d| d.page == p.url).map(|d| &d.state).collect();
        let running = mine.iter().any(|s| matches!(s, DlState::Queued | DlState::Active { .. }));
        let progress = running.then(|| {
            let part = |s: &DlState| match s {
                DlState::Active { status, done, total: Some(t) } if status == "Downloading" && *t > 0 => *done as f32 / *t as f32,
                // Unpacking and moving come after the download.
                DlState::Active { status, .. } if status != "Downloading" && !status.starts_with("Starting") => 1.0,
                DlState::Done => 1.0,
                _ => 0.0,
            };
            mine.iter().map(|s| part(s)).sum::<f32>() / mine.len().max(1) as f32
        });
        let badge: Element<'a, Msg> = match mine.iter().rev().find(|s| !matches!(s, DlState::Done)) {
            Some(DlState::Queued) if !mine.iter().any(|s| matches!(s, DlState::Active { .. })) => text("Waiting…").size(12).style(muted).into(),
            Some(DlState::Active { status, .. }) if status != "Downloading" => text(status.clone()).size(12).style(accent).into(),
            Some(DlState::Active { .. }) => text(format!("{:.0}%", progress.unwrap_or(0.0) * 100.0)).size(12).style(accent).into(),
            Some(DlState::Failed) if !running => text("Download failed").size(12).style(danger).into(),
            _ => badge,
        };
        let age = if games { "" } else { p.age.as_str() };
        let line = row![thumb, info, badge, text(age).size(12).style(muted), text(if open { "▴" } else { "▾" }).size(12).style(muted)]
            .spacing(14)
            .align_y(Alignment::Center);
        let content: Element<'a, Msg> = match progress {
            Some(f) => column![line, progress_bar(0.0..=1.0, f).girth(3)].spacing(4).into(),
            None => line.into(),
        };
        button(content)
            .padding([6, 8])
            .width(Length::Fill)
            .style(crate::style::row_button(open, false))
            .on_press(Msg::Toggle(p.url.clone()))
            .into()
    }

    /// The expanded pack: its downloads and its description.
    fn view_details<'a>(&'a self, p: &'a Pack, cx: &Ctx<'_>) -> Element<'a, Msg> {
        let games = p.kind == Kind::Games;
        let page_button =
            button(text(if games { "Open spreadsheet" } else { "Open page" }).size(12)).padding([5, 12]).style(crate::style::quiet_button(false)).on_press(Msg::Open(p.url.clone()));
        let inner: Element<'a, Msg> = match self.details.get(&p.url) {
            None | Some(None) => text("Reading the pack's page…").size(13).style(muted).into(),
            Some(Some(Err(e))) => {
                column![text(format!("Couldn't read this pack's page: {e}")).size(13).style(danger), page_button].spacing(10).into()
            }
            Some(Some(Ok(d))) => {
                let several = d.links.len() > 1;
                let mut downloads = column![].spacing(6);
                for l in &d.links {
                    let folder =
                        l.folder.clone().unwrap_or_else(|| crate::download::sanitize_filename(&pack_folder(&p.title, &l.label, several)));
                    // A patch (GH3's explicit vocals) goes into the game's charts already in the
                    // library, so it's applied and reverted rather than downloaded as a pack.
                    if let Some((game, label)) = l.game.as_ref().filter(|(_, lb)| crate::patches::is_patch(lb))
                        && can_download(&l.url)
                    {
                        let busy = self.patch_busy.as_deref() == Some(l.url.as_str());
                        let applied = self.patched.get(&l.url).copied().unwrap_or(false);
                        let size = l.size.map(|n| format!(" · {}", human_size(n))).unwrap_or_default();
                        let what = format!("{label}{size} · replaces or adds audio in the game's charts you have");
                        let main = button(text(if busy { "Working..." } else if applied { "Patched" } else { "Apply patch" }).size(12))
                            .padding([5, 12])
                            .style(crate::style::quiet_button(true));
                        let main: Element<'a, Msg> = if busy || applied || self.patch_busy.is_some() {
                            main.into()
                        } else {
                            main.on_press(Msg::ApplyPatch(l.url.clone(), game.clone())).into()
                        };
                        let mut line = row![main].spacing(10).align_y(Alignment::Center);
                        if applied {
                            let revert = button(text("Revert").size(12)).padding([5, 12]).style(crate::style::quiet_button(false));
                            line = line.push(if busy || self.patch_busy.is_some() { revert } else { revert.on_press(Msg::RevertPatch(l.url.clone())) });
                        }
                        line = line.push(text(what).size(12).style(muted));
                        if let Some(n) = l.note.as_ref().filter(|_| !applied) {
                            line = line.push(text(n.clone()).size(12).style(danger));
                        }
                        downloads = downloads.push(line);
                        continue;
                    }
                    // A game setlist download whose charts can each be got from Chorus (or
                    // RhythmVerse): installed chart by chart, the archive kept as a fallback.
                    if let Some((game, label)) = &l.game
                        && let Some(list) = self.sources.as_ref().and_then(|s| s.get(game, label))
                    {
                        let have = list.iter().filter(|s| (cx.in_library)(&s.key())).count();
                        let rv = list.iter().filter(|s| s.from == "rv").count();
                        let from = if rv == 0 { "from Chorus".to_string() } else { format!("from Chorus ({rv} from RhythmVerse)") };
                        let missing: Vec<crate::game_sources::Source> = list.iter().filter(|s| !(cx.in_library)(&s.key())).cloned().collect();
                        let all_in = missing.is_empty();
                        let name = if all_in {
                            format!("Installed ({} charts)", list.len())
                        } else if have == 0 {
                            format!("Install {} charts", list.len())
                        } else {
                            format!("Install the other {}", missing.len())
                        };
                        let b = button(text(name).size(12)).padding([5, 12]).style(crate::style::quiet_button(true));
                        let b: Element<'a, Msg> = if missing.is_empty() { b.into() } else { b.on_press(Msg::InstallGame(folder.clone(), missing)).into() };
                        let what = format!("{} · {from}", label);
                        let mut line = row![b, text(what).size(12).style(muted)].spacing(10).align_y(Alignment::Center);
                        let state = if have > 0 && !all_in {
                            format!("You have {have} of its {} charts", list.len())
                        } else {
                            String::new()
                        };
                        if !state.is_empty() {
                            line = line.push(text(state).size(12).style(accent));
                        }
                        downloads = downloads.push(line);
                    }
                    let folder_exists = cx.library.is_some_and(|lib| lib.join(&folder).exists());
                    // The pack index knows its charts, wherever they are; else the pack's folder.
                    let have = self.index.as_ref().and_then(|i| i.have(&l.url, cx.in_library));
                    let in_library = have.map_or(folder_exists, |h| h.have == h.total);
                    // Fully installed shows on the button itself, so no state text for it here.
                    let state = cx.status.get(&l.url).cloned().or_else(|| match have {
                        Some(h) if h.have == h.total => None,
                        Some(h) if h.have > 0 => Some(format!("You have {} of its {} charts", h.have, h.total)),
                        Some(_) => None,
                        None => None,
                    });
                    let busy = state.as_deref().is_some_and(|s| s == "Queued" || s.starts_with("Downloading") || s.ends_with("..."));
                    let label = match l.size {
                        Some(n) => format!("{} · {}", l.label, human_size(n)),
                        None => l.label.clone(),
                    };
                    let action: Element<'a, Msg> = if can_download(&l.url) {
                        let job = Job { url: l.url.clone(), folder, page: p.url.clone() };
                        let per_chart = l.game.as_ref().is_some_and(|(g, lb)| self.sources.as_ref().is_some_and(|s| s.get(g, lb).is_some()));
                        let name = if in_library {
                            match have {
                                Some(h) => format!("Installed ({} charts)", h.total),
                                None => "Installed".to_string(),
                            }
                        } else if per_chart {
                            "Download archive".to_string()
                        } else if several {
                            "Download".to_string()
                        } else {
                            "Download pack".to_string()
                        };
                        let b = button(text(name).size(12))
                            .padding([5, 12])
                            .style(crate::style::quiet_button(true));
                        if busy || in_library { b.into() } else { b.on_press(Msg::Download(job)).into() }
                    } else {
                        let host = l.url.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or("").trim_start_matches("www.").to_string();
                        button(text(format!("Open on {host}")).size(12))
                            .padding([5, 12])
                            .style(crate::style::quiet_button(false))
                            .on_press(Msg::Open(l.url.clone()))
                            .into()
                    };
                    let mut line = row![action, text(label).size(12).style(muted)].spacing(10).align_y(Alignment::Center);
                    if let Some(n) = l.note.as_ref().filter(|_| state.is_none()) {
                        line = line.push(text(n.clone()).size(12).style(danger));
                    }
                    if let Some(s) = state {
                        let failed = s.starts_with("Failed");
                        line = line.push(text(s).size(12).style(move |t: &Theme| {
                            if failed { danger(t) } else { text::Style { color: Some(crate::style::accent(t)) } }
                        }));
                    }
                    downloads = downloads.push(line);
                }
                if d.links.is_empty() {
                    downloads = downloads.push(text("This page has no download link.").size(12).style(muted));
                }
                if let Some(n) = self.install_note.as_ref().filter(|_| d.links.iter().any(|l| l.game.is_some())) {
                    downloads = downloads.push(text(n.clone()).size(12).style(accent));
                }
                let mut desc = column![].spacing(6);
                for b in &d.blocks {
                    desc = desc.push(match b {
                        Block::Heading(t) => Element::from(
                            text(t).size(14).font(iced::Font { weight: iced::font::Weight::Semibold, ..iced::Font::DEFAULT }),
                        ),
                        Block::Para(t) => text(t).size(13).into(),
                        Block::Item(t) => row![text("•").size(13).style(muted), text(t).size(13)].spacing(8).into(),
                    });
                }
                column![
                    downloads,
                    text(if games {
                        "Downloads go into the Game Setlists folder in your pack downloads folder (set in Settings). A full game is around 1 GB; DLC folders can be much bigger."
                    } else {
                        "Downloads go into a folder named after the pack (in your pack downloads folder, set in Settings). Packs can be several GB."
                    })
                    .size(11)
                    .style(muted),
                    desc,
                    page_button,
                ]
                .spacing(12)
                .into()
            }
        };
        container(inner)
            .padding(iced::Padding { top: 10.0, right: 18.0, bottom: 16.0, left: 8.0 + THUMB_W as f32 / 2.0 + 14.0 })
            .width(Length::Fill)
            .into()
    }
}

/// The library folder a pack's download goes into: the pack's title, plus the file's name when
/// the pack has several downloads (a base pack and its DLC, say).
fn pack_folder(title: &str, label: &str, several: bool) -> String {
    if !several {
        return title.to_string();
    }
    let stem = label.rsplit_once('.').map(|(a, _)| a).unwrap_or(label);
    let stem = stem.split(" (Google Drive").next().unwrap_or(stem).trim();
    // "Marathon Hero 2 - Full Albums DLC.rar" already names its pack.
    if stem.to_lowercase().starts_with(&title.trim().to_lowercase()) { stem.to_string() } else { format!("{title} - {stem}") }
}

fn muted(t: &Theme) -> text::Style {
    text::Style { color: Some(crate::style::muted(t)) }
}

fn success(t: &Theme) -> text::Style {
    text::Style { color: Some(t.extended_palette().success.base.color) }
}

fn accent(t: &Theme) -> text::Style {
    text::Style { color: Some(crate::style::accent(t)) }
}

fn danger(t: &Theme) -> text::Style {
    text::Style { color: Some(t.extended_palette().danger.base.color) }
}

fn list_pad() -> iced::Padding {
    iced::Padding { top: 6.0, right: 18.0, bottom: 12.0, left: 10.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &str = r#"<article class="m-article-card  post tag-song-packs tag-hash-import-2026">
    <div class="m-article-card__picture">
      <a href="/guitar-zero-encore-2/" class="m-article-card__picture-link" aria-hidden="true" tabindex="-1"></a>
        <img class="m-article-card__picture-background" src="/content/images/size/w600/2026/08/image.png" loading="lazy" alt="">
    </div>
      <div class="m-article-card__info">
        <a href="https://customsongscentral.com/tag/song-packs/" class="m-article-card__tag">Song Packs and Full Albums</a>
      <a href="/guitar-zero-encore-2/" class="m-article-card__info-link" aria-label="Guitar Zero Encore 2: King Gizzard &amp; the Lizard Wizard">
        <div><h2 class="m-article-card__title js-article-card-title " title="x">x</h2></div>
        <div class="m-article-card__timestamp">
          <span>2 months ago</span>
          <span>•</span>
        </div>
      </a>
    </div>
  </article>
  <article class="m-article-card  post tag-charters"><a href="/someone/" class="m-article-card__info-link" aria-label="A charter"></a></article>
  <article class="m-article-card  post tag-hash-import-2026"><a href="/nirvana/" class="m-article-card__info-link" aria-label="Nirvana Discography"></a></article>
  <span class="m-pagination__text">Page 2 of 29</span>"#;

    #[test]
    fn reads_cards() {
        let p = parse_cards(CARD);
        assert_eq!(p.len(), 2, "charter profiles are left out");
        assert_eq!(p[0].title, "Guitar Zero Encore 2: King Gizzard & the Lizard Wizard");
        assert_eq!(p[0].url, "https://customsongscentral.com/guitar-zero-encore-2/");
        assert_eq!(p[0].image.as_deref(), Some("https://customsongscentral.com/content/images/size/w600/2026/08/image.png"));
        assert_eq!(p[0].kind, Kind::SongPacks);
        assert_eq!(p[0].age, "2 months ago");
        assert_eq!(p[1].kind, Kind::Other);
        assert_eq!(page_count(CARD), Some(29));
    }

    #[test]
    fn reads_page_text() {
        let html = "<div class=\"m-share\">x</div><figure class=\"kg-card\"><iframe src=\"y\"></iframe></figure>\n\
<p>Welcome to Rage Quit, for Clone Hero &amp; YARG&#8217;s fans!</p><ul><li>100 (+1) songs.</li><li>Pro Drums</li></ul>\n\
<h2>Credits</h2><!--kg-card-begin: html--><a class=\"download\" href=\"https://customsongscentral.b-cdn.net/Rage%20Quit.7z\">Download</a>\n\
<table><tr><td>\u{17b5}</td><td><strong>Organizer</strong></td></tr><tr><td>fob911</td><td>Yenlow</td></tr></table>\n\
<p>---</p><a href=\"https://drive.google.com/drive/folders/abc\">other</a>";
        let (b, links) = html_blocks(html);
        assert_eq!(
            b,
            vec![
                Block::Para("Welcome to Rage Quit, for Clone Hero & YARG\u{2019}s fans!".into()),
                Block::Item("100 (+1) songs.".into()),
                Block::Item("Pro Drums".into()),
                Block::Heading("Credits".into()),
                Block::Para("Organizer".into()),
                Block::Para("fob911 · Yenlow".into()),
                Block::Para("other".into()),
            ]
        );
        assert_eq!(links, vec!["https://customsongscentral.b-cdn.net/Rage%20Quit.7z".to_string()], "download buttons win over other file links");
        assert_eq!(percent_decode("Rage%20Quit%20%5B1%5D.7z"), "Rage Quit [1].7z");
        assert_eq!(pack_folder("Marathon Hero 2", "Marathon Hero 2.rar", true), "Marathon Hero 2");
        assert_eq!(pack_folder("Marathon Hero 2", "Marathon Hero 2 - Full Albums DLC.rar", true), "Marathon Hero 2 - Full Albums DLC");
        assert_eq!(pack_folder("Revolved", "Part 1 (Google Drive file)", true), "Revolved - Part 1");
    }

    /// Reads real pack pages (TRUSS_TEST_PACKS=1).
    #[test]
    fn reads_pack_page() {
        if std::env::var_os("TRUSS_TEST_PACKS").is_none() {
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        for u in [
            "https://customsongscentral.com/rage-quit/",
            "https://customsongscentral.com/symphonyx-discography-setlist/",
            "https://customsongscentral.com/marathonhero2/",
            "https://customsongscentral.com/zero-gravity/",
        ] {
            let d = rt.block_on(fetch_details(u.into())).unwrap();
            println!("{u}: {} blocks, first {:?}", d.blocks.len(), d.blocks.first());
            for l in &d.links {
                println!("  {} | {:?} | {} | in Truss: {}", l.label, l.size, l.url, can_download(&l.url));
            }
            assert!(!d.links.is_empty() && !d.blocks.is_empty());
        }
    }

    /// Reads every pack's page and writes the ones with no download links to
    /// TRUSS_PACKS_NO_LINKS (a JSON list of page URLs), for data/packs-without-links.json.
    #[test]
    #[ignore]
    fn list_packs_without_links() {
        let Some(out) = std::env::var_os("TRUSS_PACKS_NO_LINKS") else { return };
        use futures::StreamExt;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let packs = rt.block_on(fetch_all()).unwrap();
        let checked: Vec<(String, Option<bool>)> = rt.block_on(
            futures::stream::iter(packs.iter().map(|p| p.url.clone()))
                .map(|u| async move {
                    let has = match get_text(&u).await {
                        Ok(Some(html)) => Some(!html_blocks(&html).1.is_empty()),
                        _ => None,
                    };
                    (u, has)
                })
                .buffer_unordered(6)
                .collect(),
        );
        let mut none: Vec<&String> = checked.iter().filter(|(_, h)| *h == Some(false)).map(|(u, _)| u).collect();
        none.sort();
        let failed = checked.iter().filter(|(_, h)| h.is_none()).count();
        println!("{} packs: {} without download links, {} couldn't be read", packs.len(), none.len(), failed);
        std::fs::write(PathBuf::from(out), serde_json::to_string_pretty(&none).unwrap()).unwrap();
    }

    /// The list of packs without download links reads.
    #[test]
    fn packs_without_links_list_reads() {
        assert!(no_download_links("https://customsongscentral.com/csc-championship-series-ruleset/"));
        assert!(!no_download_links("https://customsongscentral.com/rage-quit/"));
    }

    /// Wrapper folders are unwrapped; tier folders and a lone song folder are kept.
    #[cfg(feature = "rhythmverse")]
    #[test]
    fn unwraps_wrapper_folders() {
        let root = crate::test_dir().join(format!("truss-single-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |p: &str| {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        };
        // pack/Rock Band 3.zip -> Rock Band 3/Rock Band 3/<songs>
        mk("a/Rock Band 3/Rock Band 3/Song A/song.ini");
        mk("a/Rock Band 3/Rock Band 3/Song B/song.ini");
        assert_eq!(single_folder(&root.join("a")), root.join("a/Rock Band 3/Rock Band 3"));
        // tiers stay: GH3/<Tier 1, Tier 2>
        mk("b/GH3/Tier 1/Song/song.ini");
        mk("b/GH3/Tier 2/Song/song.ini");
        assert_eq!(single_folder(&root.join("b")), root.join("b/GH3"));
        // one song: the song folder itself
        mk("c/Wrap/Only Song/notes.mid");
        assert_eq!(single_folder(&root.join("c")), root.join("c/Wrap/Only Song"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Downloads a pack into a temporary library (TRUSS_TEST_PACK_DL = its download link).
    #[cfg(feature = "rhythmverse")]
    #[test]
    fn downloads_pack() {
        let Some(url) = std::env::var_os("TRUSS_TEST_PACK_DL") else { return };
        let url = url.to_string_lossy().to_string();
        let lib = crate::test_dir().join(format!("truss-pack-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&lib);
        std::fs::create_dir_all(&lib).unwrap();
        let settings = crate::settings::Settings { library_path: Some(lib.clone()), ..Default::default() };
        let job = Job { url, folder: "Test Pack".into(), page: String::new() };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut tx, mut rx) = futures::channel::mpsc::channel(64);
        let printer = std::thread::spawn(move || {
            let mut last = String::new();
            while let Some(p) = futures::executor::block_on(futures::StreamExt::next(&mut rx)) {
                if let crate::download::Progress::Status(s) = p
                    && s != last
                {
                    println!("  status: {s}");
                    last = s;
                }
            }
        });
        let t = std::time::Instant::now();
        let r = rt.block_on(run(&job, &settings, &mut tx));
        drop(tx);
        let _ = printer.join();
        println!("result after {:?}: {r:?}", t.elapsed());
        let dest = r.unwrap();
        let mut charts = 0;
        let mut stack = vec![dest.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else if n == "notes.chart" || n == "notes.mid" || n.ends_with(".sng") {
                    charts += 1;
                } else if [".zip", ".7z", ".rar"].iter().any(|x| n.ends_with(x)) {
                    panic!("archive left behind: {}", e.path().display());
                }
            }
        }
        let leftovers: Vec<_> = std::fs::read_dir(&lib).unwrap().flatten().map(|e| e.file_name()).collect();
        println!("{charts} charts in {}; library holds {leftovers:?}", dest.display());
        let top: Vec<_> = std::fs::read_dir(&dest).unwrap().flatten().take(5).map(|e| e.file_name()).collect();
        println!("top of pack folder: {top:?}");
        assert!(charts > 0);
        assert_eq!(leftovers.len(), 1, "temporary folders are cleaned up");
        let _ = std::fs::remove_dir_all(&lib);
    }

    /// Reads the real site (TRUSS_TEST_PACKS=1).
    #[test]
    fn reads_site() {
        if std::env::var_os("TRUSS_TEST_PACKS").is_none() {
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let p = rt.block_on(fetch_all()).unwrap();
        println!("{} packs; first: {:?}", p.len(), p.first());
        assert!(p.len() > 150);
    }

    /// Downloads a series' game setlists (TRUSS_GAMES_SERIES, "Guitar Hero") into TRUSS_GAMES_DEST,
    /// one folder per download, skipping those already there; with TRUSS_GAMES_INDEX (a pack
    /// index file) set, each finished folder's charts are added to it under its link.
    #[cfg(feature = "rhythmverse")]
    #[test]
    #[ignore]
    fn downloads_game_setlists() {
        let (Some(dest), Ok(series)) = (std::env::var_os("TRUSS_GAMES_DEST"), std::env::var("TRUSS_GAMES_SERIES")) else { return };
        let dest = PathBuf::from(dest);
        let index_path = std::env::var_os("TRUSS_GAMES_INDEX").map(PathBuf::from);
        let user = crate::settings::Settings::load();
        let settings = crate::settings::Settings { library_path: Some(dest.clone()), pack_path: None, download_videos: user.download_videos, ..Default::default() };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let games: Vec<_> = rt.block_on(crate::game_setlists::load()).into_iter().filter(|g| g.series.eq_ignore_ascii_case(&series)).collect();
        let mut todo = Vec::new();
        for g in &games {
            for l in &g.links {
                let name = crate::download::sanitize_filename(&if l.label == crate::game_setlists::MAIN { g.title.clone() } else { format!("{} - {}", g.title, l.label) });
                todo.push((g.key(), name, l.url.clone()));
            }
        }
        println!("{} downloads", todo.len());
        let (mut ok, mut failed) = (0, Vec::new());
        for (page, name, url) in todo {
            if !can_download(&url) {
                println!("SKIP {name}: can't download {url}");
                continue;
            }
            if !dest.join(&name).exists() {
                let job = Job { url: url.clone(), folder: name.clone(), page: page.clone() };
                let (mut tx, mut rx) = futures::channel::mpsc::channel(64);
                let label = name.clone();
                let printer = std::thread::spawn(move || {
                    let mut last = std::time::Instant::now();
                    while let Some(p) = futures::executor::block_on(futures::StreamExt::next(&mut rx)) {
                        match p {
                            crate::download::Progress::Bytes { done, total } if last.elapsed().as_secs() >= 30 => {
                                last = std::time::Instant::now();
                                println!("  {label}: {:.0} of {} MB", done as f64 / 1e6, total.map_or("?".into(), |t| format!("{:.0}", t as f64 / 1e6)));
                            }
                            crate::download::Progress::Status(s) => println!("  {label}: {s}"),
                            _ => {}
                        }
                    }
                });
                let t = std::time::Instant::now();
                let r = rt.block_on(run(&job, &settings, &mut tx));
                drop(tx);
                let _ = printer.join();
                match r {
                    Ok(_) => println!("OK  {name} in {:.0?}", t.elapsed()),
                    Err(e) => {
                        println!("ERR {name}: {e}");
                        failed.push(name);
                        continue;
                    }
                }
            }
            ok += 1;
            // Its charts, by the key Truss matches on, into the pack index.
            if let Some(ip) = &index_path {
                add_folder_to_index(ip, &dest.join(&name), url, page);
            }
        }
        println!("done: {ok} in place, {} failed {failed:?}", failed.len());
    }

    /// Adds the charts in a downloaded folder to a pack index file, under its download link.
    #[cfg(feature = "rhythmverse")]
    fn add_folder_to_index(ip: &std::path::Path, root: &std::path::Path, url: String, page: String) {
        let mut charts = Vec::new();
        let mut unread = 0;
        let mut stack = vec![root.to_path_buf()];
        let rel = |p: &std::path::Path| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(p);
                } else if n == "song.ini" {
                    charts.push((crate::library::ini_key(&std::fs::read(&p).unwrap_or_default()), rel(&d)));
                } else if n.ends_with(".sng") {
                    let head: Vec<u8> = std::fs::read(&p).unwrap_or_default().into_iter().take(256 << 10).collect();
                    match crate::sng::header_metadata(&head) {
                        Ok(Some(meta)) => charts.push((crate::library::pairs_key(&meta), rel(&p))),
                        _ => unread += 1,
                    }
                }
            }
        }
        charts.sort();
        let mut index = std::fs::read(ip).ok().and_then(|b| crate::pack_index::PackIndex::parse(&b)).unwrap_or_default();
        println!("    indexed {} charts", charts.len());
        index.links.insert(url, crate::pack_index::LinkEntry { page, charts, unread, error: None });
        index.built = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let tmp = ip.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&index).unwrap()).unwrap();
        std::fs::rename(&tmp, ip).unwrap();
    }

    /// Unpacks archives downloaded by hand (TRUSS_LOCAL_ARCHIVES = a file of lines
    /// "archive<TAB>folder<TAB>link<TAB>page") into TRUSS_GAMES_DEST the way a pack download
    /// would (archives naming the same folder go into it together), and adds each folder to
    /// TRUSS_GAMES_INDEX. The archives are only read. Folders already there are only indexed.
    #[cfg(feature = "rhythmverse")]
    #[test]
    #[ignore]
    fn unpacks_local_game_archives() {
        let (Some(list), Some(dest), Some(ip)) =
            (std::env::var_os("TRUSS_LOCAL_ARCHIVES"), std::env::var_os("TRUSS_GAMES_DEST"), std::env::var_os("TRUSS_GAMES_INDEX"))
        else {
            return;
        };
        let (dest, ip) = (PathBuf::from(dest), PathBuf::from(ip));
        let keep_video = crate::settings::Settings::load().download_videos;
        // Folder → (link, page, archives), in the file's order.
        let mut groups: Vec<(String, String, String, Vec<PathBuf>)> = Vec::new();
        for line in std::fs::read_to_string(&list).unwrap().lines().filter(|l| !l.trim().is_empty()) {
            let f: Vec<&str> = line.split('\t').collect();
            let (archive, folder, url, page) = (PathBuf::from(f[0]), f[1].to_string(), f[2].to_string(), f[3].to_string());
            match groups.iter_mut().find(|g| g.0 == folder) {
                Some(g) => g.3.push(archive),
                None => groups.push((folder, url, page, vec![archive])),
            }
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        for (n, (folder, url, page, archives)) in groups.into_iter().enumerate() {
            let target = dest.join(&folder);
            if !target.exists() {
                let work = dest.join(format!(".truss-tmp-local-{n}-{}", std::process::id()));
                let _guard = crate::download::TempGuard(Some(work.clone()));
                let _ = std::fs::remove_dir_all(&work);
                let t = std::time::Instant::now();
                let r: Result<(), String> = rt.block_on(async {
                    for a in &archives {
                        let stem = a.file_stem().unwrap_or_default().to_string_lossy().trim().to_string();
                        crate::rhythmverse::unpack_any(a.clone(), &work.join(stem), keep_video).await?;
                    }
                    crate::rhythmverse::unpack_nested(&work, keep_video).await
                });
                if let Err(e) = r.and_then(|()| {
                    if !has_chart(&work, 0) {
                        return Err("No Clone Hero charts were found.".to_string());
                    }
                    crate::download::move_path(&single_folder(&work), &target)
                }) {
                    println!("ERR {folder}: {e}");
                    continue;
                }
                println!("OK  {folder} in {:.0?}", t.elapsed());
            } else {
                println!("HAVE {folder}");
            }
            add_folder_to_index(&ip, &target, url, page);
        }
    }
}
