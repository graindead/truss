//! Tools > Star Power paths: finds the best Star Power path for a library chart with the
//! CHOpt port (`crate::chopt`) and shows CHOpt's summary and path image.

use crate::chopt::{self, Difficulty, Instrument, Options, Output, Song};
use crate::library::LocalChart;
use iced::widget::{button, column, container, image, pick_list, row, rule, scrollable, slider, text, text_input, toggler};
use iced::{Alignment, Element, Length, Task};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Rows of measures per image strip (each strip is one texture).
const ROWS_PER_STRIP: u32 = 20;

pub struct Result_ {
    pub output: Output,
    strips: Vec<image::Handle>,
    seconds: f64,
}

impl std::fmt::Debug for Result_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Result_")
    }
}

#[derive(Default)]
pub struct PathTool {
    query: String,
    chart: Option<Arc<LocalChart>>,
    song: Option<Result<Arc<Song>, String>>,
    loading: bool,
    opts: Options,
    video_lag: String,
    speed: String,
    lazy: String,
    delay: String,
    running: Option<Arc<AtomicBool>>,
    run_id: u64,
    result: Option<Result<Arc<Result_>, String>>,
    saved: Option<Result<PathBuf, String>>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Query(String),
    Pick(Arc<LocalChart>),
    Change,
    Loaded(Result<Arc<Song>, String>),
    Instrument(Instrument),
    Difficulty(Difficulty),
    Precision(bool),
    Lefty(bool),
    Squeeze(u32),
    EarlyWhammy(u32),
    VideoLag(String),
    Speed(String),
    Lazy(String),
    Delay(String),
    Run,
    Stop,
    Done(u64, Result<Arc<Result_>, String>),
    Save,
    Saved(Option<Result<PathBuf, String>>),
}

/// Reads a library chart's notes and song.ini (folder or .sng). Read only.
fn load_chart(c: &LocalChart) -> Result<Song, String> {
    let file = c.chart_file.clone().ok_or("This chart has no notes file.")?;
    let is_midi = file.to_ascii_lowercase().ends_with(".mid");
    let (notes, ini) = if c.is_sng() {
        let idx = crate::sng::read_index(&c.path)?;
        let entry = idx.files.iter().find(|e| e.name == file).ok_or("The notes file is missing from the .sng.")?;
        (crate::sng::read_entry(&c.path, &idx, entry)?, crate::sng::generate_ini(&idx.metadata).into_bytes())
    } else {
        let notes = std::fs::read(c.path.join(&file)).map_err(|e| format!("Couldn't read {file}: {e}"))?;
        let ini = std::fs::read_dir(&c.path)
            .ok()
            .and_then(|rd| rd.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("song.ini")))
            .and_then(|e| std::fs::read(e.path()).ok())
            .unwrap_or_default();
        (notes, ini)
    };
    chopt::load(&notes, is_midi, &ini)
}

fn render_strips(out: &Output) -> Vec<image::Handle> {
    let total = chopt::draw::image_height(&out.image);
    let step = ROWS_PER_STRIP * chopt::draw::ROW_HEIGHT;
    let mut strips = Vec::new();
    let mut y = 0;
    while y < total {
        // The first strip also holds the header.
        let end = if y == 0 { (step + 217).min(total) } else { (y + step).min(total) };
        if let Some((w, h, data)) = chopt::draw::render(&out.image, y, end) {
            strips.push(image::Handle::from_rgba(w, h, data));
        }
        y = end;
    }
    strips
}

fn parse_num<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.trim().trim_end_matches("ms").trim_end_matches('%').trim().parse().ok()
}

impl PathTool {
    fn reset_inputs(&mut self) {
        self.video_lag = self.opts.video_lag_ms.to_string();
        self.speed = self.opts.speed.to_string();
        self.lazy = self.opts.lazy_whammy_ms.to_string();
        self.delay = self.opts.whammy_delay_ms.to_string();
    }

    fn stop(&mut self) {
        if let Some(c) = self.running.take() {
            c.store(true, Ordering::Relaxed);
        }
    }

    /// `saved_lag` is the video calibration kept in settings; returns a new one to keep.
    pub fn update(&mut self, m: Msg, saved_lag: i32) -> (Task<Msg>, Option<i32>) {
        let mut keep = None;
        let task = self.update_inner(m, saved_lag, &mut keep);
        (task, keep)
    }

    fn update_inner(&mut self, m: Msg, saved_lag: i32, keep: &mut Option<i32>) -> Task<Msg> {
        match m {
            Msg::Query(q) => self.query = q,
            Msg::Pick(c) => {
                self.stop();
                self.chart = Some(c.clone());
                self.song = None;
                self.result = None;
                self.saved = None;
                self.loading = true;
                if self.speed.is_empty() {
                    self.opts.video_lag_ms = saved_lag;
                    self.reset_inputs();
                }
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || load_chart(&c).map(Arc::new)).await.map_err(|e| e.to_string()).and_then(|r| r)
                    },
                    Msg::Loaded,
                );
            }
            Msg::Change => {
                self.stop();
                self.chart = None;
                self.song = None;
                self.result = None;
                self.saved = None;
            }
            Msg::Loaded(r) => {
                self.loading = false;
                if let Ok(song) = &r {
                    // Keep the chosen part when the chart has it, else its first part.
                    let insts = song.instruments();
                    if !insts.contains(&self.opts.instrument)
                        && let Some(i) = insts.first()
                    {
                        self.opts.instrument = *i;
                    }
                    let diffs = song.difficulties(self.opts.instrument);
                    if !diffs.contains(&self.opts.difficulty)
                        && let Some(d) = diffs.first()
                    {
                        self.opts.difficulty = *d;
                    }
                }
                self.song = Some(r);
            }
            Msg::Instrument(i) => {
                self.opts.instrument = i;
                if let Some(Ok(song)) = &self.song {
                    let diffs = song.difficulties(i);
                    if !diffs.contains(&self.opts.difficulty)
                        && let Some(d) = diffs.first()
                    {
                        self.opts.difficulty = *d;
                    }
                }
            }
            Msg::Difficulty(d) => self.opts.difficulty = d,
            Msg::Precision(b) => self.opts.precision = b,
            Msg::Lefty(b) => self.opts.lefty = b,
            Msg::Squeeze(v) => self.opts.squeeze = v,
            Msg::EarlyWhammy(v) => self.opts.early_whammy = v,
            Msg::VideoLag(s) => {
                if let Some(v) = parse_num::<i32>(&s)
                    && (-200..=200).contains(&v)
                    && v != saved_lag
                {
                    *keep = Some(v);
                }
                self.video_lag = s;
            }
            Msg::Speed(s) => self.speed = s,
            Msg::Lazy(s) => self.lazy = s,
            Msg::Delay(s) => self.delay = s,
            Msg::Run => {
                let Some(Ok(song)) = self.song.clone() else { return Task::none() };
                self.opts.video_lag_ms = parse_num::<i32>(&self.video_lag).unwrap_or(0).clamp(-200, 200);
                self.opts.speed = (parse_num::<u32>(&self.speed).unwrap_or(100).clamp(5, 5000) / 5) * 5;
                self.opts.lazy_whammy_ms = parse_num(&self.lazy).unwrap_or(0);
                self.opts.whammy_delay_ms = parse_num(&self.delay).unwrap_or(0);
                self.reset_inputs();
                self.stop();
                let cancel = Arc::new(AtomicBool::new(false));
                self.running = Some(cancel.clone());
                self.result = None;
                self.saved = None;
                self.run_id += 1;
                let id = self.run_id;
                let (song, opts) = ((*song).clone(), self.opts.clone());
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let t = std::time::Instant::now();
                            let output = chopt::run(song, &opts, &cancel)?;
                            let seconds = t.elapsed().as_secs_f64();
                            let strips = render_strips(&output);
                            Ok(Arc::new(Result_ { output, strips, seconds }))
                        })
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r)
                    },
                    move |r| Msg::Done(id, r),
                );
            }
            Msg::Stop => {
                self.stop();
            }
            Msg::Done(id, r) => {
                // Only the latest run counts; a stopped one reports "Cancelled".
                if id != self.run_id || matches!(&r, Err(e) if e == "Cancelled") {
                    return Task::none();
                }
                self.running = None;
                self.result = Some(r);
            }
            Msg::Save => {
                let Some(Ok(res)) = &self.result else { return Task::none() };
                let res = res.clone();
                let name = format!("{} - {} path.png", sanitize(&res.output.image.artist), sanitize(&res.output.image.song_name));
                return Task::perform(
                    async move {
                        let file = rfd::AsyncFileDialog::new().set_title("Save path image").set_file_name(name).add_filter("PNG image", &["png"]).save_file().await?;
                        let path = file.path().to_path_buf();
                        let p2 = path.clone();
                        Some(
                            tokio::task::spawn_blocking(move || chopt::draw::save_png(&res.output.image, &p2))
                                .await
                                .map_err(|e| e.to_string())
                                .and_then(|r| r)
                                .map(|_| path),
                        )
                    },
                    Msg::Saved,
                );
            }
            Msg::Saved(r) => {
                if r.is_some() {
                    self.saved = r;
                }
            }
        }
        Task::none()
    }

    pub fn view<'a>(&'a self, catalog: &'a [Arc<LocalChart>]) -> Element<'a, Msg> {
        let mut left = column![
            text("Star Power paths").size(15).font(crate::BOLD),
            text("Finds the highest-scoring Star Power path for a chart in your library, using a port of CHOpt by GenericMadScientist. Charts are only read.")
                .size(12),
        ]
        .spacing(10)
        .width(340);

        match &self.chart {
            None => {
                left = left.push(text_input("Search your library…", &self.query).on_input(Msg::Query).padding(8));
                let words: Vec<String> = self.query.to_lowercase().split_whitespace().map(str::to_string).collect();
                if !words.is_empty() {
                    let hits: Vec<&Arc<LocalChart>> = catalog
                        .iter()
                        .filter(|c| {
                            let hay = format!("{} {} {}", c.name, c.artist, c.charter).to_lowercase();
                            words.iter().all(|w| hay.contains(w.as_str()))
                        })
                        .take(40)
                        .collect();
                    if hits.is_empty() {
                        left = left.push(text("No charts match.").size(12));
                    }
                    let list = hits.into_iter().fold(column![].spacing(2), |col, c| {
                        col.push(
                            button(
                                column![
                                    text(c.name.clone()).size(13).wrapping(text::Wrapping::None),
                                    text(format!("{} · {}", c.artist, c.charter)).size(11).wrapping(text::Wrapping::None),
                                ]
                                .spacing(1),
                            )
                            .width(Length::Fill)
                            .padding([5, 8])
                            .style(crate::style::subtle)
                            .on_press(Msg::Pick(c.clone())),
                        )
                    });
                    left = left.push(scrollable(list).height(Length::Fill));
                } else if catalog.is_empty() {
                    left = left.push(text("Your library is empty or still being scanned. Add a songs folder in Settings.").size(12));
                }
            }
            Some(c) => {
                left = left.push(
                    row![
                        column![text(c.name.clone()).size(14).font(crate::BOLD), text(format!("{} · {}", c.artist, c.charter)).size(12)]
                            .spacing(2)
                            .width(Length::Fill),
                        button(text("Change").size(12)).padding([4, 10]).style(crate::style::secondary).on_press(Msg::Change),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
                match &self.song {
                    None => left = left.push(text("Reading chart…").size(12)),
                    Some(Err(e)) => left = left.push(text(e.clone()).size(12).style(text::danger)),
                    Some(Ok(song)) => left = left.push(self.options_view(song)),
                }
            }
        }

        let right: Element<'a, Msg> = match &self.result {
            None if self.running.is_some() => container(text("Finding the best path… long charts can take a while.").size(13)).padding(20).into(),
            None => container(text("Pick a chart and press Find path.").size(13)).padding(20).into(),
            Some(Err(e)) => container(text(e.clone()).size(13).style(text::danger)).padding(20).into(),
            Some(Ok(res)) => {
                let mut top = column![
                    row![
                        text(format!("Found in {:.2} s", res.seconds)).size(12).width(Length::Fill),
                        button(text("Save image…").size(13)).padding([6, 12]).on_press(Msg::Save),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                    container(text(res.output.summary.clone()).size(13).font(iced::Font::MONOSPACE)).padding(10).width(Length::Fill).style(crate::style::panel),
                ]
                .spacing(10);
                match &self.saved {
                    Some(Ok(p)) => top = top.push(text(format!("Saved to {}", p.display())).size(12).style(text::success)),
                    Some(Err(e)) => top = top.push(text(format!("Couldn't save: {e}")).size(12).style(text::danger)),
                    None => {}
                }
                let strips = res.strips.iter().fold(column![], |col, h| col.push(image(h.clone()).width(Length::Fill).filter_method(image::FilterMethod::Linear)));
                scrollable(column![top, container(strips).max_width(1024)].spacing(12).padding(16)).height(Length::Fill).into()
            }
        };

        row![container(left).padding(16).height(Length::Fill), rule::vertical(1).style(crate::style::rule), container(right).width(Length::Fill).height(Length::Fill)]
            .height(Length::Fill)
            .into()
    }

    fn options_view<'a>(&'a self, song: &'a Song) -> Element<'a, Msg> {
        let o = &self.opts;
        let label = |s: &'static str| text(s).size(13).width(110);
        let num_input = |value: &'a str, msg: fn(String) -> Msg, unit: &'static str| {
            row![text_input("0", value).on_input(msg).on_submit(Msg::Run).padding(5).width(80), text(unit).size(12)]
                .spacing(6)
                .align_y(Alignment::Center)
        };
        let insts = song.instruments();
        let diffs = song.difficulties(o.instrument);
        let running = self.running.is_some();
        let go: Element<'a, Msg> = if running {
            row![
                button(text("Finding path…").size(14)).padding([8, 16]),
                button(text("Stop").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(Msg::Stop),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        } else {
            button(text("Find path").size(14)).padding([8, 16]).on_press(Msg::Run).into()
        };
        column![
            row![label("Instrument"), pick_list(insts, Some(o.instrument), Msg::Instrument).width(Length::Fill)].spacing(8).align_y(Alignment::Center),
            row![label("Difficulty"), pick_list(diffs, Some(o.difficulty), Msg::Difficulty).width(Length::Fill)].spacing(8).align_y(Alignment::Center),
            toggler(o.precision).label("Precision mode").text_size(13).on_toggle(Msg::Precision),
            rule::horizontal(1).style(crate::style::rule),
            row![label("Squeeze"), slider(0..=100u32, o.squeeze, Msg::Squeeze).step(5u32), text(format!("{}%", o.squeeze)).size(12).width(40)]
                .spacing(8)
                .align_y(Alignment::Center),
            row![
                label("Early whammy"),
                slider(0..=100u32, o.early_whammy, Msg::EarlyWhammy).step(5u32),
                text(format!("{}%", o.early_whammy)).size(12).width(40)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![label("Video calibration"), num_input(&self.video_lag, Msg::VideoLag, "ms")].spacing(8).align_y(Alignment::Center),
            row![label("Lazy whammy"), num_input(&self.lazy, Msg::Lazy, "ms")].spacing(8).align_y(Alignment::Center),
            row![label("Whammy delay"), num_input(&self.delay, Msg::Delay, "ms")].spacing(8).align_y(Alignment::Center),
            row![label("Song speed"), num_input(&self.speed, Msg::Speed, "%")].spacing(8).align_y(Alignment::Center),
            toggler(o.lefty).label("Lefty flip").text_size(13).on_toggle(Msg::Lefty),
            go,
        ]
        .spacing(10)
        .into()
    }
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect::<String>().trim().to_string()
}
