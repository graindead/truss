# Changelog

Each release's notes on GitHub are taken from its section here (see
`.github/workflows/release.yml`), so add a `## vX.Y.Z` section before tagging a release.

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
