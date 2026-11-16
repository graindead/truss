//! Downloading public MEGA file links (a few chart packs are hosted there).
//!
//! A MEGA file link carries the file's key after its ID (`mega.nz/file/<id>#<key>`, or the older
//! `mega.nz/#!<id>!<key>`). MEGA's public API gives a download address for the ID; the file comes
//! encrypted with AES-128 in CTR mode, and is decrypted here with the link's key as it arrives.

use aes::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use futures::{SinkExt, StreamExt, channel::mpsc::Sender};
use std::path::{Path, PathBuf};

type Cipher = ctr::Ctr128BE<aes::Aes128>;

const API: &str = "https://g.api.mega.co.nz/cs";

/// A file link's ID and its 32-byte key.
pub fn parse_link(url: &str) -> Option<(String, [u8; 32])> {
    let lower = url.to_ascii_lowercase();
    if !(lower.contains("mega.nz/") || lower.contains("mega.co.nz/")) || lower.contains("/folder/") || lower.contains("#f!") {
        return None;
    }
    let (id, key) = if let Some(i) = url.find("/file/") {
        url[i + 6..].split_once('#')?
    } else {
        let rest = &url[url.find("#!")? + 2..];
        rest.split_once('!')?
    };
    let key = base64url(key.trim_end_matches('/'))?;
    let key: [u8; 32] = key.try_into().ok()?;
    (!id.is_empty()).then(|| (id.to_string(), key))
}

/// MEGA's unpadded URL-safe base64.
fn base64url(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s.trim_end_matches('=').as_bytes() {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// The AES key and counter start of a file: the link key's two halves combined, and its third
/// quarter as the nonce.
fn file_key(k: &[u8; 32]) -> ([u8; 16], [u8; 16]) {
    let mut key = [0u8; 16];
    for i in 0..16 {
        key[i] = k[i] ^ k[i + 16];
    }
    let mut iv = [0u8; 16];
    iv[..8].copy_from_slice(&k[16..24]);
    (key, iv)
}

/// A file ready to download: where from, its size, and how to decrypt it.
pub struct File {
    pub url: String,
    pub size: u64,
    key: [u8; 16],
    iv: [u8; 16],
}

impl File {
    /// Decrypts `data`, which starts `offset` bytes into the file.
    pub fn decrypt(&self, offset: u64, data: &mut [u8]) {
        let mut c = Cipher::new(&self.key.into(), &self.iv.into());
        c.seek(offset);
        c.apply_keystream(data);
    }
}

/// Asks MEGA where a file link's file is.
pub async fn resolve(link: &str) -> Result<File, String> {
    let (id, k) = parse_link(link).ok_or("Couldn't read the MEGA link (Truss can download MEGA file links, not folders).")?;
    let mut wait = 1;
    for attempt in 0..5 {
        let r = crate::api::CLIENT
            .post(API)
            .query(&[("id", "0")])
            .json(&serde_json::json!([{ "a": "g", "g": 1, "ssl": 2, "p": id }]))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|_| "Couldn't reach MEGA. Check your connection and try again.".to_string())?;
        let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
        // Errors come back as negative numbers, alone or in the list.
        let item = v.get(0).cloned().unwrap_or(v.clone());
        if let Some(code) = item.as_i64() {
            if code == -3 && attempt < 4 {
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                wait *= 2;
                continue;
            }
            return Err(error_text(code));
        }
        let url = item["g"].as_str().ok_or("MEGA didn't give a download address.")?.to_string();
        let size = item["s"].as_u64().unwrap_or(0);
        let (key, iv) = file_key(&k);
        return Ok(File { url, size, key, iv });
    }
    Err(error_text(-3))
}

fn error_text(code: i64) -> String {
    match code {
        -9 | -2 => "This pack's MEGA link is dead (the file was deleted or the link is wrong).".into(),
        -11 | -16 => "MEGA won't hand over this file (it was taken down or is private). Open the link in your browser.".into(),
        -17 | -4 => "MEGA's download limit for your connection has been reached. Try again in a few hours.".into(),
        -3 | -18 => "MEGA is busy right now. Try again in a little while.".into(),
        c => format!("MEGA refused the download (error {c})."),
    }
}

/// Downloads and decrypts a MEGA file link into `dir`; returns the file.
pub async fn download_to(link: &str, dir: &Path, tx: &mut Sender<crate::download::Progress>) -> Result<PathBuf, String> {
    use crate::download::Progress;
    use std::io::Write;
    let _ = tx.send(Progress::Status("Downloading from MEGA...".into())).await;
    let f = resolve(link).await?;
    let resp = crate::api::CLIENT
        .get(&f.url)
        .timeout(std::time::Duration::from_secs(60 * 60 * 3))
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    match resp.status().as_u16() {
        200 => {}
        509 => return Err(error_text(-17)),
        c => return Err(format!("Download failed: MEGA returned {c}")),
    }
    let total = resp.content_length().or(Some(f.size)).filter(|&n| n > 0);
    let path = dir.join("pack.mega");
    let mut out = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).map_err(|e| format!("Cannot write to library: {e}"))?);
    let mut cipher = Cipher::new(&f.key.into(), &f.iv.into());
    let mut stream = resp.bytes_stream();
    let mut done = 0u64;
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    while let Some(chunk) = stream.next().await {
        let mut chunk = chunk.map_err(|e| format!("Connection lost: {e}"))?.to_vec();
        cipher.apply_keystream(&mut chunk);
        out.write_all(&chunk).map_err(|e| format!("Write failed: {e}"))?;
        done += chunk.len() as u64;
        if last.elapsed() >= std::time::Duration::from_millis(33) {
            last = std::time::Instant::now();
            let _ = tx.try_send(Progress::Bytes { done, total });
        }
    }
    let _ = tx.send(Progress::Bytes { done, total }).await;
    out.flush().map_err(|e| format!("Write failed: {e}"))?;
    if f.size > 0 && done != f.size {
        return Err("The MEGA download ended early. Try again.".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_links() {
        let (id, k) = parse_link("https://mega.nz/#!74VXACQA!rgszsgd_3BVibbUJGsh0UDKH2NJwiyEeHYRO5sYGPZY").unwrap();
        assert_eq!(id, "74VXACQA");
        assert_eq!(k.len(), 32);
        let (id2, k2) = parse_link("https://mega.nz/file/74VXACQA#rgszsgd_3BVibbUJGsh0UDKH2NJwiyEeHYRO5sYGPZY").unwrap();
        assert_eq!((id2, k2), (id, k));
        assert!(parse_link("https://mega.nz/folder/abc#def").is_none());
        assert!(parse_link("https://example.com/#!a!b").is_none());
        assert_eq!(base64url("TWFu").unwrap(), b"Man");
    }

    /// The decryption matches whether it starts at the file's start or partway in.
    #[test]
    fn decrypts_from_any_offset() {
        let f = File { url: String::new(), size: 0, key: [7; 16], iv: [9; 16] };
        let mut whole = vec![0u8; 100];
        f.decrypt(0, &mut whole);
        let mut part = vec![0u8; 37];
        f.decrypt(63, &mut part);
        assert_eq!(&whole[63..], &part[..]);
    }

    /// Downloads the start of the real Zero Gravity pack (TRUSS_TEST_MEGA=1): it should decrypt
    /// to an archive.
    #[test]
    fn real_link_decrypts() {
        if std::env::var_os("TRUSS_TEST_MEGA").is_none() {
            return;
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let f = rt.block_on(resolve("https://mega.nz/#!74VXACQA!rgszsgd_3BVibbUJGsh0UDKH2NJwiyEeHYRO5sYGPZY")).unwrap();
        let mut head = rt
            .block_on(async { crate::api::CLIENT.get(&f.url).header("Range", "bytes=0-15").send().await.unwrap().bytes().await.unwrap() })
            .to_vec();
        f.decrypt(0, &mut head);
        println!("{} bytes; starts {:?}", f.size, &head[..8]);
        assert!(head.starts_with(b"Rar!") || head.starts_with(b"PK") || head.starts_with(b"7z"));
    }
}
