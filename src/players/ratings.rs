//! The bundled Fretwork ratings (`data/fretwork-ratings.bin.gz`, made by Truss Stats) and how a
//! score is matched to one. Loaded only while a player's numbers are worked out.
//!
//! One fixed-size little-endian record per rated Expert track, stored column by column (every
//! record's first byte, then every second…) as that compresses better:
//!   9 bytes  start of the track hash (decoded; all zero when the source has none: Encore)
//!   8 bytes  key of the normalised name + artist + charter
//!   4 bytes  length in ms (0 = unknown)
//!   1 byte   instrument (index into INSTRUMENTS)    1 byte   open-ended tier
//!   4 bytes  D x 100    2 bytes  movement x 100     2 bytes  spread x 100
//!   1 byte   source (0 Encore, 1 RhythmVerse)
//! No names are stored, only their key, which keeps the file small.

use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

pub const INSTRUMENTS: [&str; 6] = ["guitar", "guitarcoop", "rhythm", "bass", "drums", "keys"];
pub const RECORD: usize = 32;
/// Two lengths this close are the same recording.
const SAME_LENGTH_MS: u64 = 3000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rating {
    pub hash: [u8; 9],
    pub key: u64,
    pub length_ms: u32,
    pub instrument: u8,
    pub tier: u8,
    pub d: f64,
    pub movement: f64,
    pub spread: f64,
    pub rhythmverse: bool,
}

impl Rating {
    #[cfg(test)]
    pub fn encode(&self, out: &mut Vec<u8>) {
        let x100 = |v: f64| (v * 100.0).round().clamp(0.0, 65535.0) as u16;
        out.extend(self.hash);
        out.extend(self.key.to_le_bytes());
        out.extend(self.length_ms.to_le_bytes());
        out.extend([self.instrument, self.tier]);
        out.extend(((self.d * 100.0).round() as u32).to_le_bytes());
        out.extend(x100(self.movement).to_le_bytes());
        out.extend(x100(self.spread).to_le_bytes());
        out.push(self.rhythmverse as u8);
    }

    fn decode(b: &[u8]) -> Rating {
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u16_at = |i: usize| u16::from_le_bytes(b[i..i + 2].try_into().unwrap());
        Rating {
            hash: b[0..9].try_into().unwrap(),
            key: u64::from_le_bytes(b[9..17].try_into().unwrap()),
            length_ms: u32_at(17),
            instrument: b[21],
            tier: b[22],
            d: f64::from(u32_at(23)) / 100.0,
            movement: f64::from(u16_at(27)) / 100.0,
            spread: f64::from(u16_at(29)) / 100.0,
            rhythmverse: b[31] == 1,
        }
    }
}

/// The first 9 bytes of a 43-character track hash.
pub fn hash_prefix(hash: &str) -> Option<[u8; 9]> {
    if hash.len() != 43 {
        return None;
    }
    let mut out = [0u8; 9];
    let (mut acc, mut bits, mut n) = (0u32, 0, 0);
    for c in hash.bytes().take(12) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = ((acc << 6) | u32::from(v)) & 0xffff;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out[n] = (acc >> bits) as u8;
            n += 1;
        }
    }
    (out != [0; 9]).then_some(out)
}

/// Hidden metadata arrives as asterisks, with its spaces kept (`*** ****`).
pub fn is_hidden(s: &str) -> bool {
    s.contains('*') && s.chars().all(|c| c == '*' || c == ' ')
}

/// Colour tags and other markup removed (`<color=#F00>Name</color>`).
pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match rest.find('>') {
            Some(j) => rest = &rest[j + 1..],
            None => break,
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Lower case, accents and punctuation dropped; names made only of punctuation are kept as is.
pub fn normalise(s: &str) -> String {
    let s = strip_tags(s).to_lowercase();
    let n: String = s.nfkd().filter(|c| c.is_alphanumeric()).collect();
    if n.is_empty() { s } else { n }
}

/// `None` when a part is missing or hidden: too little to go on.
pub fn chart_key(name: &str, artist: &str, charter: &str) -> Option<u64> {
    if [name, artist, charter].iter().any(|s| is_hidden(s)) {
        return None;
    }
    let (n, a, c) = (normalise(name), normalise(artist), normalise(charter));
    if n.is_empty() || a.is_empty() || c.is_empty() {
        return None;
    }
    // FNV-1a
    let mut h = 0xcbf29ce484222325u64;
    for b in format!("{n}\0{a}\0{c}").bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    Some(h)
}

/// The rated instrument a leaderboard instrument plays; pro drums are the drum track.
pub fn rated_instrument(instrument: &str) -> Option<u8> {
    let i = if instrument == "prodrums" { "drums" } else { instrument };
    INSTRUMENTS.iter().position(|x| *x == i).map(|i| i as u8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Miss {
    NotExpert,
    /// 6-fret and other instruments Fretwork doesn't rate.
    Instrument,
    Hidden,
    /// Several rated charts fit and they disagree.
    Ambiguous,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    pub rating: Rating,
    /// Matched by the notes themselves rather than by name.
    pub by_hash: bool,
}

#[derive(Default)]
pub struct Ratings {
    all: Vec<Rating>,
    by_hash: HashMap<([u8; 9], u8), u32>,
    by_key: HashMap<(u64, u8), Vec<u32>>,
    /// Every D, sorted, per instrument (for "harder than").
    sorted: [Vec<f64>; 6],
}

impl Ratings {
    /// The ratings that come with Truss.
    pub fn bundled() -> Ratings {
        use std::io::Read;
        let mut cols = Vec::new();
        let zipped: &[u8] = include_bytes!("../../data/fretwork-ratings.bin.gz");
        if flate2::read::GzDecoder::new(zipped).read_to_end(&mut cols).is_err() {
            return Ratings::default();
        }
        let n = cols.len() / RECORD;
        let mut one = [0u8; RECORD];
        let all = (0..n)
            .map(|i| {
                for (c, b) in one.iter_mut().enumerate() {
                    *b = cols[c * n + i];
                }
                Rating::decode(&one)
            })
            .collect();
        Ratings::new(all)
    }

    pub fn new(all: Vec<Rating>) -> Ratings {
        let mut r = Ratings { all, ..Default::default() };
        for (i, x) in r.all.iter().enumerate() {
            if x.hash != [0; 9] {
                r.by_hash.entry((x.hash, x.instrument)).or_insert(i as u32);
            }
            if x.key != 0 {
                r.by_key.entry((x.key, x.instrument)).or_default().push(i as u32);
            }
            if let Some(s) = r.sorted.get_mut(x.instrument as usize) {
                s.push(x.d);
            }
        }
        for s in &mut r.sorted {
            s.sort_by(f64::total_cmp);
        }
        r
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.all.len()
    }

    /// Share (0-100) of rated charts on the same instrument that are easier.
    pub fn percentile(&self, r: &Rating) -> f64 {
        let Some(s) = self.sorted.get(r.instrument as usize).filter(|s| !s.is_empty()) else { return 0.0 };
        100.0 * s.partition_point(|d| *d < r.d) as f64 / s.len() as f64
    }

    pub fn find(&self, s: &open_chl_api::Score) -> Result<Match, Miss> {
        if s.difficulty != "expert" {
            return Err(Miss::NotExpert);
        }
        let inst = rated_instrument(&s.instrument).ok_or(Miss::Instrument)?;
        if let Some(i) = hash_prefix(&s.track_hash).and_then(|h| self.by_hash.get(&(h, inst))) {
            return Ok(Match { rating: self.all[*i as usize], by_hash: true });
        }
        self.by_name(s, inst).map(|rating| Match { rating, by_hash: false })
    }

    /// Name + artist + charter + instrument; length rules out other recordings and breaks ties.
    pub fn by_name(&self, s: &open_chl_api::Score, inst: u8) -> Result<Rating, Miss> {
        if [&s.name, &s.artist, &s.charter].iter().any(|x| is_hidden(x)) {
            return Err(Miss::Hidden);
        }
        let key = chart_key(&s.name, &s.artist, &s.charter).ok_or(Miss::NotFound)?;
        let found = self.by_key.get(&(key, inst)).ok_or(Miss::NotFound)?;
        let found: Vec<&Rating> = found.iter().map(|i| &self.all[*i as usize]).collect();
        let near: Vec<&Rating> = found
            .iter()
            .copied()
            .filter(|r| s.duration > 0 && r.length_ms > 0 && u64::from(r.length_ms).abs_diff(s.duration) <= SAME_LENGTH_MS)
            .collect();
        // No length matches: only a chart whose length isn't known is still possible.
        let pool: Vec<&Rating> =
            if near.is_empty() { found.into_iter().filter(|r| s.duration == 0 || r.length_ms == 0).collect() } else { near };
        let first = *pool.first().ok_or(Miss::NotFound)?;
        // Copies of one chart (say, on both sites) agree; different versions don't.
        if pool.iter().all(|r| r.tier == first.tier && (r.d - first.d).abs() <= 0.02 * first.d.max(1.0)) {
            Ok(*first)
        } else {
            Err(Miss::Ambiguous)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use open_chl_api::Score;

    const HASH: &str = "-Nku724NgARbyAgbfSHvqhPBeskhr5rF-m_FTuv9B5s";

    fn rating(name: &str, length_ms: u32, d: f64) -> Rating {
        Rating {
            hash: [0; 9],
            key: chart_key(name, "Artist", "Charter").unwrap(),
            length_ms,
            instrument: 0,
            tier: (d / 3.0) as u8,
            d,
            movement: 3.0,
            spread: 0.25,
            rhythmverse: false,
        }
    }

    fn score(name: &str, duration: u64) -> Score {
        Score {
            name: name.into(),
            artist: "Artist".into(),
            charter: "Charter".into(),
            duration,
            instrument: "guitar".into(),
            difficulty: "expert".into(),
            ..Default::default()
        }
    }

    #[test]
    fn record_round_trip() {
        let r = Rating { hash: hash_prefix(HASH).unwrap(), rhythmverse: true, d: 112.34, movement: 4.25, ..rating("A", 200_000, 9.0) };
        let mut b = Vec::new();
        r.encode(&mut b);
        assert_eq!(b.len(), RECORD);
        assert_eq!(Rating::decode(&b), r);
    }

    #[test]
    fn the_bundled_ratings_load() {
        let r = Ratings::bundled();
        assert_eq!(r.len(), 252_995);
        assert!(r.all.iter().all(|x| (x.instrument as usize) < INSTRUMENTS.len() && x.d.is_finite()));
    }

    #[test]
    fn hashes() {
        // "-Nku" is f8 d9 2e
        assert_eq!(hash_prefix(HASH).unwrap()[..3], [0xf8, 0xd9, 0x2e]);
        assert_eq!(hash_prefix("short"), None);
        assert_eq!(hash_prefix(&"+".repeat(43)), None);
    }

    #[test]
    fn names() {
        assert_eq!(normalise("<color=#FF0000>Édge</color> - Of!"), "edgeof");
        assert_eq!(normalise("!!!"), "!!!");
        assert_eq!(strip_tags("I <3 You"), "I <3 You");
        assert_eq!(normalise("ДОМ 2"), "дом2");
        assert_eq!(chart_key("****", "A", "B"), None);
        assert_eq!(chart_key("A", "A", ""), None);
        assert!(is_hidden("*** **** **") && !is_hidden("") && !is_hidden("*NSYNC"));
        assert_ne!(chart_key("ab", "c", "x"), chart_key("a", "bc", "x"));
    }

    #[test]
    fn hash_beats_name() {
        let h = Rating { hash: hash_prefix(HASH).unwrap(), key: 0, ..rating("Other", 1, 30.0) };
        let rs = Ratings::new(vec![rating("Song", 200_000, 9.0), h]);
        let mut s = score("Song", 200_000);
        assert!(!rs.find(&s).unwrap().by_hash);
        s.track_hash = HASH.into();
        let m = rs.find(&s).unwrap();
        assert!(m.by_hash && m.rating.d == 30.0);
        // Another instrument with that hash isn't the same track.
        s.instrument = "bass".into();
        assert_eq!(rs.find(&s), Err(Miss::NotFound));
    }

    #[test]
    fn length_breaks_ties_and_ambiguity_is_dropped() {
        let rs = Ratings::new(vec![rating("Song", 200_000, 9.0), rating("Song", 260_000, 20.0)]);
        assert_eq!(rs.find(&score("Song", 260_900)).unwrap().rating.d, 20.0);
        assert_eq!(rs.find(&score("Song", 230_000)), Err(Miss::NotFound));
        assert_eq!(rs.find(&score("Song", 0)), Err(Miss::Ambiguous));
        let rs = Ratings::new(vec![rating("Song", 200_000, 9.0), rating("Song", 200_500, 20.0)]);
        assert_eq!(rs.find(&score("Song", 200_000)), Err(Miss::Ambiguous));
        // Two copies of one chart aren't ambiguous.
        let rs = Ratings::new(vec![rating("Song", 200_000, 9.0), rating("Song", 200_500, 9.0)]);
        assert!(rs.find(&score("Song", 200_000)).is_ok());
    }

    #[test]
    fn what_cannot_match() {
        let rs = Ratings::new(vec![Rating { instrument: 4, ..rating("Song", 200_000, 9.0) }]);
        let mut s = score("Song", 200_000);
        s.instrument = "prodrums".into();
        assert!(rs.find(&s).is_ok());
        s.difficulty = "hard".into();
        assert_eq!(rs.find(&s), Err(Miss::NotExpert));
        s.difficulty = "expert".into();
        s.instrument = "guitarghl".into();
        assert_eq!(rs.find(&s), Err(Miss::Instrument));
        s.instrument = "drums".into();
        s.name = "*****".into();
        assert_eq!(rs.find(&s), Err(Miss::Hidden));
    }

    #[test]
    fn harder_than() {
        let rs = Ratings::new((0..10).map(|i| rating("S", 0, f64::from(i))).collect());
        assert_eq!(rs.percentile(&rating("S", 0, 5.0)), 50.0);
    }
}
