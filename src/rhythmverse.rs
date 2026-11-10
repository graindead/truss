//! RhythmVerse (experimental source).
//!
//! RhythmVerse has no public API yet, so this uses the JSON endpoint its own website uses, as
//! lightly and openly as possible:
//! - only live searches you start (one request per search, or per page as you scroll), never a
//!   copy of the whole catalogue;
//! - requests say they come from Truss (no pretending to be a web browser);
//! - only charts hosted on RhythmVerse itself are downloaded in the app. Charts that RhythmVerse
//!   only links to (mostly Google Drive) open in your browser instead.

use crate::api::{CLIENT, Chart, Sort, SortDir, SortKey};
use futures::{SinkExt, StreamExt, channel::mpsc::Sender};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const BASE: &str = "https://rhythmverse.co";
const PER_PAGE: u32 = 25;

/// RhythmVerse-specific details of a search result.
#[derive(Debug, Clone, Default)]
pub struct RvFile {
    /// Direct download (hosted on RhythmVerse), if any.
    pub download_url: Option<String>,
    /// Where the chart is hosted when RhythmVerse only links to it (e.g. Google Drive).
    pub external_url: Option<String>,
    /// The chart's page on RhythmVerse.
    pub page_url: String,
}

#[derive(Debug)]
pub struct RvPage {
    pub charts: Vec<Arc<Chart>>,
    pub total: u64,
}

fn absolute(u: &str) -> String {
    if u.starts_with("http://") || u.starts_with("https://") { u.to_string() } else { format!("{BASE}{}", if u.starts_with('/') { u.to_string() } else { format!("/{u}") }) }
}

fn s(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// RhythmVerse difficulty tiers: 1-6 charted, 0/missing = not charted.
fn diff(file: &Value, data: &Value, key: &str) -> Option<i32> {
    let v = int(&file[key]).or_else(|| int(&data[key]))?;
    (v > 0).then_some(v.min(6) as i32)
}

/// Formats Clone Hero can't load without converting (Rock Band / Guitar Hero console files).
fn needs_conversion(format: &str) -> bool {
    let f = format.to_ascii_lowercase();
    f.starts_with("rb") || f.starts_with("gh") || f.contains("con") || f.contains("xbox") || f.contains("ps3") || f.contains("wii")
}

fn to_chart(song: &Value) -> Option<Chart> {
    let (data, file) = (&song["data"], &song["file"]);
    let format = s(&file["gameformat"]).unwrap_or_default();
    if needs_conversion(&format) {
        return None;
    }
    let file_id = s(&file["file_id"])?;
    let download = s(&file["download_url"]).filter(|u| !u.eq_ignore_ascii_case("none"));
    let external = s(&file["external_url"]);
    // Hosted on RhythmVerse = a download link on rhythmverse.co and no outside host.
    let hosted = download.as_deref().map(absolute).filter(|u| u.starts_with(BASE) && external.is_none());
    let page = s(&file["file_url_full"]).or_else(|| s(&file["file_url"]).map(|u| absolute(&u))).unwrap_or_else(|| BASE.into());
    let art = s(&file["album_art"]).or_else(|| s(&data["album_art"])).map(|u| absolute(&u));
    let charter = s(&file["charter"]).or_else(|| s(&file["author"]["name"]));
    // A stable id that can't collide with Encore's chart ids.
    let chart_id = 9_000_000_000u64 + file_id.bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64)) % 1_000_000_000;
    let length = int(&data["song_length"]).or_else(|| int(&file["file_song_length"])).filter(|l| *l > 0).map(|secs| secs * 1000);
    Some(Chart {
        name: s(&data["title"]).or_else(|| s(&file["file_title"])),
        artist: s(&data["artist"]).or_else(|| s(&file["file_artist"])),
        album: s(&data["album"]).or_else(|| s(&file["file_album"])),
        genre: s(&data["genre"]).or_else(|| s(&file["file_genre"])),
        year: s(&data["year"]).or_else(|| s(&file["file_year"])),
        charter,
        chart_id,
        md5: format!("rv-{file_id}"),
        album_art_md5: art,
        song_length: length,
        diff_guitar: diff(file, data, "diff_guitar"),
        diff_guitar_coop: diff(file, data, "diff_guitar_coop"),
        diff_rhythm: diff(file, data, "diff_rhythm"),
        diff_bass: diff(file, data, "diff_bass"),
        diff_drums: diff(file, data, "diff_drums"),
        diff_keys: diff(file, data, "diff_keys"),
        diff_guitarghl: diff(file, data, "diff_guitarghl"),
        diff_bassghl: diff(file, data, "diff_bassghl"),
        diff_vocals: diff(file, data, "diff_vocals"),
        modified_time: s(&file["update_date"]).map(|d| d.replacen(' ', "T", 1)),
        rv: Some(Arc::new(RvFile { download_url: hosted, external_url: external, page_url: page })),
        ..Default::default()
    })
}

/// Charts per catalogue page. RhythmVerse only pages up to ~249 pages, so large pages are
/// needed to cover the whole Clone Hero section (~134k charts → ~192 pages).
pub const CATALOG_PAGE: u32 = 700;

/// One page of the whole Clone Hero section, newest-changed first (for the local catalogue).
pub async fn fetch_catalog_page(page: u32) -> Result<(Vec<Chart>, u64), String> {
    let form: Vec<(String, String)> = vec![
        ("data_type".into(), "full".into()),
        ("records".into(), CATALOG_PAGE.to_string()),
        ("page".into(), page.to_string()),
        ("sort[0][sort_by]".into(), "update_date".into()),
        ("sort[0][sort_order]".into(), "DESC".into()),
    ];
    let resp = CLIENT
        .post(format!("{BASE}/api/ch/songfiles/list"))
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(90))
        .body(url_encode(&form))
        .send()
        .await
        .map_err(|e| format!("RhythmVerse: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("RhythmVerse returned {}", resp.status()));
    }
    let json: Value = resp.json().await.map_err(|_| "RhythmVerse sent an unexpected reply".to_string())?;
    if json["status"] != "success" {
        return Err("RhythmVerse sent an unexpected reply".into());
    }
    let total = int(&json["data"]["records"]["total_filtered"]).unwrap_or(0).max(0) as u64;
    let charts = json["data"]["songs"].as_array().into_iter().flatten().filter_map(to_chart).collect();
    Ok((charts, total))
}

/// Our instrument keys → RhythmVerse's `instrument[]` filter values.
fn rv_instrument(i: &str) -> Option<&'static str> {
    Some(match i {
        "guitar" => "guitar",
        "bass" => "bass",
        "drums" => "drums",
        "keys" => "keys",
        "rhythm" => "rhythm",
        "guitarcoop" => "guitar_coop",
        "guitarghl" => "guitarghl",
        "bassghl" => "bassghl",
        _ => return None,
    })
}

/// One page of RhythmVerse's Clone Hero section: a search, or the newest charts when `query`
/// is empty.
pub async fn search(
    query: String,
    page: u32,
    sort: Option<Sort>,
    instrument: Option<String>,
    difficulty: Option<String>,
) -> Result<RvPage, String> {
    let q = query.trim().to_string();
    let endpoint = if q.is_empty() { "list" } else { "search/live" };
    let mut form: Vec<(String, String)> = vec![
        ("data_type".into(), "full".into()),
        ("records".into(), PER_PAGE.to_string()),
        ("page".into(), page.to_string()),
    ];
    if !q.is_empty() {
        form.push(("text".into(), q));
    }
    if let Some(i) = instrument.as_deref().and_then(rv_instrument) {
        form.push(("instrument[]".into(), i.into()));
    }
    if let Some(d) = difficulty.as_deref() {
        let code = match d {
            "expert" => "x",
            "hard" => "h",
            "medium" => "m",
            _ => "e",
        };
        form.push(("difficulties[]".into(), code.into()));
    }
    if let Some(sort) = sort {
        let field = match sort.key {
            SortKey::Name => Some("title"),
            SortKey::Artist => Some("artist"),
            SortKey::Length => Some("length"),
            _ => None,
        };
        if let Some(f) = field {
            form.push(("sort[0][sort_by]".into(), f.into()));
            form.push(("sort[0][sort_order]".into(), if sort.direction == SortDir::Asc { "ASC" } else { "DESC" }.into()));
        }
    } else {
        // Newest first, so results can be interleaved with Encore's by date.
        form.push(("sort[0][sort_by]".into(), "update_date".into()));
        form.push(("sort[0][sort_order]".into(), "DESC".into()));
    }
    let body = url_encode(&form);
    let resp = CLIENT
        .post(format!("{BASE}/api/ch/songfiles/{endpoint}"))
        // Their endpoint only answers requests marked like this (as their own page sends).
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(20))
        .body(body)
        .send()
        .await
        .map_err(|e| format!("RhythmVerse search failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("RhythmVerse search failed: server returned {}", resp.status()));
    }
    let json: Value = resp.json().await.map_err(|_| "RhythmVerse sent an unexpected reply".to_string())?;
    if json["status"] != "success" {
        return Err("RhythmVerse sent an unexpected reply".into());
    }
    let total = int(&json["data"]["records"]["total_filtered"]).unwrap_or(0).max(0) as u64;
    let charts = json["data"]["songs"].as_array().into_iter().flatten().filter_map(to_chart).map(Arc::new).collect();
    Ok(RvPage { charts, total })
}

fn url_encode(form: &[(String, String)]) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                b' ' => "+".into(),
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    form.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

/// Percent-encodes the path of a RhythmVerse download link (file names contain spaces etc.).
fn encode_url(u: &str) -> String {
    u.chars()
        .map(|c| match c {
            ' ' => "%20".to_string(),
            '#' => "%23".to_string(),
            '[' => "%5B".to_string(),
            ']' => "%5D".to_string(),
            c if c.is_ascii() => c.to_string(),
            c => c.to_string().bytes().map(|b| format!("%{b:02X}")).collect(),
        })
        .collect()
}

/// True if Truss can download this chart (hosted on RhythmVerse, Google Drive or MediaFire).
pub fn can_download(rv: &RvFile) -> bool {
    rv.download_url.is_some() || rv.external_url.as_deref().is_some_and(|u| host_kind(u) != Host::Other)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Host {
    DriveFile,
    DriveFolder,
    MediaFire,
    Other,
}

fn host_kind(u: &str) -> Host {
    let l = u.to_ascii_lowercase();
    if l.contains("drive.google.com") || l.contains("docs.google.com") || l.contains("drive.usercontent.google.com") {
        if l.contains("/folders/") { Host::DriveFolder } else { Host::DriveFile }
    } else if l.contains("mediafire.com") {
        Host::MediaFire
    } else {
        Host::Other
    }
}

/// The host name shown when a link can't be downloaded in the app.
pub fn host_name(u: &str) -> String {
    u.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or(u).trim_start_matches("www.").to_string()
}

fn drive_id(u: &str) -> Option<String> {
    for marker in ["/file/d/", "/folders/", "id="] {
        if let Some(i) = u.find(marker) {
            let id: String = u[i + marker.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

/// Downloads a chart (from wherever it's hosted) and installs its chart folder at `staged`.
pub async fn download(
    rv: &RvFile,
    staged: &Path,
    library: &Path,
    tag: &str,
    keep_video: bool,
    tx: &mut Sender<crate::download::Progress>,
) -> Result<(), String> {
    use crate::download::{Progress, TempGuard};
    let work = library.join(format!(".truss-tmp-{tag}-{}", std::process::id()));
    let _guard = TempGuard(Some(work.clone()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("Cannot write to library: {e}"))?;
    let unpacked = work.join("chart");

    match (&rv.download_url, rv.external_url.as_deref().map(|u| (u, host_kind(u)))) {
        (Some(url), _) => {
            let _ = tx.send(Progress::Status("Downloading from RhythmVerse...".into())).await;
            get_archive(&encode_url(url), &work, &unpacked, keep_video, "chart.zip", tx).await?;
        }
        (None, Some((u, Host::DriveFile))) => {
            let id = drive_id(u).ok_or("Couldn't read the Google Drive link")?;
            let _ = tx.send(Progress::Status("Downloading from Google Drive...".into())).await;
            let url = format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t");
            get_archive(&url, &work, &unpacked, keep_video, "chart.bin", tx).await?;
        }
        (None, Some((u, Host::DriveFolder))) => {
            let id = drive_id(u).ok_or("Couldn't read the Google Drive link")?;
            let _ = tx.send(Progress::Status("Reading the Google Drive folder...".into())).await;
            download_drive_folder(&id, &unpacked, keep_video, 0, tx).await?;
            // Some folders hold an archive rather than loose chart files.
            if find_chart_folder(&unpacked).is_none()
                && let Some(archive) = first_archive(&unpacked)
            {
                let inner = work.join("inner");
                unpack_any(archive, &inner, keep_video).await?;
                let _ = std::fs::remove_dir_all(&unpacked);
                std::fs::rename(&inner, &unpacked).map_err(|e| e.to_string())?;
            }
        }
        (None, Some((u, Host::MediaFire))) => {
            let _ = tx.send(Progress::Status("Finding the MediaFire download...".into())).await;
            let url = mediafire_link(u).await?;
            get_archive(&url, &work, &unpacked, keep_video, "chart.bin", tx).await?;
        }
        (None, Some((u, Host::Other))) => {
            return Err(format!("This chart is hosted on {}, which Truss can't download from. Open the link to get it.", host_name(u)));
        }
        (None, None) => return Err("RhythmVerse has no download for this chart.".into()),
    }

    let folder = find_chart_folder(&unpacked).ok_or("No Clone Hero chart was found in the download.")?;
    let _ = tx.send(Progress::Status("Moving into library...".into())).await;
    crate::download::move_path(&folder, staged)
}

/// Gets a chart archive into `unpacked`. With videos off, a .zip is fetched without its video
/// (only the needed byte ranges); anything else is downloaded whole and unpacked.
async fn get_archive(
    url: &str,
    work: &Path,
    unpacked: &Path,
    keep_video: bool,
    fallback_name: &str,
    tx: &mut Sender<crate::download::Progress>,
) -> Result<(), String> {
    if !keep_video && crate::zip_ranges::fetch_selected(url, unpacked, |n| safe_rel(n, false), tx).await? {
        return Ok(());
    }
    let file = fetch_to_dir(url, work, fallback_name, tx).await?;
    unpack_any(file, unpacked, keep_video).await
}

/// Streams `url` into `dir`, named from the server's file name (or `fallback`). Returns the file.
async fn fetch_to_dir(url: &str, dir: &Path, fallback: &str, tx: &mut Sender<crate::download::Progress>) -> Result<PathBuf, String> {
    use crate::download::Progress;
    use std::io::Write;
    let resp = CLIENT
        .get(url)
        .timeout(std::time::Duration::from_secs(60 * 30))
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            404 | 410 => "The file is no longer on the host (it was moved or deleted). Try another version of this song.".into(),
            401 | 403 => "The host refused the download (the file may be private). Open the link in your browser.".into(),
            429 => "The host is limiting downloads right now. Wait a bit and try again.".into(),
            c => format!("Download failed: the host returned {c}"),
        });
    }
    let headers = resp.headers();
    if headers.get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("text/html")) {
        return Err("Google Drive didn't hand over the file (it may be over its download limit or need a sign-in). Open the link in your browser.".into());
    }
    let name = headers
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(file_name_from_disposition)
        .map(|n| crate::download::sanitize_filename(&n))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| fallback.to_string());
    let total = resp.content_length();
    let path = dir.join(name);
    let mut out = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).map_err(|e| format!("Cannot write to library: {e}"))?);
    let mut stream = resp.bytes_stream();
    let mut done = 0u64;
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Connection lost: {e}"))?;
        done += chunk.len() as u64;
        out.write_all(&chunk).map_err(|e| format!("Write failed: {e}"))?;
        if last.elapsed() >= std::time::Duration::from_millis(33) {
            last = std::time::Instant::now();
            let _ = tx.try_send(Progress::Bytes { done, total });
        }
    }
    let _ = tx.send(Progress::Bytes { done, total }).await;
    out.flush().map_err(|e| format!("Write failed: {e}"))?;
    if let Some(t) = total
        && done < t
    {
        return Err(format!(
            "The download was cut off by the server ({:.0} of {:.0} MB). Try again.",
            done as f64 / 1_048_576.0,
            t as f64 / 1_048_576.0
        ));
    }
    Ok(path)
}

fn file_name_from_disposition(cd: &str) -> Option<String> {
    if let Some(i) = cd.find("filename*=") {
        let v = cd[i + 10..].split(';').next()?.trim().trim_matches('"');
        let v = v.strip_prefix("UTF-8''").or_else(|| v.strip_prefix("utf-8''")).unwrap_or(v);
        return Some(percent_decode(v));
    }
    let i = cd.find("filename=")?;
    Some(cd[i + 9..].split(';').next()?.trim().trim_matches('"').to_string())
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lists a public Google Drive folder by reading its web page (there's no other way without a
/// Google API key). Files are matched to their names by order on the page.
async fn list_drive_folder(id: &str) -> Result<Vec<(String, String, bool)>, String> {
    let html = CLIENT
        .get(format!("https://drive.google.com/drive/folders/{id}"))
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Couldn't open the Google Drive folder: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    // Positions of each distinct data-id="..." in page order.
    let mut groups: Vec<(String, usize)> = Vec::new();
    let mut at = 0;
    while let Some(i) = html[at..].find("data-id=\"") {
        let start = at + i + 9;
        let Some(len) = html[start..].find('"') else { break };
        let found = &html[start..start + len];
        if found != "_gd" && groups.last().is_none_or(|g| g.0 != found) {
            groups.push((found.to_string(), at + i));
        }
        at = start + len;
    }
    let mut entries = Vec::new();
    for (n, (fid, pos)) in groups.iter().enumerate() {
        let end = groups.get(n + 1).map(|g| g.1).unwrap_or(html.len());
        let slice = &html[*pos..end];
        let label = slice.find("aria-label=\"").and_then(|i| {
            let s = &slice[i + 12..];
            s.find('"').map(|e| html_unescape(&s[..e]))
        });
        let label = label.unwrap_or_else(|| fid.clone());
        // Labels look like "song.ini" or "song.ini Text"; folders have no extension.
        let name = label
            .split_whitespace()
            .scan(String::new(), |acc, w| {
                if !acc.is_empty() {
                    acc.push(' ');
                }
                acc.push_str(w);
                Some(acc.clone())
            })
            .filter(|cand| has_extension(cand))
            .last()
            .unwrap_or(label.clone());
        let is_folder = !has_extension(&name);
        let name = if is_folder {
            label.trim_end_matches(" Shared folder").trim_end_matches(" Folder").to_string()
        } else {
            name
        };
        entries.push((fid.clone(), crate::download::sanitize_filename(&name), is_folder));
    }
    if entries.is_empty() {
        return Err("The Google Drive folder is empty or not public. Open the link in your browser.".into());
    }
    Ok(entries)
}

fn has_extension(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && (2..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">")
}

fn download_drive_folder<'a>(
    id: &'a str,
    dest: &'a Path,
    keep_video: bool,
    depth: u32,
    tx: &'a mut Sender<crate::download::Progress>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
        use crate::download::Progress;
        if depth > 3 {
            return Ok(());
        }
        let entries = list_drive_folder(id).await?;
        std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
        let files: Vec<_> = entries.iter().filter(|e| !e.2).collect();
        for (n, (fid, name, _)) in files.iter().enumerate() {
            let lower = name.to_ascii_lowercase();
            if lower == "desktop.ini" || (!keep_video && lower.starts_with("video.")) {
                continue;
            }
            let _ = tx.send(Progress::Status(format!("Downloading {name} ({}/{})", n + 1, files.len()))).await;
            let url = format!("https://drive.usercontent.google.com/download?id={fid}&export=download&confirm=t");
            let got = fetch_to_dir(&url, dest, name, tx).await?;
            // Keep Drive's listed name (the server's may differ in case/characters).
            let want = dest.join(name);
            if got != want {
                let _ = std::fs::rename(&got, &want);
            }
        }
        for (fid, name, _) in entries.iter().filter(|e| e.2) {
            download_drive_folder(fid, &dest.join(name), keep_video, depth + 1, tx).await?;
        }
        Ok(())
    })
}

/// MediaFire shows a page with the real download link on it.
async fn mediafire_link(page: &str) -> Result<String, String> {
    let html = CLIENT
        .get(page)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Couldn't open the MediaFire page: {e}"))?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let mut at = 0;
    while let Some(i) = html[at..].find("download") {
        let pos = at + i;
        // Look for "https://downloadNNNN.mediafire.com/..." around here.
        let start = html[..pos].rfind("https://").filter(|s| pos - s < 12);
        if let Some(s) = start {
            let end = html[s..].find(['"', '\'', ' ', '<']).map(|e| s + e).unwrap_or(html.len());
            let link = html_unescape(&html[s..end]);
            if link.contains(".mediafire.com/") && link.split('/').nth(2).is_some_and(|h| h.starts_with("download")) {
                return Ok(link);
            }
        }
        at = pos + 8;
    }
    Err("Couldn't find the MediaFire download link. Open the link in your browser.".into())
}

fn first_archive(dir: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    rd.flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| ["zip", "7z", "sng"].contains(&e.to_string_lossy().to_ascii_lowercase().as_str())))
}

/// Unpacks a downloaded .zip / .7z (or places a .sng's files) into `dest`.
async fn unpack_any(file: PathBuf, dest: &Path, keep_video: bool) -> Result<(), String> {
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        let mut magic = [0u8; 6];
        {
            use std::io::Read;
            let _ = std::fs::File::open(&file).and_then(|mut f| f.read(&mut magic));
        }
        // The file's first bytes decide; the name is only a fallback (a download saved under a
        // made-up name like "chart.zip" can really be a .7z).
        let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let kind = if magic.starts_with(b"PK") {
            "zip"
        } else if magic.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
            "7z"
        } else if magic.starts_with(b"SNGPKG") {
            "sng"
        } else if magic.starts_with(b"Rar!") {
            "rar"
        } else {
            ext.as_str()
        };
        match kind {
            "zip" => unzip(&file, &dest, keep_video),
            "7z" => un7z(&file, &dest, keep_video),
            "sng" => unsng(&file, &dest),
            "rar" => unrar_all(&file, &dest, keep_video),
            _ => Err(format!("Truss doesn't know how to open \"{}\".", file.file_name().unwrap_or_default().to_string_lossy())),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A safe relative path for an archive entry (no "..", no absolute paths), or `None` to skip it.
fn safe_rel(name: &str, keep_video: bool) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            p if p.contains(':') => return None,
            "__MACOSX" => return None,
            p => out.push(p),
        }
    }
    let file = out.file_name()?.to_string_lossy().to_ascii_lowercase();
    if file == "desktop.ini" || (!keep_video && file.starts_with("video.")) {
        return None;
    }
    Some(out)
}

fn write_entry(dest: &Path, rel: &Path, reader: &mut dyn std::io::Read) -> Result<(), String> {
    let out = dest.join(rel);
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let mut w = std::io::BufWriter::new(std::fs::File::create(&out).map_err(|e| format!("Couldn't unpack: {e}"))?);
    std::io::copy(reader, &mut w).map_err(|e| format!("Couldn't unpack: {e}"))?;
    Ok(())
}

fn unzip(file: &Path, dest: &Path, keep_video: bool) -> Result<(), String> {
    let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| format!("The download isn't a valid .zip: {e}"))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| format!("Damaged .zip: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().map(|n| n.into_owned()).unwrap_or_default();
        let Some(rel) = safe_rel(&name, keep_video) else { continue };
        write_entry(dest, &rel, &mut entry)?;
    }
    Ok(())
}

fn un7z(file: &Path, dest: &Path, keep_video: bool) -> Result<(), String> {
    let mut reader = sevenz_rust2::ArchiveReader::open(file, sevenz_rust2::Password::empty())
        .map_err(|e| format!("The download isn't a valid .7z: {e}"))?;
    let mut err = None;
    reader
        .for_each_entries(|entry, data| {
            if entry.is_directory() {
                return Ok(true);
            }
            if let Some(rel) = safe_rel(entry.name(), keep_video)
                && let Err(e) = write_entry(dest, &rel, data)
            {
                err = Some(e);
                return Ok(false);
            }
            Ok(true)
        })
        .map_err(|e| format!("Couldn't unpack the .7z: {e}"))?;
    err.map_or(Ok(()), Err)
}

fn unrar_all(file: &Path, dest: &Path, keep_video: bool) -> Result<(), String> {
    let mut archive = unrar::Archive::new(file)
        .open_for_processing()
        .map_err(|e| format!("The download isn't a valid .rar: {e}"))?;
    while let Some(header) = archive.read_header().map_err(|e| format!("Couldn't unpack the .rar: {e}"))? {
        let (is_file, name) = {
            let entry = header.entry();
            (entry.is_file(), entry.filename.to_string_lossy().into_owned())
        };
        archive = match safe_rel(&name, keep_video).filter(|_| is_file) {
            Some(rel) => {
                let (data, rest) = header.read().map_err(|e| format!("Couldn't unpack the .rar: {e}"))?;
                write_entry(dest, &rel, &mut data.as_slice())?;
                rest
            }
            None => header.skip().map_err(|e| format!("Couldn't unpack the .rar: {e}"))?,
        };
    }
    Ok(())
}

fn unsng(file: &Path, dest: &Path) -> Result<(), String> {
    use std::io::Read;
    let mut x = crate::sng::SngExtractor::new(dest);
    let mut f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        x.feed(&buf[..n])?;
    }
    if x.is_done() { Ok(()) } else { Err("The .sng file is incomplete".into()) }
}

fn find_chart_folder(dir: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            subdirs.push(e.path());
        } else if n == "song.ini" || n == "notes.chart" || n == "notes.mid" {
            return Some(dir.to_path_buf());
        }
    }
    subdirs.into_iter().find_map(|d| find_chart_folder(&d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_results() {
        let song: Value = serde_json::from_str(r#"{
            "data": {"title": "Home", "artist": "Hue", "album": "A", "year": 2026, "song_length": 205, "diff_guitar": "3", "diff_keys": "0"},
            "file": {"file_id": "12e0e0a7", "gameformat": "rv", "download_url": "/download_file/x/12e0e0a7/HUE - Home.zip",
                     "file_url_full": "https://rhythmverse.co/songfile/12e0e0a7", "album_art": "/assets/album_art/x/a.png",
                     "author": {"name": "Miggy_RG"}, "diff_drums": 5, "update_date": "2026-11-08 01:33:03"}
        }"#).unwrap();
        let c = to_chart(&song).unwrap();
        assert_eq!(c.name.as_deref(), Some("Home"));
        assert_eq!(c.charter.as_deref(), Some("Miggy_RG"));
        assert_eq!(c.song_length, Some(205_000));
        assert_eq!((c.diff_guitar, c.diff_drums, c.diff_keys), (Some(3), Some(5), None));
        assert_eq!(c.album_art_md5.as_deref(), Some("https://rhythmverse.co/assets/album_art/x/a.png"));
        let rv = c.rv.unwrap();
        assert_eq!(rv.download_url.as_deref(), Some("https://rhythmverse.co/download_file/x/12e0e0a7/HUE - Home.zip"));
        assert_eq!(encode_url(rv.download_url.as_deref().unwrap()), "https://rhythmverse.co/download_file/x/12e0e0a7/HUE%20-%20Home.zip");

        // Linked-only charts aren't downloaded in the app; Rock Band files are skipped.
        let ext: Value = serde_json::from_str(r#"{"data": {"title": "T"}, "file": {"file_id": "1", "gameformat": "ch", "download_url": "https://drive.google.com/file/d/x", "external_url": "https://drive.google.com/file/d/x"}}"#).unwrap();
        let e = to_chart(&ext).unwrap();
        assert!(e.rv.as_ref().unwrap().download_url.is_none() && e.rv.as_ref().unwrap().external_url.is_some());
        let con: Value = serde_json::from_str(r#"{"data": {}, "file": {"file_id": "2", "gameformat": "rb3xbox"}}"#).unwrap();
        assert!(to_chart(&con).is_none());
    }

    #[test]
    fn links_and_paths() {
        assert_eq!(drive_id("https://drive.google.com/file/d/1QjXY-C_h/view?usp=sharing").as_deref(), Some("1QjXY-C_h"));
        assert_eq!(drive_id("https://drive.google.com/drive/u/0/folders/1b5Bq?usp=x").as_deref(), Some("1b5Bq"));
        assert_eq!(drive_id("https://drive.google.com/open?id=abc_9").as_deref(), Some("abc_9"));
        assert_eq!(host_kind("https://drive.google.com/drive/folders/x"), Host::DriveFolder);
        assert_eq!(host_kind("https://www.mediafire.com/file/x/y.zip/file"), Host::MediaFire);
        assert_eq!(host_kind("https://mega.nz/file/x"), Host::Other);
        assert_eq!(host_name("https://www.mediafire.com/file/x"), "mediafire.com");
        assert_eq!(safe_rel("Song/notes.chart", true), Some(PathBuf::from("Song").join("notes.chart")));
        assert_eq!(safe_rel("../evil.txt", true), None);
        assert_eq!(safe_rel("C:/evil.txt", true), None);
        assert_eq!(safe_rel("Song/video.mp4", false), None);
        assert_eq!(safe_rel("__MACOSX/._x", true), None);
        assert!(has_extension("song.ini") && has_extension("notes.chart") && !has_extension("Everlong (Foo Fighters)"));
        assert_eq!(file_name_from_disposition("attachment; filename=\"Foo - Bar.7z\"").as_deref(), Some("Foo - Bar.7z"));
        assert_eq!(file_name_from_disposition("attachment; filename*=UTF-8''A%20B.zip").as_deref(), Some("A B.zip"));
    }

    /// Network test (TRUSS_TEST_RV_DL=<dir>): a .7z hosted on RhythmVerse downloads and previews.
    #[test]
    fn live_7z_from_rhythmverse() {
        // A .7z on RhythmVerse is saved as "chart.zip" (the server sends no file name); it must
        // still be opened as a .7z, then load as a preview.
        let Some(dir) = std::env::var_os("TRUSS_TEST_RV_DL") else { return };
        let dir = PathBuf::from(dir);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let p = search("Blvd. Nights".into(), 1, None, None, None).await.unwrap();
            let c = p
                .charts
                .into_iter()
                .find(|c| c.rv.as_ref().and_then(|r| r.download_url.as_deref()).is_some_and(|u| u.ends_with(".7z")))
                .expect("a .7z on RhythmVerse");
            let (mut tx, mut rx) = futures::channel::mpsc::channel(256);
            tokio::spawn(async move { while rx.next().await.is_some() {} });
            let staged = dir.join("7z-test");
            let _ = std::fs::remove_dir_all(&staged);
            download(c.rv.as_ref().unwrap(), &staged, &dir, "7z", false, &mut tx).await.unwrap();
            let files: Vec<String> =
                std::fs::read_dir(&staged).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect();
            println!("{files:?}");
            assert!(files.iter().any(|f| f.ends_with(".chart") || f.ends_with(".mid")));
            let loaded = crate::preview::rv_cache::load_folder(&staged, -1.0).unwrap();
            assert!(!loaded.parsed.tracks.is_empty());
        });
    }

    /// Network test: TRUSS_TEST_RV_DL=<dir> downloads the first downloadable "everlong" result
    /// of each kind (RhythmVerse-hosted, Drive file, Drive folder) into that scratch folder.
    #[test]
    fn live_downloads() {
        let Some(dir) = std::env::var_os("TRUSS_TEST_RV_DL") else { return };
        let dir = PathBuf::from(dir);
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut picked: Vec<(&str, Arc<Chart>)> = Vec::new();
            for q in ["everlong", "metallica", "the warning"] {
                let p = search(q.into(), 1, None, None, None).await.unwrap();
                for c in p.charts {
                    let rv = c.rv.clone().unwrap();
                    let kind = match (&rv.download_url, rv.external_url.as_deref().map(host_kind)) {
                        (Some(_), _) => "rhythmverse",
                        (None, Some(Host::DriveFile)) => "drive-file",
                        (None, Some(Host::DriveFolder)) => "drive-folder",
                        _ => continue,
                    };
                    if !picked.iter().any(|(k, _)| *k == kind) {
                        picked.push((kind, c));
                    }
                }
            }
            for (kind, c) in picked {
                let (mut tx, mut rx) = futures::channel::mpsc::channel(256);
                tokio::spawn(async move { while rx.next().await.is_some() {} });
                let staged = dir.join(format!("{kind} - {}", crate::download::sanitize_filename(c.name.as_deref().unwrap_or("?"))));
                let t = std::time::Instant::now();
                let r = download(c.rv.as_ref().unwrap(), &staged, &dir, kind, false, &mut tx).await;
                let files: Vec<String> = std::fs::read_dir(&staged)
                    .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                    .unwrap_or_default();
                println!("{kind}: {:?} in {:?} -> {r:?} {files:?}", c.name, t.elapsed());
            }
        });
    }

    /// Network test: TRUSS_TEST_RV=1 runs one real search.
    #[test]
    fn live_search() {
        if std::env::var("TRUSS_TEST_RV").is_err() {
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let p = rt.block_on(search("everlong".into(), 1, None, None, None)).unwrap();
        let hosted = p.charts.iter().filter(|c| c.rv.as_ref().unwrap().download_url.is_some()).count();
        println!("{} total, {} on this page, {} hosted on RhythmVerse", p.total, p.charts.len(), hosted);
        assert!(!p.charts.is_empty());
    }
}

