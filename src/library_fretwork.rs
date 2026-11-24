//! Fretwork tiers of the library's charts (each one's Expert lead guitar track), for the Library
//! page's "Your level". Worked out from the notes in the background, at low priority, and kept by
//! chart file, so each is only read once. Charts are only read.

use crate::library::{LocalChart, lower_priority};
use futures::SinkExt;
use futures::channel::mpsc::Sender;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Chart file hash -> its Expert guitar tier, or none (no such track, or it can't be read).
pub type Tiers = HashMap<String, Option<u32>>;

/// Charts rated between saves.
const BATCH: usize = 1000;
const THREADS: usize = 4;

#[derive(Debug, Clone)]
pub enum Event {
    /// (rated so far, to rate).
    Progress(usize, usize),
    Done(Arc<Tiers>),
}

fn cache_file() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("library-fretwork.msgpack"))
}

/// The charts being read right now. Still there at the next run: Truss closed (or a chart
/// crashed it) part-way.
fn marker_file() -> Option<PathBuf> {
    crate::settings::cache_dir().map(|d| d.join("library-fretwork.reading"))
}

fn load() -> Tiers {
    cache_file().and_then(|p| std::fs::read(p).ok()).and_then(|b| rmp_serde::from_slice(&b).ok()).unwrap_or_default()
}

fn save(t: &Tiers) {
    let Some(p) = cache_file() else { return };
    let _ = p.parent().map(std::fs::create_dir_all);
    let tmp = p.with_extension("msgpack.tmp");
    if let Ok(b) = rmp_serde::to_vec(t)
        && std::fs::write(&tmp, b).is_ok()
    {
        let _ = std::fs::rename(&tmp, &p);
    }
}

/// What a run starts with: the charts left half-read last time, to be read one at a time (so the
/// one that crashed Truss, if one did, is found), and one caught that way, not to be read again.
fn interrupted(marker: &str) -> (HashSet<String>, Option<String>) {
    let mut left: Vec<String> = marker.lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
    if left.len() == 1 { (HashSet::new(), left.pop()) } else { (left.into_iter().collect(), None) }
}

/// One chart's Expert guitar tier.
fn tier(c: &LocalChart) -> Option<u32> {
    let (files, ini) = crate::leaderboard::local_note_files(&c.path).ok()?;
    crate::trackhash::guitar_tier(&files, ini.as_deref())
}

/// Rates a batch, with a note of what's being read kept meanwhile (see `marker_file`).
fn rate_batch(batch: &[(String, Arc<LocalChart>)], tiers: &mut Tiers) {
    let marker = marker_file();
    if let Some(p) = &marker {
        let names: Vec<&str> = batch.iter().map(|(h, _)| h.as_str()).collect();
        let _ = std::fs::write(p, names.join("\n"));
    }
    let rated: Vec<(String, Option<u32>)> = std::thread::scope(|s| {
        let chunk = batch.len().div_ceil(THREADS).max(1);
        let threads: Vec<_> = batch
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    lower_priority();
                    part.iter().map(|(h, c)| (h.clone(), tier(c))).collect::<Vec<_>>()
                })
            })
            .collect();
        threads.into_iter().filter_map(|t| t.join().ok()).flatten().collect()
    });
    tiers.extend(rated);
    save(tiers);
    if let Some(p) = &marker {
        let _ = std::fs::remove_file(p);
    }
}

/// Rates the charts not rated yet (the rest come from the last time).
pub fn rate(charts: Vec<Arc<LocalChart>>) -> impl futures::Stream<Item = Event> {
    iced::stream::channel(8, async move |mut tx: Sender<Event>| {
        let mut tiers = tokio::task::spawn_blocking(load).await.unwrap_or_default();
        let marker = marker_file().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let (careful, bad) = interrupted(&marker);
        if let Some(h) = bad {
            tiers.insert(h, None);
        }
        let mut seen = HashSet::new();
        let todo: Vec<(String, Arc<LocalChart>)> = charts
            .into_iter()
            .filter_map(|c| Some((c.chart_hash.clone()?, c)))
            .filter(|(h, _)| !tiers.contains_key(h) && seen.insert(h.clone()))
            .collect();
        let total = todo.len();
        let (one_by_one, rest): (Vec<_>, Vec<_>) = todo.into_iter().partition(|(h, _)| careful.contains(h));
        let batches: Vec<Vec<(String, Arc<LocalChart>)>> =
            one_by_one.into_iter().map(|c| vec![c]).chain(rest.chunks(BATCH).map(<[_]>::to_vec)).collect();
        let mut done = 0;
        for batch in batches {
            let _ = tx.send(Event::Progress(done, total)).await;
            done += batch.len();
            let mut t = std::mem::take(&mut tiers);
            match tokio::task::spawn_blocking(move || {
                rate_batch(&batch, &mut t);
                t
            })
            .await
            {
                Ok(t) => tiers = t,
                Err(_) => {
                    tiers = load();
                    break;
                }
            }
        }
        let _ = tx.send(Event::Done(Arc::new(tiers))).await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_chart_that_crashed() {
        // Closed part-way through a batch: those are read one at a time next.
        let (careful, bad) = interrupted("a\nb\nc");
        assert_eq!((careful.len(), bad), (3, None));
        // Closed while reading just one: that one isn't read again.
        assert_eq!(interrupted("b"), (HashSet::new(), Some("b".into())));
        assert_eq!(interrupted(""), (HashSet::new(), None));
    }

    /// Rates the whole library (TRUSS_TEST_RATE=1): only reads, and saves nothing.
    #[test]
    fn rate_real_library() {
        if std::env::var("TRUSS_TEST_RATE").is_err() {
            return;
        }
        #[derive(serde::Deserialize)]
        struct Charts {
            charts: Vec<Arc<LocalChart>>,
        }
        let path = crate::settings::cache_dir().unwrap().join("library.msgpack");
        let c: Charts = rmp_serde::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let start = std::time::Instant::now();
        let mut counts = std::collections::BTreeMap::new();
        let mut crashed = Vec::new();
        std::thread::scope(|s| {
            let threads: Vec<_> = c
                .charts
                .chunks(c.charts.len().div_ceil(THREADS).max(1))
                .map(|part| {
                    s.spawn(move || {
                        part.iter()
                            .map(|c| std::panic::catch_unwind(|| tier(c)).map_err(|_| c.path.clone()))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for r in threads.into_iter().flat_map(|t| t.join().unwrap()) {
                match r {
                    Ok(t) => *counts.entry(t).or_insert(0) += 1,
                    Err(p) => crashed.push(p),
                }
            }
        });
        println!("RATED in {:?}: {counts:?}", start.elapsed());
        println!("CRASHED {}: {crashed:?}", crashed.len());
    }
}
