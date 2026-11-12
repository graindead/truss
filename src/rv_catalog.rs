//! Local copy of RhythmVerse's Clone Hero chart list (like Chart Manager's catalogue), so
//! RhythmVerse searches are instant and don't depend on its slow server.
//!
//! - First sync: pages through the whole Clone Hero section, newest-changed first, one request
//!   at a time with a short pause (resumable if Truss is closed part way).
//! - Later syncs: only fetch pages until they reach charts older than the last sync.
//! - Stored on disk in SQLite (%LOCALAPPDATA%\truss\cache\rhythmverse.sqlite), not in memory.

use crate::api::{Chart, Sort, SortDir, SortKey};
use crate::rhythmverse::{self, RvFile};
use futures::{SinkExt, channel::mpsc::Sender};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{Arc, Mutex};

/// Pause between catalogue pages, to keep the load on RhythmVerse low.
const PAGE_PAUSE: std::time::Duration = std::time::Duration::from_millis(1500);
/// How far back a delta sync re-checks (covers clock differences and edits during a sync).
const OVERLAP_SECS: i64 = 24 * 3600;

static DB: Mutex<Option<Connection>> = Mutex::new(None);

fn db_path() -> Option<std::path::PathBuf> {
    crate::settings::project_dirs().map(|d| d.cache_dir().join("rhythmverse.sqlite"))
}

/// The database and its WAL files.
fn delete_files(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(p));
    }
}

fn is_corrupt(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _)
        if matches!(f.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase))
}

/// How long to wait when another copy of Truss is using the database.
#[cfg(not(test))]
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
#[cfg(test)]
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);

/// The backup made after each successful update (`backup`).
fn backup_path(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension("backup.sqlite")
}

fn connect(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(BUSY_TIMEOUT)?;
    Ok(c)
}

/// Whether a database file opens and passes SQLite's own check. Err = couldn't tell (for
/// example another copy of Truss has it locked), which is never treated as damage.
fn check(c: &Connection) -> Result<bool, rusqlite::Error> {
    match c.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)) {
        Ok(v) => Ok(v == "ok"),
        Err(e) if is_corrupt(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Opens the database. Only if SQLite itself reports damage is it replaced: with the backup
/// from the last successful update when there is one (then an update fetches just what's new),
/// otherwise with an empty one (it's only a copy of RhythmVerse's list).
fn open(path: &std::path::Path) -> Result<Connection, String> {
    let conn = connect(path).map_err(|e| format!("Couldn't open the local chart list: {e}"))?;
    match check(&conn) {
        Ok(true) => return Ok(conn),
        Ok(false) => {}
        Err(e) => return Err(format!("The local chart list is busy ({e}). Try again in a moment.")),
    }
    drop(conn);
    restore(path)
}

/// Replaces a damaged database with the backup (or nothing).
fn restore(path: &std::path::Path) -> Result<Connection, String> {
    delete_files(path);
    let backup = backup_path(path);
    let backup_ok = backup.is_file() && connect(&backup).ok().is_some_and(|c| check(&c).is_ok_and(|ok| ok));
    if backup_ok {
        std::fs::copy(&backup, path).map_err(|e| format!("Couldn't restore the chart list backup: {e}"))?;
    } else {
        let _ = std::fs::remove_file(&backup);
    }
    connect(path).map_err(|e| e.to_string())
}

/// Saves a compact copy of the database to restore from if it's ever damaged. Written to a
/// temporary file first, so a crash part-way can't spoil the previous backup.
fn backup() -> Result<(), String> {
    let path = db_path().ok_or("No cache folder available")?;
    let target = backup_path(&path);
    let tmp = path.with_extension("backup.tmp");
    let _ = std::fs::remove_file(&tmp);
    with_db(|c| c.execute("VACUUM INTO ?1", [tmp.to_string_lossy()]).map(|_| ()))?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())
}

fn with_db<T>(f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
    let mut guard = DB.lock().unwrap();
    if guard.is_none() {
        let path = db_path().ok_or("No cache folder available")?;
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let conn = open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS charts (
                uid TEXT PRIMARY KEY, modified TEXT NOT NULL,
                name TEXT, artist TEXT, album TEXT, genre TEXT, year TEXT, charter TEXT,
                length_ms INTEGER,
                d_guitar INTEGER, d_guitar_coop INTEGER, d_rhythm INTEGER, d_bass INTEGER,
                d_drums INTEGER, d_keys INTEGER, d_guitarghl INTEGER, d_bassghl INTEGER, d_vocals INTEGER,
                art TEXT, download_url TEXT, external_url TEXT, page_url TEXT);
             CREATE INDEX IF NOT EXISTS charts_modified ON charts(modified DESC);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .map_err(|e| e.to_string())?;
        *guard = Some(conn);
    }
    match f(guard.as_ref().unwrap()) {
        Ok(v) => Ok(v),
        Err(e) if is_corrupt(&e) => {
            // Damaged while open: put the backup (or an empty copy) in its place.
            *guard = None;
            if let Some(path) = db_path() {
                let _ = restore(&path);
            }
            Err(CORRUPT.into())
        }
        Err(e) => Err(e.to_string()),
    }
}

const CORRUPT: &str = "The local chart list was damaged, so it was replaced with its backup";

fn get_meta(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [key, value])?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct Status {
    pub charts: u64,
    /// The first full copy has finished, so searches can use it.
    pub ready: bool,
    /// Unix seconds of the last finished sync.
    pub last_sync: Option<i64>,
}

pub fn status() -> Status {
    with_db(|c| {
        let charts: i64 = c.query_row("SELECT COUNT(*) FROM charts", [], |r| r.get(0))?;
        Ok(Status {
            charts: charts as u64,
            ready: get_meta(c, "full_done")?.as_deref() == Some("1"),
            last_sync: get_meta(c, "last_sync")?.and_then(|v| v.parse().ok()),
        })
    })
    .unwrap_or(Status { charts: 0, ready: false, last_sync: None })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "YYYY-MM-DDTHH:MM:SS" for unix seconds (UTC), to compare with RhythmVerse's dates.
fn iso(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.format("%Y-%m-%dT%H:%M:%S").to_string()).unwrap_or_default()
}

#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// Pages fetched so far and the total (a full copy reports this; quick updates don't).
    Progress { page: u32, pages: u32 },
    Finished(Result<u64, String>),
}

/// Brings the local catalogue up to date. Full copy the first time (resuming where a previous
/// run stopped), otherwise only what changed since the last sync.
pub async fn sync(mut tx: Sender<SyncEvent>) {
    let mut result = run_sync(&mut tx).await;
    if result.as_ref().is_err_and(|e| e == CORRUPT) {
        // Restored from the backup (or cleared): update again from there.
        result = run_sync(&mut tx).await;
    }
    if result.is_ok() {
        // Off the UI thread: it rewrites ~40 MB.
        let _ = tokio::task::spawn_blocking(backup).await;
    }
    let _ = tx.send(SyncEvent::Finished(result)).await;
}

async fn run_sync(tx: &mut Sender<SyncEvent>) -> Result<u64, String> {
    let (full_done, resume_page, cursor) = with_db(|c| {
        Ok((
            get_meta(c, "full_done")?.as_deref() == Some("1"),
            get_meta(c, "build_page")?.and_then(|v| v.parse::<u32>().ok()).unwrap_or(1),
            get_meta(c, "last_sync")?.and_then(|v| v.parse::<i64>().ok()),
        ))
    })?;
    let started = now_secs();
    let full = !full_done;
    let stop_before = cursor.map(|c| iso(c - OVERLAP_SECS));
    let mut page = if full { resume_page.max(1) } else { 1 };
    let mut pages = u32::MAX;
    let mut added = 0u64;
    loop {
        let (charts, total) = fetch_with_retry(page).await?;
        pages = (total.div_ceil(rhythmverse::CATALOG_PAGE as u64) as u32).max(1).min(pages);
        if charts.is_empty() {
            break;
        }
        let all_old = stop_before.as_ref().is_some_and(|stop| {
            charts.iter().all(|c| c.modified_time.as_deref().is_some_and(|m| m < stop.as_str()))
        });
        added += charts.len() as u64;
        let next = page + 1;
        let save = charts;
        tokio::task::spawn_blocking(move || store(&save, if full { Some(next) } else { None }))
            .await
            .map_err(|e| e.to_string())??;
        if full {
            let _ = tx.send(SyncEvent::Progress { page, pages }).await;
        }
        if (!full && all_old) || page >= pages {
            break;
        }
        page = next;
        tokio::time::sleep(PAGE_PAUSE).await;
    }
    with_db(|c| {
        set_meta(c, "full_done", "1")?;
        set_meta(c, "build_page", "1")?;
        set_meta(c, "last_sync", &started.to_string())
    })?;
    Ok(added)
}

async fn fetch_with_retry(page: u32) -> Result<(Vec<Chart>, u64), String> {
    let mut last = String::new();
    for attempt in 0..3u64 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(2 * attempt + 1)).await;
        }
        match rhythmverse::fetch_catalog_page(page).await {
            Ok(r) => return Ok(r),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Saves a page of charts (replacing older copies), and where a full copy should resume.
fn store(charts: &[Chart], resume_at: Option<u32>) -> Result<(), String> {
    let mut guard = DB.lock().unwrap();
    let conn = guard.as_mut().ok_or("Catalogue isn't open")?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    {
        let mut stmt = tx
            .prepare_cached(
                "INSERT OR REPLACE INTO charts VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
            )
            .map_err(|e| e.to_string())?;
        for c in charts {
            let Some(rv) = &c.rv else { continue };
            stmt.execute(params![
                c.md5.trim_start_matches("rv-"),
                c.modified_time.clone().unwrap_or_default(),
                c.name,
                c.artist,
                c.album,
                c.genre,
                c.year,
                c.charter,
                c.song_length,
                c.diff_guitar,
                c.diff_guitar_coop,
                c.diff_rhythm,
                c.diff_bass,
                c.diff_drums,
                c.diff_keys,
                c.diff_guitarghl,
                c.diff_bassghl,
                c.diff_vocals,
                c.album_art_md5,
                rv.download_url,
                rv.external_url,
                rv.page_url,
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    if let Some(p) = resume_at {
        set_meta(&tx, "build_page", &p.to_string()).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Searches the local catalogue: every word must appear in the name, artist, album or charter.
/// Empty query lists the newest charts. Returns (page of charts, total matches).
pub fn search(
    query: &str,
    sort: Option<Sort>,
    instrument: Option<&str>,
    page: u32,
    per_page: u32,
) -> Result<(Vec<Arc<Chart>>, u64), String> {
    let words: Vec<String> = query.split_whitespace().map(|w| format!("%{}%", w.replace(['%', '_'], ""))).collect();
    let mut sql_where = Vec::new();
    for i in 0..words.len() {
        let n = i + 1;
        sql_where.push(format!("(name LIKE ?{n} OR artist LIKE ?{n} OR album LIKE ?{n} OR charter LIKE ?{n})"));
    }
    if let Some(col) = instrument.and_then(diff_column) {
        sql_where.push(format!("{col} > 0"));
    }
    let where_sql = if sql_where.is_empty() { String::new() } else { format!("WHERE {}", sql_where.join(" AND ")) };
    let order = match sort {
        Some(Sort { key, direction }) => {
            let col = match key {
                SortKey::Name => "name COLLATE NOCASE",
                SortKey::Artist => "artist COLLATE NOCASE",
                SortKey::Album => "album COLLATE NOCASE",
                SortKey::Charter => "charter COLLATE NOCASE",
                SortKey::Length => "length_ms",
            };
            format!("{col} {}", if direction == SortDir::Asc { "ASC" } else { "DESC" })
        }
        None => "modified DESC".into(),
    };
    let offset = (page.saturating_sub(1) * per_page) as i64;
    with_db(|c| {
        let params: Vec<&dyn rusqlite::ToSql> = words.iter().map(|w| w as &dyn rusqlite::ToSql).collect();
        let total: i64 = c.query_row(&format!("SELECT COUNT(*) FROM charts {where_sql}"), params.as_slice(), |r| r.get(0))?;
        let mut stmt = c.prepare(&format!("SELECT * FROM charts {where_sql} ORDER BY {order} LIMIT {per_page} OFFSET {offset}"))?;
        let rows = stmt.query_map(params.as_slice(), row_to_chart)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows.into_iter().map(Arc::new).collect(), total as u64))
    })
}

fn diff_column(instrument: &str) -> Option<&'static str> {
    Some(match instrument {
        "guitar" => "d_guitar",
        "guitarcoop" => "d_guitar_coop",
        "rhythm" => "d_rhythm",
        "bass" => "d_bass",
        "drums" => "d_drums",
        "keys" => "d_keys",
        "guitarghl" => "d_guitarghl",
        "bassghl" => "d_bassghl",
        _ => return None,
    })
}

fn row_to_chart(r: &rusqlite::Row) -> rusqlite::Result<Chart> {
    let uid: String = r.get("uid")?;
    Ok(Chart {
        chart_id: 9_000_000_000u64 + uid.bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64)) % 1_000_000_000,
        md5: format!("rv-{uid}"),
        modified_time: r.get("modified")?,
        name: r.get("name")?,
        artist: r.get("artist")?,
        album: r.get("album")?,
        genre: r.get("genre")?,
        year: r.get("year")?,
        charter: r.get("charter")?,
        song_length: r.get("length_ms")?,
        diff_guitar: r.get("d_guitar")?,
        diff_guitar_coop: r.get("d_guitar_coop")?,
        diff_rhythm: r.get("d_rhythm")?,
        diff_bass: r.get("d_bass")?,
        diff_drums: r.get("d_drums")?,
        diff_keys: r.get("d_keys")?,
        diff_guitarghl: r.get("d_guitarghl")?,
        diff_bassghl: r.get("d_bassghl")?,
        diff_vocals: r.get("d_vocals")?,
        album_art_md5: r.get("art")?,
        rv: Some(Arc::new(RvFile {
            download_url: r.get("download_url")?,
            external_url: r.get("external_url")?,
            page_url: r.get::<_, Option<String>>("page_url")?.unwrap_or_default(),
            title: r.get("name")?,
        })),
        ..Default::default()
    })
}

/// Deletes the local copy (Settings > "Rebuild"). Deletes the files rather than emptying the
/// tables, so it also works when the database is damaged.
pub fn reset() -> Result<(), String> {
    let mut guard = DB.lock().unwrap();
    *guard = None; // close it first, or Windows won't delete the files
    let path = db_path().ok_or("No cache folder available")?;
    delete_files(&path);
    let _ = std::fs::remove_file(backup_path(&path));
    if path.exists() {
        return Err("Couldn't delete the local chart list (is another copy of Truss open?)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database another copy of Truss has locked is left alone, not treated as damaged.
    #[test]
    fn locked_database_is_not_deleted() {
        let dir = std::env::temp_dir().join(format!("truss-rvlock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rhythmverse.sqlite");
        let a = Connection::open(&path).unwrap();
        a.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES(1); BEGIN EXCLUSIVE;").unwrap();
        assert!(open(&path).is_err(), "busy, so it isn't opened");
        a.execute_batch("COMMIT;").unwrap();
        let n: i64 = open(&path).unwrap().query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "the data survived");
        drop(a);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A damaged database is replaced by the backup, keeping its data.
    #[test]
    fn restores_from_backup() {
        let dir = std::env::temp_dir().join(format!("truss-rvbak-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rhythmverse.sqlite");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES(42);").unwrap();
            c.execute("VACUUM INTO ?1", [backup_path(&path).to_string_lossy()]).unwrap();
        }
        let mut b = std::fs::read(&path).unwrap();
        for v in b.iter_mut().skip(100).take(3000) {
            *v = 0xAB;
        }
        std::fs::write(&path, &b).unwrap();
        let c = open(&path).unwrap();
        let x: i64 = c.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(x, 42);
        drop(c);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A damaged database file is replaced with an empty, working one.
    #[test]
    fn replaces_damaged_database() {
        let dir = std::env::temp_dir().join(format!("truss-rvdb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rhythmverse.sqlite");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES(1);").unwrap();
        }
        // Scribble over the middle of the file, like a torn write.
        let mut b = std::fs::read(&path).unwrap();
        for v in b.iter_mut().skip(100).take(3000) {
            *v = 0xAB;
        }
        std::fs::write(&path, &b).unwrap();
        let c = open(&path).unwrap();
        let ok: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0)).unwrap();
        assert_eq!(ok, "ok");
        drop(c);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Network test: TRUSS_TEST_RV_CATALOG=1 fetches one real catalogue page and times it.
    #[test]
    fn one_catalog_page() {
        if std::env::var("TRUSS_TEST_RV_CATALOG").is_err() {
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let t = std::time::Instant::now();
        let (charts, total) = rt.block_on(rhythmverse::fetch_catalog_page(1)).unwrap();
        println!("page 1: {} charts of {total} in {:?}", charts.len(), t.elapsed());
        assert!(!charts.is_empty());
        let newest = charts.first().and_then(|c| c.modified_time.clone());
        let oldest = charts.last().and_then(|c| c.modified_time.clone());
        println!("newest {newest:?}, oldest on page {oldest:?}");
        assert!(newest >= oldest);
    }
}
