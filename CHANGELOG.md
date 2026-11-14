# Changelog

Each release's notes on GitHub are taken from its section here (see
`.github/workflows/release.yml`), so add a `## vX.Y.Z` section before tagging a release.

## v0.1.12

- **Star Power paths:** a new tool (first in Tools) that works out the best Star Power path
  for a chart in your library, like CHOpt, and draws it. It's a Rust port of CHOpt 1.19 built
  into Truss, so nothing else needs installing, and its results match CHOpt's. Clone Hero
  scoring (standard and precision) for guitar, bass, rhythm, keys and co-op, with squeeze,
  early whammy, video calibration, speed and lefty options, and Save image.
- **Library details:** click a chart on the Library page to open its details below it: album
  art, song info, difficulties with Fretwork tiers, a preview played from your own copy, Open
  folder, and the leaderboard.
- **Difficulty graphs:** the details show Fretwork's difficulty-over-the-song graph (the same
  one as Truss Index), with the values under the mouse.
- Truss now opens inside the screen's usable area instead of partly under the taskbar.

## v0.1.11

- **Library shows instantly:** Truss remembers your library between runs, so "in your library"
  ticks and the Library page are ready as soon as it opens. It still rescans in the background to
  pick up changes.
- **Scan progress:** a progress bar at the bottom of the window shows how far a library scan has
  got.

## v0.1.10

- **Coloured names:** song names, artists and charters show the colours and bold/italic that
  charters gave them for Clone Hero (on the Library page, and wherever a chart's own text has
  them). Turn it off in Settings.
- **Runs on any Windows PC:** Truss no longer needs the Visual C++ Redistributable installed.
- Fixed charts losing their "in your library" tick when they finished downloading while Truss
  was still scanning the library at start-up.

## v0.1.9

- **Sharper chart preview:** the whole fret row fits with room to spare, the highway and notes
  are smooth instead of grainy, notes fade in at the far end instead of flashing, and the
  preview has a proper rounded border.
- **No downloading charts you already have:** previews, leaderboards and Fretwork tiers read
  the copy in your library (RhythmVerse and Encore charts), so they start instantly.
  RhythmVerse charts you have show their leaderboard and tiers without clicking Find scores.

## v0.1.8

- **Sort the Library:** a Sort menu on the Library page (artist, song, charter, recently added,
  oldest added) and an Added column with the date each chart arrived. Click Added to switch
  between newest and oldest first. Your choice is remembered.
- **Faster album art:** covers you've seen before are saved on your PC and show up instantly
  at start-up, when you search again and when you scroll back, instead of downloading again.

## v0.1.7

- **More Fretwork search filters** (Advanced search, Encore charts only): difficulty tier,
  harder or easier than the chart's listed difficulty, hand movement (low / medium / high),
  pacing (steady or with hard spikes), and for drums kick speed and a double-kick tier. Hover
  the ⓘ next to each for what it means.
- **Advanced search reworked:** Song, Chart and Fretwork in their own cards with Features
  below, easier-to-see Exact / Exclude checkboxes.
- **RhythmVerse in Advanced search:** its charts are included when you only use filters its
  chart list has (name, artist, album, genre, year, charter, length, date); otherwise a note
  says why they're left out. Reset now goes back to the normal results.
- **See-through slider** for frosted glass (Settings > Appearance).
- **Select all** skips charts you already have.
- **Changelog** in Settings > Updates.
- Album art keeps loading after scrolling far down and back (covers load nearest-first, a few
  at a time, and retry if a download fails).
- Scrolling far down no longer turns into a long run of only RhythmVerse charts.

## v0.1.6

- **Frosted glass (beta, Windows):** Settings > Appearance > Transparency. The window blurs and
  tints whatever is behind it. Takes effect after a restart; not available in low memory mode.
- **New defaults for new installs:** Beta UI on, Fretwork tiers shown next to difficulties,
  frosted glass on, background videos off. If you've already changed these, your choices are
  kept.
- A settings file saved with a byte-order mark (e.g. edited in PowerShell or Notepad) is now
  read instead of being ignored.

## v0.1.5

- **Themed taskbar icon:** pinned Truss buttons and Truss shortcuts on the desktop and in the
  Start menu now change colour with the theme. (The icon of `truss.exe` itself can't change.)
- **More RhythmVerse charts download:** charts whose link points back to their own RhythmVerse
  page used to say "open the link"; they now download in Truss.
- **Clearer errors:** when a chart's Google Drive, MediaFire, direct or Encore link is dead,
  Truss says the chart is no longer available instead of showing a raw HTTP error.

## v0.1.4

- **New themes:** Kusha (blue) and Nick (purple).
- **Themed window icon:** the window and taskbar icon recolour to match the theme.
- **Removed tools:** Art Studio, Lyrics and Video Sync. Tools now holds the Scene Switcher.
  The Library page still shows which charts have lyrics.

## v0.1.3

- **Leaderboards:** the top 50 Clone Hero scores for the selected chart, for Encore and
  RhythmVerse charts (RhythmVerse charts need a click on "Find scores").
- **Fretwork tiers:** hover a difficulty to see where the chart lands on Fretwork's scale, or
  show them next to every difficulty (Settings > Appearance).
- **Clickable details:** click an artist, album, genre, year or charter to search for it.
- **Small windows:** album art scales down, details are more compact, and the list and top
  bar fit narrow windows.
- Buttons and links no longer get cut off, and tooltips have a solid background.
- **Credits:** a new section at the bottom of Settings.

## v0.1.2

- The local RhythmVerse chart list is no longer rebuilt from scratch when something goes
  wrong: it's backed up after each update and restored from the backup if damaged, and a
  list another copy of Truss is using is waited for instead of being deleted.

## v0.1.1

- **Beta UI:** a quieter look across the app (Settings > Appearance), with text tabs,
  hairline dividers, muted secondary text and a slimmer selected-row highlight, in the
  colours of the current theme.

## v0.1.0

- First release: search Chorus Encore (and RhythmVerse) and download charts straight into your
  Clone Hero / YARG songs folder, with chart preview, a Library page, the OBS Scene Switcher,
  and automatic updates.
