# Truss

A native Rust rewrite of [Bridge](https://github.com/Geomitron/Bridge) (formerly "Bridge RS"):
search the Chorus Encore chart database and download charts straight into your
Clone Hero / YARG songs folder. No Electron.

On top of Bridge:

- **RhythmVerse** as a second chart source (Settings > Chart sources): its Clone Hero charts
  appear alongside Encore's (newest first, mixed by upload date), and download or preview from
  RhythmVerse, Google Drive or MediaFire (.zip, .7z, .rar, .sng). Truss keeps a local copy of
  RhythmVerse's chart list for instant search. RhythmVerse has no public API, so this may stop
  working if the site changes.
- **Scene Switcher** for OBS (Tools tab): switches scenes when Clone Hero starts and ends a song,
  shows the Ouvert request queue, Next song via Ouvert or a Twitch chat command; settings shared
  with the standalone [CH Scene Switcher](https://github.com/graindead/chswitcher).
- **Leaderboards**: the top 50 Clone Hero scores for a chart, including RhythmVerse charts and
  Encore charts whose listed hashes are out of date (Truss reads the chart and works them out).
- **Fretwork tiers**: hover a difficulty to see where the chart lands on Fretwork's scale, or turn
  them on next to every difficulty in Settings.
- **Frosted glass** (beta, Windows): the window blurs what's behind it (Settings > Appearance >
  Transparency). Needs small patches to iced, kept in `vendor/` (see `vendor/README.md`).
- **Library** page for the charts you already have, with tick boxes and a ✓ on search results
  you already own.

## Build

```
cargo build --release
```

The binary is `target/release/truss.exe` (single file, ~20 MB). To build without RhythmVerse:
`cargo build --release --no-default-features`.

## Layout

- `src/api.rs`: Encore search API client and album-art fetch (thumbnails decoded off the UI thread)
- `src/sng.rs`: streaming `.sng` extractor (writes files to disk as bytes arrive)
- `src/download.rs`: download pipeline, folder naming, filename sanitizing
- `src/settings.rs`: settings stored in `%APPDATA%\truss\config\settings.json` (moved from the old `bridge-rs` folders on first start)
- `src/chart.rs`: `.chart`/`.mid` note parser ported from scan-chart (verified against Encore's track hashes)
- `src/audio.rs`: multi-stem audio playback (libopus + symphonia, lock-free buffer, sample-accurate clock)
- `src/textures.rs`: preview artwork, cached on disk after first use
- `src/preview.rs`, `src/preview_panel.rs`: chart preview renderer and player controls
- `src/advanced.rs`: advanced search form (text/exact/exclude filters, ranges, feature flags)
- `src/library.rs`: scans library folders (song.ini / .sng metadata): the local chart catalog and "Already in your library"
- `src/library_ui.rs`: Library page (missing art/background/video/lyrics, duplicates, search, delete to Recycle Bin)
- `src/tools_ui.rs`: Tools page (Scene Switcher; the Issue scanner is hidden for now)
- `src/scenes.rs`, `src/scenes_ui.rs`: OBS scene switcher (OBS WebSocket 5), Ouvert queue, Twitch sign-in
- `src/rhythmverse.rs`, `src/rv_catalog.rs`, `src/rv_ui.rs`: RhythmVerse search, local chart list (SQLite), downloads
- `src/zip_ranges.rs`: fetches only the needed files from a remote .zip (skips videos) with HTTP range requests
- `src/lyrics.rs`: which library charts have lyrics (cached)
- `src/issues.rs`: chart issue scanner (port of scan-chart's checks) and the Excel report
- `src/vlist.rs`: virtualized list for 30k+ row tables
- `src/extras.rs`: chart details: library badge, detected issues, version picker, Google Drive link
- `src/main.rs`: iced UI (click column headers to sort; Ctrl/Shift-click to select several, then
  download them through the queue, 2 at a time)

## Updates and releases

Truss checks this repo's latest GitHub release when it starts (Settings > Updates), and can
download it, verify its SHA-256 and restart into it.

To publish a release, set `version` in `Cargo.toml`, commit, then push a matching tag:

```
git tag v0.2.0
git push origin main v0.2.0
```

GitHub Actions (`.github/workflows/release.yml`) runs the tests, builds `truss.exe` and publishes
it with `truss.exe.sha256`.

## Tests

```
cargo test --release
```

Network test that checks the parser against Encore: `TRUSS_TEST_ENCORE=1 cargo test --release track_hashes`

Compare the issue scanner with Encore's results: `TRUSS_TEST_ENCORE=1 cargo test --release issues_match_encore -- --nocapture`

Time a scan of a real library: `TRUSS_TEST_LIBRARY="C:/path/to/Songs" cargo test --release scan_real -- --nocapture`

## Not yet ported from Bridge

Report issue, UI zoom, custom themes, update checker.

## Credits and licence

Licensed GPL-3.0 (see `LICENSE`), like [Bridge](https://github.com/Geomitron/Bridge), which it
is a port of. The chart parser, issue checks and preview are ported from Geomitron's
[scan-chart](https://github.com/Geomitron/scan-chart) and
[chart-preview](https://github.com/Geomitron/chart-preview) (MIT). Fretwork difficulty tiers are
ported from Staycation's [Fretwork](https://github.com/Staycation44/fretwork) (MIT). The full list,
including the libraries Truss is built with, is under Settings > Credits.
