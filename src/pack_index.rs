//! The pack index: which charts are in each Custom Songs Central pack (Packs page), so Truss can
//! tell which packs you already have, wherever and however their charts are kept.
//!
//! Each chart is listed by the key Truss matches charts on (artist, name and charter from its
//! song.ini; see `library::ini_key`). The index is built once with the `build` test below, which
//! reads only the song.ini files where the archive allows it (.zip, non-solid .rar, Google Drive
//! folders) and streams the rest (.7z, which is solid), and is published in this repo.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where Truss gets the latest index.
pub const INDEX_URL: &str = "https://raw.githubusercontent.com/graindead/truss/main/data/pack-index.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackIndex {
    pub version: u32,
    /// When it was last added to (seconds since 1970).
    pub built: u64,
    /// By download link (as the pack's page links it, short links followed).
    pub links: BTreeMap<String, LinkEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkEntry {
    /// The pack's page.
    pub page: String,
    /// Each chart: [key, its folder inside the pack].
    pub charts: Vec<(String, String)>,
    /// Charts that couldn't be read (.sng files, say), so they aren't counted.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unread: u32,
    /// Why this download couldn't be indexed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}
/// How much of a pack download is in the library.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Have {
    pub have: usize,
    pub total: usize,
}

impl PackIndex {
    pub fn parse(bytes: &[u8]) -> Option<PackIndex> {
        serde_json::from_slice(bytes).ok()
    }

    /// How many of a download's charts are in the library (`None` if it isn't indexed).
    pub fn have(&self, link: &str, in_library: &dyn Fn(&str) -> bool) -> Option<Have> {
        let e = self.links.get(link).filter(|e| e.error.is_none() && !e.charts.is_empty())?;
        // A chart can be in a pack twice (a bonus tier, say): count each once.
        let keys: std::collections::BTreeSet<&str> = e.charts.iter().map(|(k, _)| k.as_str()).collect();
        Some(Have { have: keys.iter().filter(|k| in_library(k)).count(), total: keys.len() })
    }

    /// The same for a pack's page, over all its downloads.
    pub fn have_page(&self, page: &str, in_library: &dyn Fn(&str) -> bool) -> Option<Have> {
        let keys: std::collections::BTreeSet<&str> = self
            .links
            .values()
            .filter(|e| e.page == page && e.error.is_none())
            .flat_map(|e| e.charts.iter().map(|(k, _)| k.as_str()))
            .collect();
        if keys.is_empty() {
            return None;
        }
        Some(Have { have: keys.iter().filter(|k| in_library(k)).count(), total: keys.len() })
    }
}

fn cache_path() -> Option<std::path::PathBuf> {
    crate::settings::project_dirs().map(|d| d.cache_dir().join("packs").join("pack-index.json"))
}

/// The index: the newest of the copy built into Truss, the last one downloaded, and the one
/// published now.
pub async fn load() -> Option<PackIndex> {
    let built_in = PackIndex::parse(include_bytes!("../data/pack-index.json"));
    let cached = tokio::task::spawn_blocking(|| cache_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| PackIndex::parse(&b)))
        .await
        .ok()
        .flatten();
    let fresh = async {
        let r = crate::api::CLIENT.get(INDEX_URL).timeout(std::time::Duration::from_secs(20)).send().await.ok()?;
        let b = r.error_for_status().ok()?.bytes().await.ok()?;
        let i = PackIndex::parse(&b)?;
        if let Some(p) = cache_path() {
            let _ = std::fs::create_dir_all(p.parent()?);
            let _ = std::fs::write(p, &b);
        }
        Some(i)
    }
    .await;
    [built_in, cached, fresh].into_iter().flatten().max_by_key(|i| i.built)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_charts_in_library() {
        let mut i = PackIndex::default();
        let e = |page: &str, keys: &[&str]| LinkEntry {
            page: page.into(),
            charts: keys.iter().map(|k| (k.to_string(), String::new())).collect(),
            ..Default::default()
        };
        i.links.insert("a.rar".into(), e("p1", &["x|a|c", "x|b|c", "x|b|c"]));
        i.links.insert("dlc.rar".into(), e("p1", &["x|d|c"]));
        i.links.insert("bad.rar".into(), LinkEntry { page: "p2".into(), error: Some("gone".into()), ..Default::default() });
        let lib = |k: &str| k == "x|a|c" || k == "x|d|c";
        assert_eq!(i.have("a.rar", &lib), Some(Have { have: 1, total: 2 }), "a chart listed twice counts once");
        assert_eq!(i.have_page("p1", &lib), Some(Have { have: 2, total: 3 }));
        assert_eq!(i.have("bad.rar", &lib), None);
        assert_eq!(i.have_page("p2", &lib), None);
        assert!(PackIndex::parse(include_bytes!("../data/pack-index.json")).is_some(), "the built-in index parses");
    }
}

// ---------- Building the index ----------

#[cfg(all(test, feature = "rhythmverse"))]
mod build {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    /// A remote file read through HTTP range requests, a chunk at a time.
    struct Remote {
        url: String,
        len: u64,
        pos: u64,
        chunk: u64,
        buf: Vec<u8>,
        buf_at: u64,
        fetched: Arc<AtomicU64>,
        rt: Arc<tokio::runtime::Runtime>,
        /// A MEGA file, decrypted as it's read.
        mega: Option<Arc<crate::mega::File>>,
    }

    impl Remote {
        fn open(
            url: &str,
            chunk: u64,
            fetched: Arc<AtomicU64>,
            rt: Arc<tokio::runtime::Runtime>,
            mega: Option<Arc<crate::mega::File>>,
        ) -> Result<Remote, String> {
            let mut r = Remote { url: url.to_string(), len: u64::MAX, pos: 0, chunk, buf: Vec::new(), buf_at: 0, fetched, rt, mega };
            let (head, total) = r.get(0, 15)?;
            r.len = total.ok_or("the host doesn't allow partial downloads")?;
            r.buf = head;
            Ok(r)
        }

        /// Bytes `start..=end`, and the file's full length.
        fn get(&self, start: u64, end: u64) -> Result<(Vec<u8>, Option<u64>), String> {
            let mut last = String::new();
            for attempt in 0..5 {
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_secs(2 << attempt));
                }
                let url = self.url.clone();
                let r: Result<(Vec<u8>, Option<u64>), String> = self.rt.block_on(async move {
                    let r = crate::api::CLIENT
                        .get(&url)
                        .header("Range", format!("bytes={start}-{end}"))
                        .timeout(std::time::Duration::from_secs(120))
                        .send()
                        .await
                        .map_err(|e| e.to_string())?;
                    let status = r.status().as_u16();
                    if status != 206 {
                        return Err(format!("the host returned {status}"));
                    }
                    let total = r
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.rsplit('/').next())
                        .and_then(|n| n.parse().ok());
                    Ok((r.bytes().await.map_err(|e| e.to_string())?.to_vec(), total))
                });
                match r {
                    Ok(mut v) => {
                        self.fetched.fetch_add(v.0.len() as u64, Ordering::Relaxed);
                        if let Some(m) = &self.mega {
                            m.decrypt(start, &mut v.0);
                        }
                        return Ok(v);
                    }
                    // A missing or private file won't come back.
                    Err(e) if e.contains("returned 404") || e.contains("returned 403") || e.contains("returned 410") => return Err(e),
                    Err(e) => last = e,
                }
            }
            Err(last)
        }

        /// Exactly `n` bytes at `at` (outside the read-ahead buffer).
        fn exact(&mut self, at: u64, n: u64) -> Result<Vec<u8>, String> {
            let mut out = Vec::with_capacity(n as usize);
            self.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
            let mut left = n;
            while left > 0 {
                let mut b = vec![0u8; left.min(1 << 20) as usize];
                let got = self.read(&mut b).map_err(|e| e.to_string())?;
                if got == 0 {
                    return Err("the file ended early".into());
                }
                out.extend_from_slice(&b[..got]);
                left -= got as u64;
            }
            Ok(out)
        }
    }

    impl Read for Remote {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.len || out.is_empty() {
                return Ok(0);
            }
            let in_buf = self.pos >= self.buf_at && self.pos < self.buf_at + self.buf.len() as u64;
            if !in_buf {
                let end = (self.pos + self.chunk.max(out.len() as u64)).min(self.len) - 1;
                let (b, _) = self.get(self.pos, end).map_err(std::io::Error::other)?;
                if b.is_empty() {
                    return Ok(0);
                }
                self.buf = b;
                self.buf_at = self.pos;
            }
            let i = (self.pos - self.buf_at) as usize;
            let n = out.len().min(self.buf.len() - i);
            out[..n].copy_from_slice(&self.buf[i..i + n]);
            self.pos += n as u64;
            Ok(n)
        }
    }

    impl Seek for Remote {
        fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
            self.pos = match p {
                SeekFrom::Start(n) => n,
                SeekFrom::End(n) => (self.len as i64 + n).max(0) as u64,
                SeekFrom::Current(n) => (self.pos as i64 + n).max(0) as u64,
            };
            Ok(self.pos)
        }
    }

    /// What reading one download found.
    #[derive(Default)]
    struct Found {
        charts: Vec<(String, String)>,
        unread: u32,
    }

    impl Found {
        /// A .sng chart at `path`, from its first bytes (its metadata comes first).
        fn sng(&mut self, path: &str, head: &[u8]) {
            match crate::sng::header_metadata(head) {
                Ok(Some(meta)) => self.charts.push((crate::library::pairs_key(&meta), path.replace('\\', "/"))),
                _ => self.unread += 1,
            }
        }

        /// A song.ini at `path` (inside the archive) with these bytes.
        fn ini(&mut self, path: &str, bytes: &[u8]) {
            let folder = path.replace('\\', "/");
            let folder = folder.rsplit_once('/').map(|(d, _)| d).unwrap_or("").to_string();
            self.charts.push((crate::library::ini_key(bytes), folder));
        }
    }

    fn is_ini(name: &str) -> bool {
        let n = name.replace('\\', "/").to_ascii_lowercase();
        n == "song.ini" || n.ends_with("/song.ini")
    }

    fn is_sng(name: &str) -> bool {
        name.to_ascii_lowercase().ends_with(".sng")
    }

    // ----- .zip: the file list is at the end; each song.ini is read on its own -----

    fn read_zip(r: Remote, out: &mut Found) -> Result<(), String> {
        let mut z = zip::ZipArchive::new(r).map_err(|e| format!("not a valid .zip: {e}"))?;
        for i in 0..z.len() {
            let mut e = z.by_index(i).map_err(|e| e.to_string())?;
            let name = e.name().map(|n| n.into_owned()).unwrap_or_default();
            if is_ini(&name) {
                let mut b = Vec::new();
                e.read_to_end(&mut b).map_err(|e| e.to_string())?;
                out.ini(&name, &b);
            } else if is_sng(&name) {
                let mut head = Vec::new();
                let _ = (&mut e).take(SNG_HEAD).read_to_end(&mut head);
                out.sng(&name, &head);
            }
        }
        Ok(())
    }

    /// Enough of a .sng file to hold its metadata.
    const SNG_HEAD: u64 = 256 << 10;

    // ----- .7z: solid, so the archive is streamed until every song.ini has been read -----

    fn read_7z(r: Remote, out: &mut Found) -> Result<(), String> {
        let mut reader = sevenz_rust2::ArchiveReader::new(r, sevenz_rust2::Password::empty()).map_err(|e| format!("not a valid .7z: {e}"))?;
        let inis = reader.archive().files.iter().filter(|f| f.has_stream && (is_ini(&f.name) || is_sng(&f.name))).count();
        let mut got = 0;
        let mut err = None;
        if inis > 0 {
            reader
                .for_each_entries(|entry, data| {
                    if entry.has_stream && (is_ini(entry.name()) || is_sng(entry.name())) {
                        let sng = is_sng(entry.name());
                        let mut b = Vec::new();
                        let r = if sng { data.take(SNG_HEAD).read_to_end(&mut b) } else { data.read_to_end(&mut b) };
                        if let Err(e) = r {
                            err = Some(e.to_string());
                            return Ok(false);
                        }
                        if sng {
                            out.sng(entry.name(), &b);
                        } else {
                            out.ini(entry.name(), &b);
                        }
                        got += 1;
                        // Every song.ini read: the rest of the archive isn't needed.
                        return Ok(got < inis);
                    }
                    Ok(true)
                })
                .map_err(|e| format!("couldn't read the .7z: {e}"))?;
        }
        err.map_or(Ok(()), Err)
    }

    // ----- .rar (not solid): walk the file headers, fetching only each song.ini's data -----

    fn vint(b: &[u8], i: &mut usize) -> Result<u64, String> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let c = *b.get(*i).ok_or("truncated .rar header")?;
            *i += 1;
            v |= ((c & 0x7f) as u64) << shift;
            shift += 7;
            if c & 0x80 == 0 || shift > 63 {
                return Ok(v);
            }
        }
    }

    fn write_vint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    /// Unpacks one file from a small archive made of `parts` (signature, main header, the
    /// file's header and data, end of archive).
    fn unrar_one(parts: &[&[u8]]) -> Result<Vec<u8>, String> {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("truss-index-{}-{}.rar", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, parts.concat()).map_err(|e| e.to_string())?;
        let r = (|| {
            let a = unrar::Archive::new(&path).open_for_processing().map_err(|e| e.to_string())?;
            let h = a.read_header().map_err(|e| e.to_string())?.ok_or("empty")?;
            let (data, _) = h.read().map_err(|e| e.to_string())?;
            Ok(data)
        })();
        let _ = std::fs::remove_file(&path);
        r
    }

    fn read_rar5(mut r: Remote, out: &mut Found) -> Result<(), String> {
        const SIG: &[u8] = b"Rar!\x1a\x07\x01\x00";
        let mut pos = 8u64;
        let mut main: Option<Vec<u8>> = None;
        // End of archive header: type 5, no flags.
        let end = {
            let body = [5u8, 0, 0];
            let mut sized = Vec::new();
            write_vint(body.len() as u64, &mut sized);
            sized.extend_from_slice(&body);
            let mut v = crc32fast::hash(&sized).to_le_bytes().to_vec();
            v.extend_from_slice(&sized);
            v
        };
        while pos + 7 < r.len {
            // CRC (4), header size (vint, up to 3 bytes for our purposes), then the header.
            let start = r.exact(pos, 7.min(r.len - pos))?;
            let mut i = 4;
            let hsize = vint(&start, &mut i)?;
            let hlen = i as u64 + hsize;
            if hsize == 0 || hsize > 2 << 20 {
                return Err("damaged .rar header".into());
            }
            let header = r.exact(pos, hlen)?;
            let mut j = i;
            let htype = vint(&header, &mut j)?;
            let hflags = vint(&header, &mut j)?;
            if hflags & 1 != 0 {
                vint(&header, &mut j)?;
            }
            let data_size = if hflags & 2 != 0 { vint(&header, &mut j)? } else { 0 };
            match htype {
                1 => main = Some(header.clone()),
                2 => {
                    let file_flags = vint(&header, &mut j)?;
                    let _unpacked = vint(&header, &mut j)?;
                    let _attr = vint(&header, &mut j)?;
                    if file_flags & 2 != 0 {
                        j += 4;
                    }
                    if file_flags & 4 != 0 {
                        j += 4;
                    }
                    let _comp = vint(&header, &mut j)?;
                    let _os = vint(&header, &mut j)?;
                    let nlen = vint(&header, &mut j)? as usize;
                    let name = String::from_utf8_lossy(header.get(j..j + nlen).ok_or("truncated .rar name")?).into_owned();
                    let is_dir = file_flags & 1 != 0;
                    // Split across volumes: can't be read on its own.
                    let split = hflags & (0x08 | 0x10) != 0;
                    if !is_dir && is_ini(&name) && !split {
                        let data = r.exact(pos + hlen, data_size)?;
                        let main = main.as_deref().ok_or("no .rar main header")?;
                        match unrar_one(&[SIG, main, &header, &data, &end]) {
                            Ok(b) => out.ini(&name, &b),
                            Err(_) => out.unread += 1,
                        }
                    } else if !is_dir && is_sng(&name) {
                        out.unread += 1;
                    }
                }
                4 => return Err("the .rar is encrypted".into()),
                5 => break,
                _ => {}
            }
            pos += hlen + data_size;
        }
        Ok(())
    }

    fn read_rar4(mut r: Remote, out: &mut Found) -> Result<(), String> {
        const SIG: &[u8] = b"Rar!\x1a\x07\x00";
        const END: &[u8] = &[0xc4, 0x3d, 0x7b, 0x00, 0x40, 0x07, 0x00];
        let mut pos = 7u64;
        let mut main: Option<Vec<u8>> = None;
        while pos + 7 <= r.len {
            let base = r.exact(pos, 7)?;
            let htype = base[2];
            let flags = u16::from_le_bytes([base[3], base[4]]);
            let hsize = u16::from_le_bytes([base[5], base[6]]) as u64;
            if hsize < 7 {
                return Err("damaged .rar header".into());
            }
            let header = r.exact(pos, hsize)?;
            let mut add = 0u64;
            if flags & 0x8000 != 0 && header.len() >= 11 {
                add = u32::from_le_bytes([header[7], header[8], header[9], header[10]]) as u64;
            }
            match htype {
                0x73 => main = Some(header.clone()),
                0x74 if header.len() >= 32 => {
                    let mut pack = u32::from_le_bytes([header[7], header[8], header[9], header[10]]) as u64;
                    let nlen = u16::from_le_bytes([header[26], header[27]]) as usize;
                    let mut name_at = 32;
                    if flags & 0x100 != 0 && header.len() >= 40 {
                        pack |= (u32::from_le_bytes([header[32], header[33], header[34], header[35]]) as u64) << 32;
                        name_at = 40;
                    }
                    add = pack;
                    let raw = header.get(name_at..name_at + nlen).unwrap_or(&[]);
                    // Unicode names keep a plain copy before a zero byte.
                    let plain = raw.split(|&b| b == 0).next().unwrap_or(raw);
                    let name = String::from_utf8_lossy(plain).into_owned();
                    let is_dir = flags & 0xe0 == 0xe0;
                    let split = flags & 0x03 != 0;
                    if !is_dir && is_ini(&name) && !split {
                        let data = r.exact(pos + hsize, pack)?;
                        let main = main.as_deref().ok_or("no .rar main header")?;
                        match unrar_one(&[SIG, main, &header, &data, END]) {
                            Ok(b) => out.ini(&name, &b),
                            Err(_) => out.unread += 1,
                        }
                    } else if !is_dir && is_sng(&name) {
                        out.unread += 1;
                    }
                }
                0x7b => break,
                _ => {}
            }
            pos += hsize + add;
        }
        Ok(())
    }

    // ----- One download -----

    fn drive_file_id(url: &str) -> Option<String> {
        let i = url.find("/file/d/").map(|i| i + 8).or_else(|| url.find("id=").map(|i| i + 3))?;
        let id: String = url[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        (!id.is_empty()).then_some(id)
    }

    fn index_link(url: &str, rt: &Arc<tokio::runtime::Runtime>, fetched: &Arc<AtomicU64>) -> Result<Found, String> {
        let mut out = Found::default();
        if url.contains("drive.google.com") && url.contains("/folders/") {
            // Loose chart folders and/or archives (a pack's tiers, its DLC...).
            let files = rt.block_on(crate::rhythmverse::drive_tree(url))?;
            for (id, path) in files {
                let lower = path.to_ascii_lowercase();
                let direct = format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t");
                if is_ini(&lower) {
                    let (b, _) = plain_get(&id, rt, fetched)?;
                    out.ini(&path, &b);
                } else if [".zip", ".7z", ".rar"].iter().any(|x| lower.ends_with(x)) && !rar_later_part(&lower) {
                    let mut inner = Found::default();
                    read_archive(&direct, None, rt, fetched, &mut inner).map_err(|e| format!("{path}: {e}"))?;
                    let base = path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
                    out.charts.extend(inner.charts.into_iter().map(|(k, f)| (k, format!("{base}{f}"))));
                    out.unread += inner.unread;
                } else if is_sng(&lower) {
                    let head = Remote { url: direct, len: u64::MAX, pos: 0, chunk: 0, buf: Vec::new(), buf_at: 0, fetched: fetched.clone(), rt: rt.clone(), mega: None }
                        .get(0, SNG_HEAD - 1)
                        .map(|(b, _)| b)
                        .unwrap_or_default();
                    out.sng(&path, &head);
                }
            }
            return Ok(out);
        }
        if crate::mega::parse_link(url).is_some() {
            let f = Arc::new(rt.block_on(crate::mega::resolve(url))?);
            read_archive(&f.url.clone(), Some(f), rt, fetched, &mut out)?;
            return Ok(out);
        }
        let direct = match drive_file_id(url).filter(|_| url.contains("drive.google.com")) {
            Some(id) => format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t"),
            None => url.replace(' ', "%20"),
        };
        read_archive(&direct, None, rt, fetched, &mut out)?;
        Ok(out)
    }

    /// "x.part2.rar": read through its first part.
    fn rar_later_part(lower: &str) -> bool {
        lower
            .strip_suffix(".rar")
            .and_then(|s| s.rsplit_once(".part"))
            .and_then(|(_, n)| n.parse::<u32>().ok())
            .is_some_and(|n| n > 1)
    }

    /// A small file from Drive, whole.
    fn plain_get(id: &str, rt: &Arc<tokio::runtime::Runtime>, fetched: &Arc<AtomicU64>) -> Result<(Vec<u8>, Option<u64>), String> {
        let url = format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t");
        let b = rt.block_on(async move {
            crate::api::CLIENT
                .get(&url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| e.to_string())?
                .bytes()
                .await
                .map_err(|e| e.to_string())
        })?;
        fetched.fetch_add(b.len() as u64, Ordering::Relaxed);
        Ok((b.to_vec(), None))
    }

    /// Reads the song.ini files of a remote archive, by its type.
    fn read_archive(
        direct: &str,
        mega: Option<Arc<crate::mega::File>>,
        rt: &Arc<tokio::runtime::Runtime>,
        fetched: &Arc<AtomicU64>,
        out: &mut Found,
    ) -> Result<(), String> {
        let remote = |chunk| Remote::open(direct, chunk, fetched.clone(), rt.clone(), mega.clone());
        let head = remote(64 << 10)?;
        let magic = head.buf.clone();
        if magic.starts_with(b"PK") {
            read_zip(remote(256 << 10)?, out)?;
        } else if magic.starts_with(b"7z\xbc\xaf\x27\x1c") {
            read_7z(remote(8 << 20)?, out)?;
        } else if magic.starts_with(b"Rar!\x1a\x07\x01\x00") {
            read_rar5(head, out)?;
        } else if magic.starts_with(b"Rar!\x1a\x07\x00") {
            read_rar4(head, out)?;
        } else if magic.starts_with(b"SNGPKG") {
            let (head, _) = remote(64 << 10)?.get(0, SNG_HEAD - 1)?;
            out.sng(direct, &head);
        } else {
            return Err("not an archive Truss can read".into());
        }
        Ok(())
    }

    fn now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }

    /// Builds or tops up the pack index (TRUSS_BUILD_PACK_INDEX = the index file). Downloads
    /// already in it are skipped, so it can be stopped and run again; failed ones are retried.
    /// TRUSS_PACK_INDEX_ONLY = a page URL (or part of one) limits it to matching packs.
    #[test]
    fn build_pack_index() {
        let Some(path) = std::env::var_os("TRUSS_BUILD_PACK_INDEX") else { return };
        let path = std::path::PathBuf::from(path);
        let only = std::env::var("TRUSS_PACK_INDEX_ONLY").ok();
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let index = Arc::new(Mutex::new(
            std::fs::read(&path).ok().and_then(|b| PackIndex::parse(&b)).unwrap_or(PackIndex { version: 1, ..Default::default() }),
        ));
        // Every download link, from each pack's page.
        let mut todo: Vec<(String, String, String)> = Vec::new();
        // TRUSS_PACK_INDEX_GAMES = a series ("Guitar Hero"): index those game setlists instead.
        if let Ok(series) = std::env::var("TRUSS_PACK_INDEX_GAMES") {
            for g in rt.block_on(crate::game_setlists::load()).into_iter().filter(|g| g.series.eq_ignore_ascii_case(&series)) {
                if only.as_ref().is_some_and(|o| !g.title.contains(o.as_str())) {
                    continue;
                }
                for l in &g.links {
                    let done = index.lock().unwrap().links.get(&l.url).is_some_and(|e| e.error.is_none());
                    if !done && crate::packs::can_download(&l.url) {
                        todo.push((g.key(), format!("{} - {}", g.title, l.label), l.url.clone()));
                    }
                }
            }
        }
        let packs = if todo.is_empty() && std::env::var_os("TRUSS_PACK_INDEX_GAMES").is_none() {
            rt.block_on(crate::packs::fetch_all()).unwrap()
        } else {
            Vec::new()
        };
        println!("{} packs on the site", packs.len());
        for p in &packs {
            if only.as_ref().is_some_and(|o| !p.url.contains(o.as_str())) {
                continue;
            }
            match rt.block_on(crate::packs::fetch_details(p.url.clone())) {
                Ok(d) => {
                    for l in d.links {
                        let done = index.lock().unwrap().links.get(&l.url).is_some_and(|e| e.error.is_none());
                        if !done && crate::packs::can_download(&l.url) {
                            todo.push((p.url.clone(), p.title.clone(), l.url));
                        }
                    }
                }
                Err(e) => println!("  couldn't read {}: {e}", p.url),
            }
        }
        println!("{} downloads to index", todo.len());
        let todo = Arc::new(Mutex::new(todo.into_iter().collect::<std::collections::VecDeque<_>>()));
        let total_fetched = Arc::new(AtomicU64::new(0));
        let save = |i: &PackIndex| {
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(i).unwrap()).unwrap();
            std::fs::rename(&tmp, &path).unwrap();
        };
        let workers: Vec<_> = (0..3)
            .map(|_| {
                let (todo, index, rt, total_fetched) = (todo.clone(), index.clone(), rt.clone(), total_fetched.clone());
                let path = path.clone();
                std::thread::spawn(move || {
                    loop {
                        let Some((page, title, url)) = todo.lock().unwrap().pop_front() else { break };
                        let fetched = Arc::new(AtomicU64::new(0));
                        let t = std::time::Instant::now();
                        let r = index_link(&url, &rt, &fetched);
                        let bytes = fetched.load(Ordering::Relaxed);
                        total_fetched.fetch_add(bytes, Ordering::Relaxed);
                        let entry = match r {
                            Ok(f) => {
                                println!(
                                    "OK  {title}: {} charts{} ({:.1} MB in {:.0?})",
                                    f.charts.len(),
                                    if f.unread > 0 { format!(", {} unread", f.unread) } else { String::new() },
                                    bytes as f64 / 1e6,
                                    t.elapsed()
                                );
                                LinkEntry { page, charts: f.charts, unread: f.unread, error: None }
                            }
                            Err(e) => {
                                println!("ERR {title}: {e}");
                                LinkEntry { page, error: Some(e), ..Default::default() }
                            }
                        };
                        let mut i = index.lock().unwrap();
                        i.links.insert(url, entry);
                        i.built = now();
                        let tmp = path.with_extension("json.tmp");
                        std::fs::write(&tmp, serde_json::to_vec(&*i).unwrap()).unwrap();
                        std::fs::rename(&tmp, &path).unwrap();
                    }
                })
            })
            .collect();
        for w in workers {
            let _ = w.join();
        }
        let i = index.lock().unwrap();
        save(&i);
        let charts: usize = i.links.values().map(|e| e.charts.len()).sum();
        let errors = i.links.values().filter(|e| e.error.is_some()).count();
        println!(
            "index: {} downloads, {charts} charts, {errors} failed; {:.2} GB fetched this run",
            i.links.len(),
            total_fetched.load(Ordering::Relaxed) as f64 / 1e9
        );
    }
}
