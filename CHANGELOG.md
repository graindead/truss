# Changelog

Each release's notes on GitHub are taken from its section here (see
`.github/workflows/release.yml`), so add a `## vX.Y.Z` section before tagging a release.

## v0.1.22

- **Packs:** packs with nothing to download (a rules page, say) are no longer listed.
- **Top bar:** starts with Browse; the "Truss" title is gone.

## v0.1.21

- **Clone Hero's own look:** when Clone Hero is installed, Truss reads its pictures from the
  game's files (once, then keeps them in its cache until the game updates).
- **Chart preview, as the game draws it:** guitar and drums use Clone Hero's camera, lane
  spacing, strike rings, rails, note sizes and timing, sustains (with rounded ends, glowing while
  held), hit flames, sparks and particles, and its track fade, all taken from the game itself.
  Your note colours apply.
- **Note colors:** guitar and drums show every piece built from the game's own layers, or a
  Highway view playing a demo chart; 6-fret and the other colours use the game's art too. Click
  any part to edit its colour. Profiles can be deleted (to the Recycle Bin, after asking).
- **Settings > Chart preview:** note colour profile, frame rate, and note speed and highway
  length as the game has them (1-100, 20-1000%), following your Clone Hero profile until changed.
- **Settings > Directories:** every folder Truss uses in one place: song folders, pack
  downloads, the Clone Hero folder (found from your songs folder when it's in its usual place),
  and the issue scanner's folders.
- **Setup screen:** new users are asked for these folders when Truss first opens (with any it
  can find filled in); after an update that adds one, it's shown once with the new one marked.
- **Smoother previews:** previews now reach their frame rate on Windows (they ran at about 32
  FPS when set to 60).
- **Previews stop when you change tabs.**
- **Less memory:** the library takes about half the memory it did, and the game's pictures and
  the colour editor's are freed when they're not in use.

## v0.1.20

- **Note colors (Tools):** an editor for Clone Hero colour profiles. Open, change and save the
  profiles in your Colors folder, or start a new one from Clone Hero's defaults. Pick colours
  with a colour picker or hex code for guitar, drums, 6-fret and the other colours. Truss keeps
  a backup of each profile before it first saves over it.
- **Live preview:** guitar and drums play a demo chart on the chart preview's highway in your
  colours, with notes, sustains and strike rings that light up on hits, and a "Star power active"
  toggle. The note sprites are downloaded the first time you open the tab. 6-fret and the other
  colours have a preview you can click to pick a part.

## v0.1.19

- **Game setlists (Packs):** every Guitar Hero and Rock Band game's setlist, DLC and extras
  from the community Master List, each in its own folder in Game Setlists.
- **Install chart by chart:** each game download can now be installed from Chorus (a few charts
  from RhythmVerse), skipping the charts you already have. All 4,316 setlist charts have a
  source; the Google Drive archive is still there as "Download archive".
- **Installed downloads:** a download you already have reads "Installed (N charts)", and buttons
  you can't press are greyed out.
- **Guitar Hero III Explicit Patch:** Apply patch puts the uncensored vocals into the GH3 charts
  you have. Any file it would replace is moved into Truss's cache first, and Revert puts the
  originals back.
- **Packs in wrapper folders** ("Rock Band 3/Rock Band 3/...") unpack one level up; tier
  folders inside packs are kept.
- **Testing:** `TRUSS_CACHE_DIR` gives a second copy of Truss its own cache, alongside
  `TRUSS_CONFIG_DIR` for its settings.

## v0.1.18

- **Repair bad songs (Tools):** reads the badsongs.txt Clone Hero writes after a scan and fixes
  what it can without changing how any chart plays: it shortens chart folder names that are too
  long for Windows, renames misnamed chart files (`note.mid`, `song.chart`…) to `notes.*`,
  writes a missing song.ini or song name, and moves loose duplicates whose notes match exactly
  to the Recycle Bin. It never edits notes, never touches the folders inside packs, and undoes
  any fix that would change a chart's leaderboard hashes. Songs it can't fix are listed with
  the reason (a broken download, a notes file Clone Hero can't read…).
- **Undo any repair:** every repair is remembered (renames, new files, the old song.ini text,
  recycled copies), and Repair history can undo one or all of them.

## v0.1.17

- **Better "already downloaded" marks:** Browse now recognises a chart in your library by its
  chart file, and by its notes, not only by artist, title and charter. A pack's copy with its
  song.ini written differently now shows as downloaded when its notes are identical.
- **Other versions:** a yellow ✓ marks songs where you have a different version of the chart
  (an update, say) but not this one.
- **Merge pack duplicates (Library):** finds charts sitting loose in your songs folder that are
  also in a pack, with notes identical track for track, and moves the loose copies to the
  Recycle Bin after you review them. Nothing inside a pack or any other folder is touched.
  After a pack download, Truss offers the same for that pack's charts.
- **Remove videos (Library):** moves every chart's video background to the Recycle Bin.
- **Packs tab:** each pack shows its own progress bar while it downloads; the downloads side
  panel is gone (the floating downloads button still lists everything).
- **Advanced search updates as you go:** results follow the filters as you fill them in, like
  the main search box.
- **Credits:** Settings > Credits has a Chart Sources section for Chorus Encore, RhythmVerse
  and Custom Songs Central, and the Packs tab links to Custom Songs Central.

## v0.1.16

- **Packs tab:** every chart pack, setlist and full album from Custom Songs Central, newest
  first, with categories and a filter. Click a pack to read its description and download it.
- **Download whole packs:** packs go through the download queue into a folder named after the
  pack, keeping the pack's own folders (tiers and all) as they are. Truss downloads from the
  site's own server, Google Drive (files and folders) and MEGA, and unpacks .zip, .7z and .rar
  for you. The Packs tab has its own downloads panel with progress, Cancel, Retry and Open
  folder.
- **Pack folder:** packs can go to their own folder (Settings > Library folders); if you don't
  set one, they go to your songs folder.
- **See which packs you have:** each pack shows Installed, or how many of its charts you have
  ("12 of 18 charts"), wherever your charts are and however they're organised. Hide installed
  hides the ones you already have.
- **Faster Google Drive folders:** charts in Drive folders download several files at a time
  (RhythmVerse charts too), and Drive folders holding documents or archives now work.
- **Fix:** charts whose song.ini has a stray `[song] =` line now show as downloaded.

## v0.1.15

- **Make your own themes:** Tools > Themes. Start from the theme you're using, set each colour
  with a colour picker (click a swatch), and watch a preview as you go. Save and use it, export
  it as a small .json file to share, or import themes other people made. Your themes are listed
  in Settings > Theme too.
- **Themes can set transparency:** a theme can be solid or frosted glass with its own
  see-through amount, or follow your Transparency setting.
- **See-through up to 100%:** the frosted glass slider now goes all the way.
- **New charts without restarting:** Truss checks Encore and RhythmVerse for new uploads on
  start and every 3 minutes, and refreshes the list when you're at the top of it with nothing
  open. RhythmVerse charts still come straight from rhythmverse.co into your own copy.
- **One look:** the Beta UI is now the only UI, and its switch is gone from Settings.

## v0.1.14

- **Faster start-up:** Truss no longer re-reads every chart's song.ini when it checks your
  library for changes. It only opens the ones that changed, so on a big library the check after
  opening takes about a second instead of several (and much less than before after a restart).
- **Library details:** each instrument now gets its own line, so its Fretwork tier is never
  squeezed out.

## v0.1.13

- **New downloads button:** the downloads bar at the bottom is gone. A small button in the
  bottom-right corner shows overall progress (and the library scan) and opens the downloads list
  over the page; it hides when there's nothing to show.
- **Download notices:** one notice per batch that counts up ("Downloaded 23 charts · 27 to go"),
  and one for failures with Retry. Turn them off in Settings > Downloads.
- **Progress where you're looking:** a chart's row shows a thin bar while it downloads, and the
  Download button in the details becomes a progress bar.
- "Download N" for ticked charts sits at the bottom of the results; "Update to vX" moved to the
  top bar.
- **Star Power path images** now draw notes with CHOpt's own sprites, so they look like CHOpt's.

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
