// Ported from CHOpt / SightRead (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! Songs as CHOpt sees them: note tracks with Star Power phrases and solos, read from .chart
//! and .mid files the way Clone Hero reads them (five-fret instruments).

use super::time::{Bpm, TempoMap, TimeSignature};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const FLAGS_HOPO: u32 = 1 << 3;
pub const FLAGS_TAP: u32 = 1 << 4;
pub const FLAGS_FORCE_FLIP: u32 = 1 << 5;
pub const FLAGS_FORCE_HOPO: u32 = 1 << 6;
pub const FLAGS_FORCE_STRUM: u32 = 1 << 7;
pub const FLAGS_FIVE_FRET_GUITAR: u32 = 1 << 31;

pub const GREEN: usize = 0;
pub const RED: usize = 1;
pub const YELLOW: usize = 2;
pub const BLUE: usize = 3;
pub const ORANGE: usize = 4;
pub const OPEN: usize = 5;

/// No note of that colour.
pub const ABSENT: i32 = -1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    Expert,
}

impl Difficulty {
    pub const ALL: [Difficulty; 4] = [Difficulty::Expert, Difficulty::Hard, Difficulty::Medium, Difficulty::Easy];
    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
            Difficulty::Expert => "Expert",
        }
    }
}

impl std::fmt::Display for Difficulty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Instrument {
    Guitar,
    GuitarCoop,
    Bass,
    Rhythm,
    Keys,
}

impl Instrument {
    pub fn label(self) -> &'static str {
        match self {
            Instrument::Guitar => "Lead Guitar",
            Instrument::GuitarCoop => "Co-op Guitar",
            Instrument::Bass => "Bass",
            Instrument::Rhythm => "Rhythm",
            Instrument::Keys => "Keys",
        }
    }
}

impl std::fmt::Display for Instrument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    pub position: i32,
    /// Sustain length per colour; `ABSENT` when the colour isn't in the note.
    pub lengths: [i32; 7],
    pub flags: u32,
}

impl Note {
    pub fn new(position: i32, colour: usize, length: i32, flags: u32) -> Self {
        let mut lengths = [ABSENT; 7];
        lengths[colour] = length;
        Note { position, lengths, flags }
    }
    pub fn colours(&self) -> u32 {
        self.lengths.iter().enumerate().filter(|(_, l)| **l != ABSENT).fold(0, |a, (i, _)| a | 1 << i)
    }
    pub fn chord_size(&self) -> i32 {
        self.lengths.iter().filter(|l| **l != ABSENT).count() as i32
    }
    fn is_chord(&self) -> bool {
        self.chord_size() >= 2
    }
    pub fn max_length(&self) -> i32 {
        *self.lengths.iter().max().unwrap_or(&0)
    }
    fn merge_non_opens_into_open(&mut self) {
        let open = self.lengths[OPEN];
        if open == ABSENT {
            return;
        }
        for (i, l) in self.lengths.iter_mut().enumerate() {
            if i != OPEN && *l == open {
                *l = ABSENT;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarPower {
    pub position: i32,
    pub length: i32,
}

/// Notes in [start, end) count for the solo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Solo {
    pub start: i32,
    pub end: i32,
    pub value: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigRockEnding {
    pub start: i32,
    pub end: i32,
}

#[derive(Debug, Clone)]
pub struct NoteTrack {
    pub notes: Vec<Note>,
    sp_phrases: Vec<StarPower>,
    solos: Vec<Solo>,
    pub bres: Vec<BigRockEnding>,
    pub resolution: i32,
    base_score_ticks: i32,
}

impl NoteTrack {
    pub fn new(notes: Vec<Note>, resolution: i32, is_from_midi: bool, allow_open_chords: bool, max_hopo_gap: i32) -> Self {
        let mut t = NoteTrack { notes, sp_phrases: Vec::new(), solos: Vec::new(), bres: Vec::new(), resolution, base_score_ticks: 0 };
        t.notes.sort_by_key(|n| n.position);
        t.merge_same_time_notes();
        t.fix_note_overlaps();
        t.compute_base_score_ticks();
        if !allow_open_chords {
            for n in &mut t.notes {
                n.merge_non_opens_into_open();
            }
        }
        t.add_hopos(max_hopo_gap, is_from_midi);
        t
    }

    fn merge_same_time_notes(&mut self) {
        let mut merged: Vec<Note> = Vec::with_capacity(self.notes.len());
        for n in &self.notes {
            match merged.last_mut() {
                Some(m) if m.position == n.position => {
                    for i in 0..7 {
                        m.lengths[i] = m.lengths[i].max(n.lengths[i]);
                    }
                }
                _ => merged.push(*n),
            }
        }
        self.notes = merged;
    }

    fn fix_note_overlaps(&mut self) {
        let mut previous: HashMap<usize, usize> = HashMap::new();
        for idx in 0..self.notes.len() {
            for colour in 0..7 {
                if self.notes[idx].lengths[colour] == ABSENT {
                    continue;
                }
                if let Some(&p) = previous.get(&colour) {
                    let prev_pos = self.notes[p].position;
                    let prev_end = prev_pos + self.notes[p].lengths[colour];
                    let pos = self.notes[idx].position;
                    let cur = &mut self.notes[idx].lengths[colour];
                    *cur = (*cur).max(prev_end - pos);
                    if prev_end > pos {
                        self.notes[p].lengths[colour] = pos - prev_pos;
                    }
                }
                previous.insert(colour, idx);
            }
        }
    }

    fn compute_base_score_ticks(&mut self) {
        let mut total: i64 = 0;
        for n in &self.notes {
            let mut ls: Vec<i32> = n.lengths.iter().copied().filter(|l| *l != ABSENT).collect();
            ls.sort_unstable();
            if ls.first() == ls.last() {
                total += *ls.first().unwrap_or(&0) as i64;
            } else {
                total += ls.iter().map(|l| *l as i64).sum::<i64>();
            }
        }
        let res = self.resolution as i64;
        self.base_score_ticks = ((total * 25 + res - 1) / res) as i32;
    }

    fn add_hopos(&mut self, max_hopo_gap: i32, is_from_midi: bool) {
        for i in 0..self.notes.len() {
            let flags = self.notes[i].flags;
            if flags & FLAGS_TAP != 0 {
                continue;
            }
            if flags & FLAGS_FORCE_HOPO != 0 {
                self.notes[i].flags |= FLAGS_HOPO;
                continue;
            }
            if flags & FLAGS_FORCE_STRUM != 0 {
                continue;
            }
            let mut is_hopo = flags & FLAGS_FORCE_FLIP != 0;
            if i != 0 {
                let (n, p) = (&self.notes[i], &self.notes[i - 1]);
                let gap = n.position - p.position;
                if !n.is_chord()
                    && n.colours() != p.colours()
                    && ((n.colours() & p.colours()) == 0 || !is_from_midi)
                    && gap <= max_hopo_gap
                {
                    is_hopo = !is_hopo;
                }
            }
            if is_hopo {
                self.notes[i].flags |= FLAGS_HOPO;
            }
        }
    }

    pub fn set_sp_phrases(&mut self, mut phrases: Vec<StarPower>) {
        phrases.sort_by_key(|p| p.position);
        phrases.dedup_by_key(|p| p.position);
        for i in 0..phrases.len().saturating_sub(1) {
            let dist = phrases[i + 1].position - phrases[i].position;
            phrases[i + 1].length = phrases[i + 1].length.max(phrases[i].length - dist);
            phrases[i].length = phrases[i].length.min(dist);
        }
        self.sp_phrases = phrases;
    }

    pub fn sp_phrases(&self) -> &[StarPower] {
        &self.sp_phrases
    }

    pub fn set_solos(&mut self, mut solos: Vec<Solo>) {
        solos.sort_by_key(|s| s.start);
        self.solos = solos;
    }

    pub fn solos(&self) -> &[Solo] {
        &self.solos
    }

    pub fn base_score(&self) -> i32 {
        50 * self.notes.iter().map(|n| n.chord_size()).sum::<i32>() + self.base_score_ticks
    }

    /// Whether a note at this tick is inside a Star Power phrase.
    pub fn is_sp_note(&self, position: i32) -> bool {
        let i = self.sp_phrases.partition_point(|p| p.position + p.length <= position);
        i < self.sp_phrases.len() && position >= self.sp_phrases[i].position
    }
}

#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub name: String,
    pub artist: String,
    pub charter: String,
    pub hopo: HopoThreshold,
    pub sustain_cutoff_threshold: Option<i32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum HopoThreshold {
    #[default]
    Resolution,
    Frequency(i32),
    EighthNote,
}

impl HopoThreshold {
    fn chart_max_gap(self, resolution: i32) -> i32 {
        match self {
            HopoThreshold::Frequency(f) => f,
            HopoThreshold::EighthNote => (resolution + 3) / 2,
            HopoThreshold::Resolution => 65 * resolution / 192,
        }
    }
    fn midi_max_gap(self, resolution: i32) -> i32 {
        match self {
            HopoThreshold::Frequency(f) => f,
            HopoThreshold::EighthNote => (resolution + 3) / 2,
            HopoThreshold::Resolution => resolution / 3 + 1,
        }
    }
}

/// Text that may be UTF-8 (with or without a BOM), UTF-16LE with a BOM, or Latin-1.
pub fn to_utf8(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

/// SightRead's whitespace set.
fn skip_whitespace(s: &str) -> &str {
    s.trim_start_matches([' ', '\x0c', '\n', '\r', '\t', '\x0b'])
}

/// The next line (without its line ending), skipping blank space before the line after it,
/// as SightRead reads lines.
fn break_off_newline<'a>(input: &mut &'a str) -> Result<&'a str, String> {
    if input.is_empty() {
        return Err("No lines left".into());
    }
    match input.find('\n') {
        None => {
            let line = *input;
            *input = "";
            Ok(line)
        }
        Some(n) => {
            let end = if n > 0 && input.as_bytes()[n - 1] == b'\r' { n - 1 } else { n };
            let line = &input[..end];
            *input = skip_whitespace(&input[end..]);
            Ok(line)
        }
    }
}

/// An int written exactly (like std::from_chars: optional '-', digits, nothing else).
fn strict_int(s: &str) -> Option<i32> {
    if s.starts_with('+') {
        return None;
    }
    s.parse().ok()
}

/// std::stoi: leading whitespace, optional sign, digits, anything after.
fn stoi(s: &str) -> Result<Option<i32>, String> {
    let t = skip_whitespace(s);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let n = digits.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 {
        return Ok(None);
    }
    let v: i64 = digits[..n].parse().map_err(|_| "Resolution out of range")?;
    let v = if neg { -v } else { v };
    i32::try_from(v).map(Some).map_err(|_| "Resolution out of range".to_string())
}

pub fn parse_ini(data: &[u8]) -> Metadata {
    let text = to_utf8(data);
    let mut m = Metadata { name: "Unknown Song".into(), artist: "Unknown Artist".into(), charter: "Unknown Charter".into(), ..Default::default() };
    const KEYS: [&str; 10] = [
        "artist",
        "charter",
        "eighthnote_hopo",
        "frets",
        "hammer_on_measure_scale",
        "hopo_frequency",
        "name",
        "sustain_cutoff_threshold",
        "pro_drums",
        "five_lane_drums",
    ];
    let mut rest = text.as_str();
    while let Ok(line) = break_off_newline(&mut rest) {
        for key in KEYS {
            let Some(after) = line.strip_prefix(key) else { continue };
            let Some(value) = skip_whitespace(after).strip_prefix('=') else { continue };
            let value = skip_whitespace(value);
            match key {
                "name" => m.name = value.to_string(),
                "artist" => m.artist = value.to_string(),
                "charter" | "frets" => {
                    if !value.is_empty() {
                        m.charter = value.to_string();
                    }
                }
                "hopo_frequency" => {
                    if let Some(v) = strict_int(value) {
                        m.hopo = HopoThreshold::Frequency(v);
                    }
                }
                "eighthnote_hopo" => {
                    if let Some(v) = strict_int(value) {
                        m.hopo = if v == 0 { HopoThreshold::Resolution } else { HopoThreshold::EighthNote };
                    }
                }
                "sustain_cutoff_threshold" => {
                    if let Some(v) = strict_int(value) {
                        m.sustain_cutoff_threshold = Some(v);
                    }
                }
                _ => {}
            }
        }
    }
    m
}

#[derive(Debug, Clone)]
pub struct PracticeSection {
    pub name: String,
    pub start: i32,
}

#[derive(Debug, Clone)]
pub struct Song {
    pub tempo_map: TempoMap,
    pub resolution: i32,
    pub name: String,
    pub artist: String,
    pub charter: String,
    pub practice_sections: Vec<PracticeSection>,
    pub tracks: BTreeMap<(Instrument, Difficulty), NoteTrack>,
}

impl Song {
    fn new(meta: &Metadata) -> Self {
        Song {
            tempo_map: TempoMap::default(),
            resolution: 192,
            name: meta.name.clone(),
            artist: meta.artist.clone(),
            charter: meta.charter.clone(),
            practice_sections: Vec::new(),
            tracks: BTreeMap::new(),
        }
    }

    fn add_track(&mut self, inst: Instrument, diff: Difficulty, track: NoteTrack) {
        if !track.notes.is_empty() {
            self.tracks.entry((inst, diff)).or_insert(track);
        }
    }

    pub fn instruments(&self) -> Vec<Instrument> {
        let set: BTreeSet<Instrument> = self.tracks.keys().map(|k| k.0).collect();
        set.into_iter().collect()
    }

    pub fn difficulties(&self, inst: Instrument) -> Vec<Difficulty> {
        Difficulty::ALL.into_iter().filter(|d| self.tracks.contains_key(&(inst, *d))).collect()
    }

    pub fn speedup(&mut self, speed: i32) {
        if speed == 100 || speed <= 0 {
            return;
        }
        self.name = format!("{} ({speed}%)", self.name);
        self.tempo_map = self.tempo_map.speedup(speed);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SoloBehaviour {
    PreferLaterStarts,
    PreferEarlierStarts,
}

fn combine_solo_events(on: &[i32], off: &[i32], behaviour: SoloBehaviour) -> Vec<(i32, i32)> {
    let mut ranges = Vec::new();
    match behaviour {
        SoloBehaviour::PreferLaterStarts => {
            let mut ons: BTreeSet<i32> = on.iter().copied().collect();
            for &o in off {
                let Some(&start) = ons.range(..=o).next_back() else { continue };
                ranges.push((start, o));
                ons.remove(&start);
            }
        }
        SoloBehaviour::PreferEarlierStarts => {
            let (mut i, mut j) = (0, 0);
            while i < on.len() && j < off.len() {
                if on[i] >= off[j] {
                    j += 1;
                    continue;
                }
                ranges.push((on[i], off[j]));
                while i < on.len() && on[i] < off[j] {
                    i += 1;
                }
            }
        }
    }
    ranges
}

fn form_solos(on: &[i32], off: &[i32], notes: &[Note], behaviour: SoloBehaviour, is_midi: bool) -> Vec<Solo> {
    let mut ranges = combine_solo_events(on, off, behaviour);
    if !is_midi {
        for r in &mut ranges {
            r.1 += 1;
        }
    }
    ranges.sort();
    for i in 0..ranges.len().saturating_sub(1) {
        ranges[i + 1].1 = ranges[i + 1].1.max(ranges[i].1);
        ranges[i].1 = ranges[i].1.min(ranges[i + 1].0);
    }
    ranges
        .into_iter()
        .filter_map(|(start, end)| {
            let positions: BTreeSet<i32> = notes.iter().map(|n| n.position).filter(|p| *p >= start && *p < end).collect();
            (!positions.is_empty()).then(|| Solo { start, end, value: 100 * positions.len() as i32 })
        })
        .collect()
}

// ---------- .chart ----------

struct ChartSection<'a> {
    name: &'a str,
    kv: HashMap<&'a str, String>,
    bpms: Vec<(i32, i32)>,
    tses: Vec<(i32, i32, i32)>,
    events: Vec<(i32, String)>,
    notes: Vec<(i32, i32, i32)>,
    specials: Vec<(i32, i32, i32)>,
}

fn parse_chart_sections(text: &str) -> Result<Vec<ChartSection<'_>>, String> {
    let mut rest = text;
    let mut sections = Vec::new();
    let int = |s: &str| strict_int(s).ok_or_else(|| format!("Bad number in .chart: {s}"));
    while !rest.is_empty() {
        let header = break_off_newline(&mut rest)?;
        if header.is_empty() {
            return Err("Header string empty".into());
        }
        let name = header.get(1..header.len().saturating_sub(1).max(1)).unwrap_or("");
        if break_off_newline(&mut rest)? != "{" {
            return Err("Section does not open with {".into());
        }
        let mut s = ChartSection { name, kv: HashMap::new(), bpms: vec![], tses: vec![], events: vec![], notes: vec![], specials: vec![] };
        loop {
            let line = break_off_newline(&mut rest)?;
            if line == "}" {
                break;
            }
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() < 3 {
                continue;
            }
            match strict_int(parts[0]) {
                Some(pos) => match parts[2] {
                    "N" | "S" => {
                        if parts.len() < 5 {
                            return Err("Line incomplete".into());
                        }
                        let v = (pos, int(parts[3])?, int(parts[4])?);
                        if parts[2] == "N" { s.notes.push(v) } else { s.specials.push(v) }
                    }
                    "B" => {
                        if parts.len() < 4 {
                            return Err("Line incomplete".into());
                        }
                        s.bpms.push((pos, int(parts[3]).map_err(|_| "Bad BPM event")?));
                    }
                    "TS" => {
                        if parts.len() < 4 {
                            return Err("Line incomplete".into());
                        }
                        let den = if parts.len() >= 5 { int(parts[4])? } else { 2 };
                        s.tses.push((pos, int(parts[3])?, den));
                    }
                    "E" => {
                        if parts.len() < 4 {
                            return Err("Line incomplete".into());
                        }
                        s.events.push((pos, parts[3..].join(" ")));
                    }
                    _ => {}
                },
                None => {
                    s.kv.insert(parts[0], parts[2..].concat());
                }
            }
        }
        sections.push(s);
    }
    Ok(sections)
}

fn chart_instrument(header: &str) -> Option<(Difficulty, Instrument)> {
    let diff = [("Easy", Difficulty::Easy), ("Medium", Difficulty::Medium), ("Hard", Difficulty::Hard), ("Expert", Difficulty::Expert)]
        .into_iter()
        .find(|(n, _)| header.starts_with(n))?
        .1;
    let inst = [
        ("Single", Instrument::Guitar),
        ("DoubleGuitar", Instrument::GuitarCoop),
        ("DoubleBass", Instrument::Bass),
        ("DoubleRhythm", Instrument::Rhythm),
        ("Keyboard", Instrument::Keys),
    ]
    .into_iter()
    .find(|(n, _)| header.ends_with(n))?
    .1;
    Some((diff, inst))
}

fn practice_section_name(s: &str) -> Option<&str> {
    ["section ", "section_", "prc_"].into_iter().find_map(|p| s.strip_prefix(p))
}

pub fn parse_chart(data: &[u8], meta: &Metadata) -> Result<Song, String> {
    let text = to_utf8(data);
    let sections = parse_chart_sections(&text)?;
    let mut song = Song::new(meta);
    let mut coda: Option<i32> = None;
    for sec in &sections {
        match sec.name {
            "Song" => {
                if let Some(r) = stoi(sec.kv.get("Resolution").map_or("192", |v| v.as_str()))? {
                    if r <= 0 {
                        return Err("Resolution must be positive".into());
                    }
                    song.resolution = r;
                }
            }
            "SyncTrack" => {
                let bpms = sec.bpms.iter().map(|&(p, b)| Bpm { position: p, millibeats_per_minute: b as f64 }).collect::<Vec<_>>();
                if bpms.iter().any(|b| b.millibeats_per_minute <= 0.0) {
                    return Err("BPMs must be positive".into());
                }
                let mut tses = Vec::new();
                for &(p, n, d) in &sec.tses {
                    if d >= 32 {
                        return Err("Invalid Time Signature denominator".into());
                    }
                    let den = if d < 0 { 0 } else { 1i32.wrapping_shl(d as u32) };
                    if n <= 0 || den <= 0 {
                        return Err("Time signatures must be positive/positive".into());
                    }
                    tses.push(TimeSignature { position: p, numerator: n, denominator: den });
                }
                song.tempo_map = TempoMap::new(tses, bpms, vec![], song.resolution);
            }
            "Events" => {
                for (pos, data) in &sec.events {
                    if let Some(d) = data.strip_prefix('"').and_then(|d| d.strip_suffix('"'))
                        && let Some(n) = practice_section_name(d)
                    {
                        song.practice_sections.push(PracticeSection { name: n.to_string(), start: *pos });
                    }
                    if data == "\"coda\"" || data == "\"[coda]\"" {
                        coda = coda.or(Some(*pos));
                    }
                }
            }
            name => {
                let Some((diff, inst)) = chart_instrument(name) else { continue };
                let mut notes = Vec::new();
                let (mut forcing, mut taps) = (BTreeSet::new(), BTreeSet::new());
                for &(pos, fret, len) in &sec.notes {
                    let colour = match fret {
                        0 => Some(GREEN),
                        1 => Some(RED),
                        2 => Some(YELLOW),
                        3 => Some(BLUE),
                        4 => Some(ORANGE),
                        7 => Some(OPEN),
                        _ => None,
                    };
                    match colour {
                        Some(c) => notes.push(Note::new(pos, c, len, FLAGS_FIVE_FRET_GUITAR)),
                        None if fret == 5 => {
                            forcing.insert(pos);
                        }
                        None if fret == 6 => {
                            taps.insert(pos);
                        }
                        None => {}
                    }
                }
                for n in &mut notes {
                    if taps.contains(&n.position) {
                        n.flags |= FLAGS_TAP;
                    } else if forcing.contains(&n.position) {
                        n.flags |= FLAGS_FORCE_FLIP;
                    }
                }
                let sp = sec.specials.iter().filter(|s| s.1 == 2).map(|&(p, _, l)| StarPower { position: p, length: l }).collect();
                let mut on: Vec<i32> = sec.events.iter().filter(|e| e.1 == "solo").map(|e| e.0).collect();
                let mut off: Vec<i32> = sec.events.iter().filter(|e| e.1 == "soloend").map(|e| e.0).collect();
                on.sort_unstable();
                off.sort_unstable();
                let solos = form_solos(&on, &off, &notes, SoloBehaviour::PreferLaterStarts, false);
                let mut track = NoteTrack::new(notes, song.resolution, false, true, meta.hopo.chart_max_gap(song.resolution));
                track.set_sp_phrases(sp);
                track.set_solos(solos);
                song.add_track(inst, diff, track);
            }
        }
    }
    let _ = coda; // big rock endings only count on drums in .chart files
    if song.tracks.is_empty() {
        return Err("This chart has no guitar, bass, rhythm or keys notes.".into());
    }
    Ok(song)
}

// ---------- .mid ----------

#[derive(Clone, Copy)]
struct Toggle {
    position: i32,
    velocity: u8,
    channel: u8,
    rank: i32,
}

#[derive(Clone, Default)]
struct OnOff {
    on: Vec<Toggle>,
    off: Vec<Toggle>,
    last_rank: i32,
}

impl OnOff {
    fn add_on(&mut self, position: i32, velocity: u8, channel: u8) {
        self.last_rank += 1;
        self.on.push(Toggle { position, velocity, channel, rank: self.last_rank });
    }
    fn add_off(&mut self, position: i32, velocity: u8, channel: u8) {
        self.last_rank += 1;
        self.off.push(Toggle { position, velocity, channel, rank: self.last_rank });
    }
    /// (position, length, velocity) of each matched on/off pair.
    fn combined(&self, expand_zero: bool) -> Vec<(i32, i32, u8)> {
        let mut out = Vec::new();
        let mut unmatched: HashMap<u8, Vec<Toggle>> = HashMap::new();
        let mut i = 0;
        for off in &self.off {
            while i < self.on.len() && self.on[i].rank < off.rank {
                unmatched.entry(self.on[i].channel).or_default().push(self.on[i]);
                i += 1;
            }
            let Some(start) = unmatched.entry(off.channel).or_default().pop() else { continue };
            let mut end = off.position;
            if start.position == end && expand_zero {
                end += 1;
            }
            out.push((start.position, end - start.position, start.velocity));
        }
        out
    }
    fn interval_set(&self) -> IntervalSet {
        IntervalSet::new(self.combined(true).into_iter().map(|(p, l, _)| (p, p + l)).collect())
    }
}

/// Merged half-open intervals.
#[derive(Default)]
struct IntervalSet(Vec<(i32, i32)>);

impl IntervalSet {
    fn new(mut v: Vec<(i32, i32)>) -> Self {
        v.sort();
        let mut out: Vec<(i32, i32)> = Vec::new();
        let mut i = 0;
        while i < v.len() {
            let mut cur = v[i];
            let mut j = i + 1;
            while j < v.len() && v[j].0 <= cur.1 {
                cur.1 = cur.1.max(v[j].1);
                j += 1;
            }
            if cur.0 < cur.1 {
                out.push(cur);
            }
            i = j;
        }
        IntervalSet(out)
    }
    fn contains(&self, p: i32) -> bool {
        let i = self.0.partition_point(|iv| iv.1 < p);
        i < self.0.len() && self.0[i].0 <= p && p < self.0[i].1
    }
}

/// (tick, order within the track)
type EventPos = (i32, i32);

fn combine_positions(on: &[EventPos], off: &[EventPos], expand_zero: bool) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut stack: Vec<EventPos> = Vec::new();
    let mut i = 0;
    for o in off {
        while i < on.len() && on[i].1 < o.1 {
            stack.push(on[i]);
            i += 1;
        }
        let Some(s) = stack.pop() else { continue };
        let mut end = o.0;
        if s.0 == end && expand_zero {
            end += 1;
        }
        out.push((s.0, end));
    }
    out
}

#[derive(Default)]
struct MidiInstrumentTrack {
    keys: BTreeMap<u8, OnOff>,
    open_on: BTreeMap<Difficulty, Vec<EventPos>>,
    open_off: BTreeMap<Difficulty, Vec<EventPos>>,
    tap_on: BTreeMap<Difficulty, Vec<EventPos>>,
    tap_off: BTreeMap<Difficulty, Vec<EventPos>>,
    enhanced_opens: bool,
}

impl MidiInstrumentTrack {
    fn key(&self, k: u8) -> OnOff {
        self.keys.get(&k).cloned().unwrap_or_default()
    }
    fn use_solos_for_sp(&self) -> bool {
        let solo_ons = self.keys.get(&103).map_or(0, |e| e.on.len());
        solo_ons > 1 && self.keys.get(&116).is_none_or(|e| e.on.is_empty())
    }
}

fn sysex_diffs(d: u8) -> Vec<Difficulty> {
    match d {
        0 => vec![Difficulty::Easy],
        1 => vec![Difficulty::Medium],
        2 => vec![Difficulty::Hard],
        3 => vec![Difficulty::Expert],
        0xFF => vec![Difficulty::Easy, Difficulty::Medium, Difficulty::Hard, Difficulty::Expert],
        _ => vec![],
    }
}

/// One MIDI track event, read the way SightRead reads them.
enum MidiEvent<'a> {
    Midi { status: u8, data: [u8; 2] },
    Sysex(&'a [u8]),
    Meta { kind: u8, data: &'a [u8] },
}

type MidiTrack<'a> = Vec<(i32, MidiEvent<'a>)>;

fn pop(r: &mut &[u8]) -> Result<u8, String> {
    let (&b, tail) = r.split_first().ok_or("Insufficient bytes in .mid file")?;
    *r = tail;
    Ok(b)
}

fn read_vlq(r: &mut &[u8]) -> Result<i32, String> {
    let mut n: i32 = 0;
    let mut read = 0;
    while r.first().is_some_and(|b| b & 0x80 != 0) {
        read += 1;
        if read >= 4 {
            return Err("Too long variable length number".into());
        }
        n = (n << 7) | (pop(r)? & 0x7F) as i32;
    }
    n = (n << 7) | (pop(r)? & 0x7F) as i32;
    Ok(n)
}

/// A standard MIDI file reader matching SightRead: format 1 only, and running status carries
/// over meta and SysEx events (Harmonix charts rely on this).
fn read_smf(data: &[u8]) -> Result<(i32, Vec<MidiTrack<'_>>), String> {
    if data.len() < 14 {
        return Err("Insufficient bytes in .mid file".into());
    }
    if data[..10] != [0x4D, 0x54, 0x68, 0x64, 0, 0, 0, 6, 0, 1] {
        return Err("Invalid .mid file".into());
    }
    let ntracks = i16::from_be_bytes([data[10], data[11]]);
    let division = i16::from_be_bytes([data[12], data[13]]);
    if division < 0 {
        return Err("Only ticks per quarter-note is supported".into());
    }
    let mut rest = &data[14..];
    let mut tracks = Vec::new();
    let mut i = 0;
    while i < ntracks as i32 && !rest.is_empty() {
        i += 1;
        if rest.len() < 8 || rest[..4] != *b"MTrk" {
            return Err("Invalid .mid file".into());
        }
        let size = i32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]);
        if size < 0 || size as usize > rest.len() - 8 {
            return Err("Insufficient bytes in .mid file".into());
        }
        let mut body = &rest[8..8 + size as usize];
        rest = &rest[8 + size as usize..];
        let mut time: i32 = 0;
        let mut prev_status: Option<u8> = None;
        let mut track = Vec::new();
        while !body.is_empty() {
            time = time.wrapping_add(read_vlq(&mut body)?);
            let first = *body.first().ok_or("Insufficient bytes in .mid file")?;
            let ev = if first == 0xFF {
                body = &body[1..];
                let kind = pop(&mut body)?;
                let len = read_vlq(&mut body)? as usize;
                if len > body.len() {
                    return Err("Meta Event too long".into());
                }
                let (d, tail) = body.split_at(len);
                body = tail;
                MidiEvent::Meta { kind, data: d }
            } else if first == 0xF0 {
                body = &body[1..];
                let len = read_vlq(&mut body)? as usize;
                if len > body.len() {
                    return Err("Sysex Event too long".into());
                }
                let (d, tail) = body.split_at(len);
                body = tail;
                MidiEvent::Sysex(d)
            } else {
                let status = if first & 0x80 != 0 {
                    body = &body[1..];
                    first
                } else {
                    prev_status.ok_or("MIDI Event has no status byte and there is no running status")?
                };
                if status & 0xF0 == 0xF0 {
                    return Err("MIDI Events with high nibble 0xF are not supported".into());
                }
                let a = pop(&mut body)?;
                let b = if status & 0xF0 != 0xC0 && status & 0xF0 != 0xD0 { pop(&mut body)? } else { 0 };
                prev_status = Some(status);
                MidiEvent::Midi { status, data: [a, b] }
            };
            track.push((time, ev));
        }
        tracks.push(track);
    }
    Ok((division as i32, tracks))
}

fn read_instrument_track(events: &[(i32, MidiEvent<'_>)]) -> MidiInstrumentTrack {
    let mut t = MidiInstrumentTrack::default();
    let mut rank = 0;
    for (time, ev) in events {
        rank += 1;
        match ev {
            MidiEvent::Midi { status, data: [key, vel] } => {
                let ch = status & 0x0F;
                match status & 0xF0 {
                    0x90 => {
                        let e = t.keys.entry(*key).or_default();
                        if *vel == 0 { e.add_off(*time, 0, ch) } else { e.add_on(*time, *vel, ch) }
                    }
                    0x80 => t.keys.entry(*key).or_default().add_off(*time, *vel, ch),
                    _ => {}
                }
            }
            MidiEvent::Sysex(data) => {
                let d = *data;
                if d.len() != 8 || d[0] != 0x50 || d[1] != 0x53 || d[2] != 0 || d[3] != 0 || d[7] != 0xF7 {
                    continue;
                }
                let on = d[6] != 0;
                // Open notes: type 1; taps: type 4.
                let (ons, offs) = if d[5] == 1 && d[4] <= 3 && d[6] <= 1 {
                    (&mut t.open_on, &mut t.open_off)
                } else if d[5] == 4 && (d[4] <= 3 || d[4] == 0xFF) && d[6] <= 1 {
                    (&mut t.tap_on, &mut t.tap_off)
                } else {
                    continue;
                };
                for diff in sysex_diffs(d[4]) {
                    if on { ons.entry(diff).or_default().push((*time, rank)) } else { offs.entry(diff).or_default().push((*time, rank)) }
                }
            }
            MidiEvent::Meta { kind: 1, data } if *data == b"[ENHANCED_OPENS]" => t.enhanced_opens = true,
            _ => {}
        }
    }
    t
}

fn midi_difficulty(key: u8, enhanced: bool) -> Option<Difficulty> {
    let ranges = if enhanced {
        [(95, 100, Difficulty::Expert), (83, 88, Difficulty::Hard), (71, 76, Difficulty::Medium), (59, 64, Difficulty::Easy)]
    } else {
        [(96, 100, Difficulty::Expert), (84, 88, Difficulty::Hard), (72, 76, Difficulty::Medium), (60, 64, Difficulty::Easy)]
    };
    ranges.into_iter().find(|(lo, hi, _)| (*lo..=*hi).contains(&key)).map(|r| r.2)
}

fn midi_colour(key: u8, enhanced: bool) -> Option<usize> {
    let (mins, colours): ([u8; 4], &[usize]) =
        if enhanced { ([95, 83, 71, 59], &[OPEN, GREEN, RED, YELLOW, BLUE, ORANGE]) } else { ([96, 84, 72, 60], &[GREEN, RED, YELLOW, BLUE, ORANGE]) };
    mins.into_iter().find(|m| key >= *m && ((key - m) as usize) < colours.len()).map(|m| colours[(key - m) as usize])
}

fn midi_note_tracks(
    events: &[(i32, MidiEvent<'_>)],
    meta: &Metadata,
    resolution: i32,
    coda: Option<i32>,
) -> Result<BTreeMap<Difficulty, NoteTrack>, String> {
    let t = read_instrument_track(events);
    if t.open_on.keys().any(|d| !t.open_off.contains_key(d)) {
        return Err("No open Note Off events".into());
    }
    if t.tap_on.keys().any(|d| !t.tap_off.contains_key(d)) {
        return Err("No tap Note Off events".into());
    }
    let sustain_cutoff = meta.sustain_cutoff_threshold.unwrap_or(resolution / 3 + 1);
    let bres: Vec<BigRockEnding> = t
        .key(120)
        .combined(false)
        .into_iter()
        .filter(|(p, _, _)| coda.is_some_and(|c| c <= *p))
        .map(|(p, l, _)| BigRockEnding { start: p, end: p + l })
        .collect();
    let open_events: BTreeMap<Difficulty, IntervalSet> = t
        .open_on
        .iter()
        .map(|(d, ons)| (*d, IntervalSet::new(combine_positions(ons, t.open_off.get(d).map_or(&[][..], |v| v), true))))
        .collect();
    let mut notes: BTreeMap<Difficulty, Vec<Note>> = BTreeMap::new();
    for (key, ev) in &t.keys {
        let Some(diff) = midi_difficulty(*key, t.enhanced_opens) else { continue };
        let Some(base) = midi_colour(*key, t.enhanced_opens) else { continue };
        for (pos, len, _) in ev.combined(false) {
            let length = if len > sustain_cutoff { len } else { 0 };
            let colour = if open_events.get(&diff).is_some_and(|s| s.contains(pos)) { OPEN } else { base };
            notes.entry(diff).or_default().push(Note::new(pos, colour, length, FLAGS_FIVE_FRET_GUITAR));
        }
    }
    // Forcing.
    let taps = t.key(104).interval_set();
    let tap_sysex: BTreeMap<Difficulty, IntervalSet> = t
        .tap_on
        .iter()
        .map(|(d, ons)| (*d, IntervalSet::new(combine_positions(ons, t.tap_off.get(d).map_or(&[][..], |v| v), false))))
        .collect();
    for (diff, set) in notes.iter_mut() {
        let base = match diff {
            Difficulty::Easy => 65,
            Difficulty::Medium => 77,
            Difficulty::Hard => 89,
            Difficulty::Expert => 101,
        };
        let force_hopo = t.key(base).interval_set();
        let force_strum = t.key(base + 1).interval_set();
        for n in set.iter_mut() {
            if taps.contains(n.position) || tap_sysex.get(diff).is_some_and(|s| s.contains(n.position)) {
                n.flags |= FLAGS_TAP;
            }
            if force_hopo.contains(n.position) {
                n.flags |= FLAGS_FORCE_HOPO;
            }
            if force_strum.contains(n.position) {
                n.flags |= FLAGS_FORCE_STRUM;
            }
        }
        set.sort_by_key(|n| (n.position, n.colours()));
    }
    let sp_key = if t.use_solos_for_sp() { 103 } else { 116 };
    let sp: Vec<StarPower> = t.key(sp_key).combined(false).into_iter().map(|(p, l, _)| StarPower { position: p, length: l }).collect();
    let solo_events = if t.use_solos_for_sp() { OnOff::default() } else { t.key(103) };
    let solo_on: Vec<i32> = solo_events.on.iter().map(|e| e.position).collect();
    let solo_off: Vec<i32> = solo_events.off.iter().map(|e| e.position).collect();
    let tracks = notes
        .into_iter()
        .map(|(diff, set)| {
            let solos = form_solos(&solo_on, &solo_off, &set, SoloBehaviour::PreferEarlierStarts, true);
            let mut track = NoteTrack::new(set, resolution, true, true, meta.hopo.midi_max_gap(resolution));
            track.set_sp_phrases(sp.clone());
            track.set_solos(solos);
            track.bres = bres.clone();
            (diff, track)
        })
        .collect();
    Ok(tracks)
}

pub fn parse_midi(data: &[u8], meta: &Metadata) -> Result<Song, String> {
    let (resolution, tracks) = read_smf(data)?;
    if resolution == 0 {
        return Err("Resolution must be > 0".into());
    }
    let mut song = Song::new(meta);
    song.resolution = resolution;
    let Some(first) = tracks.first() else { return Err("This chart has no guitar, bass, rhythm or keys notes.".into()) };
    let (mut bpms, mut tses) = (Vec::new(), Vec::new());
    for (time, ev) in first {
        match ev {
            MidiEvent::Meta { kind: 0x51, data } => {
                if data.len() < 3 {
                    return Err("Tempo meta event too short".into());
                }
                let us = ((data[0] as u32) << 16) | ((data[1] as u32) << 8) | data[2] as u32;
                bpms.push(Bpm { position: *time, millibeats_per_minute: 60_000_000_000.0 / us as f64 });
            }
            MidiEvent::Meta { kind: 0x58, data } => {
                if data.len() < 2 {
                    return Err("Time signature meta event too short".into());
                }
                if data[1] >= 32 {
                    return Err("Time sig denominator too large".into());
                }
                tses.push(TimeSignature { position: *time, numerator: data[0] as i32, denominator: 1i32.wrapping_shl(data[1] as u32) });
            }
            _ => {}
        }
    }
    if bpms.iter().any(|b| b.millibeats_per_minute <= 0.0) {
        return Err("BPMs must be positive".into());
    }
    if tses.iter().any(|t| t.numerator <= 0 || t.denominator <= 0) {
        return Err("Time signatures must be positive/positive".into());
    }
    song.tempo_map = TempoMap::new(tses, bpms, vec![], resolution);
    let mut named: BTreeMap<String, usize> = BTreeMap::new();
    for (i, tr) in tracks.iter().enumerate() {
        let name = tr.iter().find_map(|(_, ev)| match ev {
            MidiEvent::Meta { kind: 3, data } => Some(String::from_utf8_lossy(data).into_owned()),
            _ => None,
        });
        if let Some(n) = name {
            named.entry(n).or_insert(i);
        }
    }
    let mut coda = None;
    if let Some(&i) = named.get("EVENTS") {
        for (time, kind) in &tracks[i] {
            let MidiEvent::Meta { kind: 1, data: b } = kind else { continue };
            let text = String::from_utf8_lossy(b);
            if text == "[coda]" {
                coda = Some(*time);
            }
            let inner = text.strip_suffix(']').unwrap_or(&text);
            if let Some(n) = ["[section ", "[section_", "[prc_"].into_iter().find_map(|p| inner.strip_prefix(p)) {
                song.practice_sections.push(PracticeSection { name: n.to_string(), start: *time });
            }
        }
    }
    let od_beats: Vec<i32> = named
        .get("BEAT")
        .map(|&i| {
            tracks[i]
                .iter()
                .filter_map(|(time, k)| match k {
                    MidiEvent::Midi { status, data: [key, vel] } if status & 0xF0 == 0x90 && *vel != 0 && (*key == 12 || *key == 13) => {
                        Some(*time)
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    for (name, &i) in &named {
        let inst = match name.as_str() {
            "PART GUITAR" | "T1 GEMS" => Instrument::Guitar,
            "PART GUITAR COOP" => Instrument::GuitarCoop,
            "PART BASS" => Instrument::Bass,
            "PART RHYTHM" => Instrument::Rhythm,
            "PART KEYS" => Instrument::Keys,
            _ => continue,
        };
        for (diff, track) in midi_note_tracks(&tracks[i], meta, resolution, coda)? {
            song.add_track(inst, diff, track);
        }
    }
    if !od_beats.is_empty() {
        song.tempo_map = song.tempo_map.with_od_beats(od_beats);
    }
    if song.tracks.is_empty() {
        return Err("This chart has no guitar, bass, rhythm or keys notes.".into());
    }
    Ok(song)
}

#[cfg(test)]
pub fn make_note(position: i32, length: i32, colour: usize) -> Note {
    Note::new(position, colour, length, FLAGS_FIVE_FRET_GUITAR)
}

#[cfg(test)]
pub fn make_chord(position: i32, colours: &[(usize, i32)]) -> Note {
    let mut n = Note::new(position, colours[0].0, colours[0].1, FLAGS_FIVE_FRET_GUITAR);
    for &(c, l) in &colours[1..] {
        n.lengths[c] = l;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART: &str = "[Song]\n{\n  Resolution = 192\n}\n[SyncTrack]\n{\n  0 = TS 4\n  0 = B 120000\n}\n[Events]\n{\n  0 = E \"section Intro\"\n}\n[ExpertSingle]\n{\n  0 = N 0 0\n  192 = N 1 0\n  192 = N 2 0\n  288 = N 3 100\n  288 = N 5 0\n  384 = N 4 0\n  384 = S 2 200\n  500 = E solo\n  600 = N 0 0\n  700 = E soloend\n}\n";

    #[test]
    fn reads_a_chart() {
        let song = parse_chart(CHART.as_bytes(), &parse_ini(b"[song]\nname = Test\nartist = Me\n")).unwrap();
        assert_eq!(song.name, "Test");
        assert_eq!(song.practice_sections[0].name, "Intro");
        let t = &song.tracks[&(Instrument::Guitar, Difficulty::Expert)];
        assert_eq!(t.notes.len(), 5);
        assert_eq!(t.notes[1].colours(), 0b110);
        // Forced note 288 (blue after red/yellow chord): naturally a HOPO-candidate gap of 96 > 65, forced flips to HOPO.
        assert!(t.notes[2].flags & FLAGS_HOPO != 0);
        assert_eq!(t.sp_phrases(), &[StarPower { position: 384, length: 200 }]);
        assert_eq!(t.solos(), &[Solo { start: 500, end: 701, value: 100 }]);
        assert_eq!(t.base_score(), 50 * 6 + (100 * 25 + 191) / 192);
    }
}
