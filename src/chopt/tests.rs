// Tests ported from CHOpt's optimiser_unittest.cpp (overlap guitar paths).
// Copyright (C) 2020-2026 Raymond Wright. Licensed under the GNU GPL v3.0 or later.

use super::engine::Engine;
use super::optimiser::Optimiser;
use super::points::PathingSettings;
use super::processed::{Path, ProcessedSong};
use super::song::*;
use super::time::{SpMode, SpTimeMap, TempoMap};
use std::sync::atomic::AtomicBool;

fn note(pos: i32) -> Note {
    make_note(pos, 0, GREEN)
}
fn sus(pos: i32, len: i32) -> Note {
    make_note(pos, len, GREEN)
}
fn gry(pos: i32) -> Note {
    make_chord(pos, &[(GREEN, 0), (RED, 0), (YELLOW, 0)])
}
fn sp(position: i32, length: i32) -> StarPower {
    StarPower { position, length }
}

fn track(notes: Vec<Note>, phrases: Vec<StarPower>) -> NoteTrack {
    let mut t = NoteTrack::new(notes, 192, false, false, 65);
    t.set_sp_phrases(phrases);
    t
}

fn optimise_with(notes: Vec<Note>, phrases: Vec<StarPower>, s: PathingSettings) -> (Path, ProcessedSong) {
    let t = track(notes, phrases);
    let tm = SpTimeMap::new(TempoMap::default(), SpMode::Measure);
    let song = ProcessedSong::new(&t, &tm, &[], &s);
    let cancel = AtomicBool::new(false);
    let path = Optimiser::new(&song, &cancel, s.whammy_delay).optimal_path().unwrap();
    (path, song)
}

fn optimise(notes: Vec<Note>, phrases: Vec<StarPower>) -> Path {
    optimise_with(notes, phrases, PathingSettings::new(Engine::clone_hero(false))).0
}

fn check_act(path: &Path, i: usize, start: usize, end: usize, sp_start: f64, sp_end: f64) {
    let a = &path.activations[i];
    assert_eq!((a.act_start, a.act_end), (start, end), "act {i} points");
    assert!((a.sp_start - sp_start).abs() < 0.01, "act {i} sp_start {} vs {sp_start}", a.sp_start);
    assert!((a.sp_end - sp_end).abs() < 0.01, "act {i} sp_end {} vs {sp_end}", a.sp_end);
}

#[test]
fn simplest_song_with_a_non_empty_path() {
    let p = optimise(vec![note(0), note(192), note(384)], vec![sp(0, 50), sp(192, 50)]);
    assert_eq!(p.score_boost, 50);
    assert_eq!(p.activations.len(), 1);
    check_act(&p, 0, 2, 2, 2.0, 18.0);
}

#[test]
fn simplest_song_with_multiple_acts() {
    let p = optimise(
        vec![note(0), note(192), gry(384), note(3840), note(4032), gry(10368)],
        vec![sp(0, 50), sp(192, 50), sp(3840, 50), sp(4032, 50)],
    );
    assert_eq!(p.score_boost, 300);
    assert_eq!(p.activations.len(), 2);
    check_act(&p, 0, 2, 2, 2.0, 18.0);
    check_act(&p, 1, 5, 5, 54.0, 70.0);
}

#[test]
fn simplest_song_with_an_act_containing_more_than_one_note() {
    let p = optimise(vec![note(0), note(192), note(384), note(576)], vec![sp(0, 50), sp(192, 50)]);
    assert_eq!(p.score_boost, 100);
    check_act(&p, 0, 2, 3, 2.0, 18.0);
}

#[test]
fn simplest_song_with_an_act_that_must_go_as_long_as_possible() {
    let p = optimise(vec![note(0), note(192), note(384), note(3360)], vec![sp(0, 50), sp(192, 50)]);
    assert_eq!(p.score_boost, 100);
    check_act(&p, 0, 2, 3, 2.0, 18.0);
}

#[test]
fn simplest_song_where_greedy_algorithm_fails() {
    let p = optimise(
        vec![note(0), note(192), note(384), make_chord(3840, &[(GREEN, 0), (RED, 0)])],
        vec![sp(0, 50), sp(192, 50)],
    );
    assert_eq!(p.score_boost, 100);
    check_act(&p, 0, 3, 3, 20.0, 36.0);
}

#[test]
fn simplest_song_where_a_phrase_must_be_hit_early() {
    let p = optimise(
        vec![note(0), note(192), note(384), note(3224), note(9378), note(15714), note(15715)],
        vec![sp(0, 50), sp(192, 50), sp(3224, 50), sp(9378, 50)],
    );
    assert_eq!(p.score_boost, 150);
    check_act(&p, 0, 2, 2, 0.8958, 16.8958);
    check_act(&p, 1, 5, 6, 81.84375, 97.84375);
}

#[test]
fn simplest_song_where_activations_ending_late_matter() {
    let mut notes = vec![note(0), note(192), note(384), sus(3234, 1440)];
    notes.extend((10944..=10953).map(note));
    let p = optimise(notes, vec![sp(0, 50), sp(192, 50), sp(3234, 50)]);
    assert_eq!(p.score_boost, 750);
    assert_eq!(p.activations.len(), 1);
}

#[test]
fn early_whammy_at_start_of_an_sp_phrase_is_always_counted() {
    let p = optimise(vec![sus(0, 1420), note(1500), note(1600)], vec![sp(0, 1550)]);
    assert_eq!(p.score_boost, 50);
    assert_eq!(p.activations.len(), 1);
}

#[test]
fn whammy_just_before_the_activation_point_is_not_double_counted() {
    let p = optimise(vec![sus(192, 1440), note(1632), note(6336)], vec![sp(192, 1), sp(1632, 1)]);
    assert!(p.score_boost < 100);
}

#[test]
fn activation_right_after_a_sp_sustain_is_drawn_correctly() {
    let p = optimise(vec![sus(0, 1488), sus(2880, 3264)], vec![sp(0, 1)]);
    assert!(p.activations[0].sp_start >= 15.0);
}

#[test]
fn songs_ending_in_es1_are_pathed_correctly() {
    let p = optimise(
        vec![note(0), note(192), note(384), note(576), note(768), note(4032), note(4224)],
        vec![sp(0, 50), sp(192, 50), sp(4032, 50)],
    );
    assert_eq!(p.score_boost, 150);
    assert_eq!(p.activations.len(), 1);
}

#[test]
fn compressed_whammy_is_specified_correctly() {
    let p = optimise(
        vec![sus(192, 192), note(672), gry(1000), note(3840), note(9984), gry(10176)],
        vec![sp(192, 50), sp(672, 50), sp(3840, 50), sp(9984, 50)],
    );
    let a = p.activations[0];
    assert_eq!(p.score_boost, 300);
    assert_eq!(p.activations.len(), 2);
    assert!(a.whammy_end > 1.06 && a.whammy_end < 1.74, "{}", a.whammy_end);
    assert!(a.sp_start < 3.6);
}

#[test]
fn acts_covering_the_last_note_do_not_compress_whammy() {
    let p = optimise(vec![sus(0, 1536), note(1728)], vec![sp(0, 50)]);
    assert!(p.activations[0].whammy_end > 16.0);
}

#[test]
fn use_next_point_to_work_out_compressed_whammy() {
    let p = optimise(
        vec![note(0), note(192), gry(384), note(3350), note(3360), note(9504), gry(9696)],
        vec![sp(0, 50), sp(192, 50), sp(3350, 50), sp(9504, 50)],
    );
    assert!(p.activations[0].whammy_end > 17.45, "{}", p.activations[0].whammy_end);
}

#[test]
fn forbidden_squeeze_does_not_grant_extra_whammy_next_act() {
    let p = optimise(
        vec![note(0), note(192), note(768), sus(3840, 192), note(4224), sus(19200, 192), note(38400), note(41990)],
        vec![sp(0, 1), sp(192, 1), sp(3840, 576), sp(19200, 1)],
    );
    assert_eq!(p.score_boost, 200);
}

#[test]
fn easier_activations_are_chosen_where_possible() {
    let p = optimise(vec![note(0), note(192), note(384), note(3504), note(9600), note(12672)], vec![sp(0, 1), sp(192, 1)]);
    assert!(p.activations[0].sp_start > 20.0);
}

#[test]
fn early_whammy_from_a_note_cannot_be_obtained_until_the_note_is_hit() {
    let p = optimise(vec![sus(0, 1392), sus(1536, 192)], vec![sp(0, 1), sp(1536, 1)]);
    assert_eq!(p.score_boost, 28);
}

#[test]
fn does_not_crash_with_positive_video_lag() {
    let mut s = PathingSettings::new(Engine::clone_hero(false));
    s.video_lag = 0.1;
    let (p, _) = optimise_with(vec![sus(192, 192)], vec![sp(192, 1)], s);
    assert_eq!(p.score_boost, 0);
}

#[test]
fn whammy_delay_is_handled_correctly() {
    let mut notes = vec![note(0), note(192), note(768), sus(3840, 1420), note(5376)];
    notes.extend((0..7).map(|i| note(13056 + 192 * i)));
    let mut s = PathingSettings::new(Engine::clone_hero(false));
    s.whammy_delay = 0.1;
    let (p, _) = optimise_with(notes, vec![sp(0, 1), sp(192, 1), sp(3840, 1728)], s);
    assert_eq!(p.activations.len(), 2);
    assert_eq!(p.score_boost, 550);
}

#[test]
fn act_end_is_constrained_by_next_act_start() {
    let mut notes = vec![sus(0, 1440), note(1536), sus(4608, 1420), note(6144)];
    notes.extend((19200..=19206).map(note));
    let p = optimise(notes, vec![sp(0, 1), sp(4608, 1537)]);
    assert!(p.activations[0].sp_end < 23.9, "{}", p.activations[0].sp_end);
}

#[test]
fn whammy_delay_to_stop_early_halfbar_is_considered() {
    let mut notes = vec![note(0), sus(192, 1536)];
    notes.extend((0..10).map(|i| note(1728 + 192 * i)));
    notes.push(note(5476));
    notes.extend((0..5).map(|i| note(19200 + 192 * i)));
    let p = optimise(notes, vec![sp(0, 1), sp(192, 1540), sp(5476, 1), sp(19200, 1)]);
    assert_eq!(p.activations.len(), 2);
    assert_eq!(p.score_boost, 1000);
}

#[test]
fn summary_matches_chopt_format() {
    let (p, song) = optimise_with(
        vec![note(0), note(192), gry(384), note(3840), note(4032), gry(10368)],
        vec![sp(0, 50), sp(192, 50), sp(3840, 50), sp(4032, 50)],
        PathingSettings::new(Engine::clone_hero(false)),
    );
    let s = song.path_summary(&p);
    assert!(s.starts_with("Path: 2-2\nNo SP score: "), "{s}");
    assert!(s.contains("Average multiplier: "), "{s}");
    assert!(s.contains("\n2: NN (GRY)"), "{s}");
}

/// Runs the optimiser over real charts when TRUSS_TEST_LIBRARY is set (read only), printing
/// timings and any failures.
#[test]
fn real_library_paths() {
    let Some(dir) = std::env::var_os("TRUSS_TEST_LIBRARY") else { return };
    let limit: usize = std::env::var("TRUSS_TEST_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    let lib = crate::library::scan_blocking(&[std::path::PathBuf::from(dir)]);
    let (mut ok, mut failed, mut slowest) = (0, 0, (0.0f64, String::new()));
    let total = std::time::Instant::now();
    for c in lib.catalog.iter().filter(|c| !c.is_sng()).take(limit) {
        let Some(f) = c.chart_file.as_ref() else { continue };
        let Ok(notes) = std::fs::read(c.path.join(f)) else { continue };
        let ini = std::fs::read(c.path.join("song.ini")).unwrap_or_default();
        let song = match super::load(&notes, f.to_lowercase().ends_with(".mid"), &ini) {
            Ok(s) => s,
            Err(e) => {
                println!("LOAD {}: {e}", c.path.display());
                failed += 1;
                continue;
            }
        };
        let Some(&(inst, diff)) = song.tracks.keys().find(|k| k.1 == Difficulty::Expert) else { continue };
        let opts = super::Options { instrument: inst, difficulty: diff, ..Default::default() };
        let t = std::time::Instant::now();
        let cancel = AtomicBool::new(false);
        match super::run(song, &opts, &cancel) {
            Ok(out) => {
                ok += 1;
                let secs = t.elapsed().as_secs_f64();
                if secs > slowest.0 {
                    slowest = (secs, format!("{} | {}", c.path.display(), out.summary.lines().next().unwrap_or("")));
                }
            }
            Err(e) => {
                println!("RUN {}: {e}", c.path.display());
                failed += 1;
            }
        }
    }
    println!("{ok} ok, {failed} failed in {:?}; slowest {:.2}s: {}", total.elapsed(), slowest.0, slowest.1);
}

#[test]
fn profile_one_chart() {
    let Some(dir) = std::env::var_os("CHOPT_PROFILE") else { return };
    let dir = std::path::PathBuf::from(dir);
    let (f, midi) = if dir.join("notes.mid").exists() { ("notes.mid", true) } else { ("notes.chart", false) };
    let song = super::load(&std::fs::read(dir.join(f)).unwrap(), midi, &std::fs::read(dir.join("song.ini")).unwrap_or_default()).unwrap();
    let t = std::time::Instant::now();
    let out = super::run(song, &super::Options::default(), &AtomicBool::new(false)).unwrap();
    eprintln!("total {:?}\n{}", t.elapsed(), out.summary.lines().take(4).collect::<Vec<_>>().join("\n"));
}

#[test]
fn render_one_chart() {
    let (Some(dir), Some(out)) = (std::env::var_os("CHOPT_RENDER"), std::env::var_os("CHOPT_OUT")) else { return };
    let dir = std::path::PathBuf::from(dir);
    let (f, midi) = if dir.join("notes.mid").exists() { ("notes.mid", true) } else { ("notes.chart", false) };
    let song = super::load(&std::fs::read(dir.join(f)).unwrap(), midi, &std::fs::read(dir.join("song.ini")).unwrap_or_default()).unwrap();
    let mut opts = super::Options::default();
    if std::env::var_os("CHOPT_MEDIUM").is_some() {
        opts.difficulty = Difficulty::Medium;
    }
    let out_img = super::run(song, &opts, &AtomicBool::new(false)).unwrap();
    eprintln!("{}", out_img.summary);
    let t = std::time::Instant::now();
    super::draw::save_png(&out_img.image, std::path::Path::new(&out)).unwrap();
    eprintln!("drawn in {:?}", t.elapsed());
}

/// Writes the port's summaries for a spread of library charts (CHOPT_DUMP=dir), for comparing
/// with CHOpt's own output.
#[test]
fn dump_for_comparison() {
    let (Some(lib), Some(out)) = (std::env::var_os("TRUSS_TEST_LIBRARY"), std::env::var_os("CHOPT_DUMP")) else { return };
    let out = std::path::PathBuf::from(out);
    let lib = crate::library::scan_blocking(&[std::path::PathBuf::from(lib)]);
    let charts: Vec<_> = lib.catalog.iter().filter(|c| !c.is_sng() && c.chart_file.is_some()).collect();
    let step = (charts.len() / 100).max(1);
    let (mut list, mut ours) = (String::new(), String::new());
    for c in charts.iter().step_by(step).take(100) {
        let f = c.chart_file.as_ref().unwrap();
        let file = c.path.join(f);
        let ini = std::fs::read(c.path.join("song.ini")).unwrap_or_default();
        let Ok(song) = super::load(&std::fs::read(&file).unwrap(), f.to_lowercase().ends_with(".mid"), &ini) else { continue };
        if !song.tracks.contains_key(&(Instrument::Guitar, Difficulty::Expert)) {
            continue;
        }
        let opts = if std::env::var_os("CHOPT_ALT").is_some() {
            super::Options { precision: true, squeeze: 50, early_whammy: 30, video_lag_ms: 20, lazy_whammy_ms: 10, whammy_delay_ms: 20, speed: 120, ..Default::default() }
        } else {
            super::Options::default()
        };
        let Ok(o) = super::run(song, &opts, &AtomicBool::new(false)) else { continue };
        list += &format!("{}\n", file.display());
        ours += &format!("### {}\n{}\n", file.display(), o.summary);
    }
    std::fs::write(out.join("list.txt"), list).unwrap();
    std::fs::write(out.join("ours.txt"), ours).unwrap();
}

/// Like `dump_for_comparison`, but every five-fret part and difficulty of CHOPT_N charts
/// spread over the library, recording load/run errors too. Lines in list.txt are
/// "file|instrument|difficulty".
#[test]
fn dump_all_parts() {
    let (Some(lib), Some(out)) = (std::env::var_os("TRUSS_TEST_LIBRARY"), std::env::var_os("CHOPT_DUMP_ALL")) else { return };
    let n: usize = std::env::var("CHOPT_N").ok().and_then(|v| v.parse().ok()).unwrap_or(500);
    let out = std::path::PathBuf::from(out);
    let lib = crate::library::scan_blocking(&[std::path::PathBuf::from(lib)]);
    let charts: Vec<_> = lib.catalog.iter().filter(|c| !c.is_sng() && c.chart_file.is_some()).collect();
    let step = (charts.len() / n).max(1);
    let flag = |i: Instrument| match i {
        Instrument::Guitar => "guitar",
        Instrument::GuitarCoop => "coop",
        Instrument::Bass => "bass",
        Instrument::Rhythm => "rhythm",
        Instrument::Keys => "keys",
    };
    let (mut list, mut ours, mut loads) = (String::new(), String::new(), String::new());
    for c in charts.iter().step_by(step).take(n) {
        let f = c.chart_file.as_ref().unwrap();
        let file = c.path.join(f);
        let ini = std::fs::read(c.path.join("song.ini")).unwrap_or_default();
        let song = match super::load(&std::fs::read(&file).unwrap(), f.to_lowercase().ends_with(".mid"), &ini) {
            Ok(s) => s,
            Err(e) => {
                loads += &format!("{}|{e}\n", file.display());
                continue;
            }
        };
        for &(inst, diff) in song.tracks.keys() {
            let opts = super::Options { instrument: inst, difficulty: diff, ..Default::default() };
            let key = format!("{}|{}|{}", file.display(), flag(inst), diff.label().to_lowercase());
            let summary = match super::run(song.clone(), &opts, &AtomicBool::new(false)) {
                Ok(o) => o.summary,
                Err(e) => format!("ERROR {e}"),
            };
            list += &format!("{key}\n");
            ours += &format!("### {key}\n{summary}\n");
        }
    }
    std::fs::write(out.join("list.txt"), list).unwrap();
    std::fs::write(out.join("ours.txt"), ours).unwrap();
    std::fs::write(out.join("load_errors.txt"), loads).unwrap();
}

#[test]
fn midi_event_counts() {
    let Some(f) = std::env::var_os("CHOPT_MIDI") else { return };
    let data = std::fs::read(f).unwrap();
    let smf = midly::Smf::parse(&data).unwrap();
    for (i, t) in smf.tracks.iter().enumerate() {
        let notes = t.iter().filter(|e| matches!(e.kind, midly::TrackEventKind::Midi { .. })).count();
        let last: Vec<String> = t.iter().rev().take(3).map(|e| format!("{:?}", e.kind)).collect();
        eprintln!("track {i}: {} events, {notes} midi; last {:?}", t.len(), last);
    }
}

/// Runs the "file|instrument|difficulty" lines in CHOPT_LIST, writing ours.txt beside it.
#[test]
fn run_list() {
    let Some(list) = std::env::var_os("CHOPT_LIST") else { return };
    let list = std::path::PathBuf::from(list);
    let mut ours = String::new();
    for line in std::fs::read_to_string(&list).unwrap().lines().filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = line.split('|').collect();
        let file = std::path::Path::new(parts[0]);
        let inst = match parts[1] {
            "guitar" => Instrument::Guitar,
            "coop" => Instrument::GuitarCoop,
            "bass" => Instrument::Bass,
            "rhythm" => Instrument::Rhythm,
            _ => Instrument::Keys,
        };
        let diff = Difficulty::ALL.into_iter().find(|d| d.label().eq_ignore_ascii_case(parts[2])).unwrap();
        let ini = std::fs::read(file.parent().unwrap().join("song.ini")).unwrap_or_default();
        let summary = super::load(&std::fs::read(file).unwrap(), parts[0].to_lowercase().ends_with(".mid"), &ini)
            .and_then(|song| {
                let mut o = super::Options { instrument: inst, difficulty: diff, ..Default::default() };
                if std::env::var_os("CHOPT_SQZ_EW").is_some() {
                    o = super::Options { squeeze: 30, early_whammy: 100, ..o };
                }
                if std::env::var_os("CHOPT_ALT").is_some() {
                    o = super::Options { precision: true, squeeze: 50, early_whammy: 30, video_lag_ms: 20, lazy_whammy_ms: 10, whammy_delay_ms: 20, speed: 120, ..o };
                }
                super::run(song, &o, &AtomicBool::new(false))
            })
            .map_or_else(|e| format!("ERROR {e}"), |o| o.summary);
        ours += &format!("### {line}\n{summary}\n");
    }
    std::fs::write(list.with_file_name("ours.txt"), ours).unwrap();
}
