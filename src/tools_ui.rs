//! The Tools page: the Scene Switcher (and the Issue scanner, hidden for now).

use crate::vlist::VList;
use futures::{SinkExt, channel::mpsc::Sender};
use iced::widget::{button, column, container, progress_bar, row, rule, scrollable, text};
use iced::{Alignment, Border, Element, Length, Subscription, Task, Theme};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The Issue scanner is hidden for now (a full library produces more rows than a spreadsheet
/// can hold). Its code stays so it can be revamped and switched back on.
const ISSUE_SCANNER_ENABLED: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Scenes,
    Issues,
}

/// What the page reads from the app.
pub struct Ctx<'a> {
    pub settings: &'a crate::settings::Settings,
}

/// Requests for the app.
pub enum Out {
    None,
    Open(PathBuf),
    SetIssuePaths(Option<PathBuf>, Option<PathBuf>),
}

fn toggle_chip<'a, M: Clone + 'a>(label: &'static str, active: bool, msg: M) -> Element<'a, M> {
    button(text(label).size(12))
        .padding([3, 10])
        .style(move |t: &Theme, s| {
            if crate::style::refined() {
                return crate::style::quiet_button(active)(t, s);
            }
            let mut b = if active { button::primary(t, s) } else { button::secondary(t, s) };
            b.border = Border::default().rounded(if crate::style::refined() { 5 } else { 12 });
            b
        })
        .on_press(msg)
        .into()
}

fn heading<'a, M: 'a>(s: &'a str) -> Element<'a, M> {
    text(s).size(15).font(crate::BOLD).into()
}

fn note<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into()).size(12).into()
}

// ---------- Issue scanner ----------

#[derive(Default)]
struct IssueTool {
    progress: Option<(usize, usize, iced::task::Handle)>,
    result: Option<Result<(usize, usize, Option<PathBuf>), String>>,
    top: Vec<(String, usize)>,
    rows: Vec<crate::issues::IssueRow>,
    /// Only show this issue type.
    only: Option<String>,
    shown: Vec<usize>,
    list: VList,
}

#[derive(Debug, Clone)]
pub enum IssueMsg {
    PickScan,
    PickOut,
    Picked(bool, Option<PathBuf>),
    Start,
    Cancel,
    Event(ScanEvent),
    OpenReport(PathBuf),
    Only(Option<String>),
    Scrolled(scrollable::Viewport),
}

#[derive(Debug, Clone)]
pub enum ScanEvent {
    Found(usize),
    Scanned(usize),
    Done(Result<(usize, Vec<crate::issues::IssueRow>, Option<PathBuf>), String>),
}

fn scan_stream(root: PathBuf, out_dir: PathBuf) -> impl futures::Stream<Item = ScanEvent> {
    iced::stream::channel(16, async move |mut tx: Sender<ScanEvent>| {
        let root2 = root.clone();
        let charts = tokio::task::spawn_blocking(move || crate::issues::find_charts(&root2)).await.unwrap_or_default();
        let total = charts.len();
        let _ = tx.send(ScanEvent::Found(total)).await;
        let charts = Arc::new(charts);
        let mut rows = Vec::new();
        let mut done = 0;
        const CHUNK: usize = 64;
        while done < total {
            let (cs, start) = (charts.clone(), done);
            let part = tokio::task::spawn_blocking(move || {
                let end = (start + CHUNK).min(cs.len());
                std::thread::scope(|s| {
                    let hs: Vec<_> = cs[start..end]
                        .chunks(CHUNK.div_ceil(4))
                        .map(|c| s.spawn(move || c.iter().flat_map(crate::issues::scan_chart).collect::<Vec<_>>()))
                        .collect();
                    hs.into_iter().filter_map(|h| h.join().ok()).flatten().collect::<Vec<_>>()
                })
            })
            .await
            .unwrap_or_default();
            rows.extend(part);
            done = (done + CHUNK).min(total);
            let _ = tx.send(ScanEvent::Scanned(done)).await;
        }
        let result = if rows.is_empty() {
            Ok((total, rows, None))
        } else {
            let r2 = rows.clone();
            tokio::task::spawn_blocking(move || crate::issues::write_xlsx(&r2, &out_dir))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
                .map(|p| (total, rows, Some(p)))
        };
        let _ = tx.send(ScanEvent::Done(result)).await;
    })
}

impl IssueTool {
    fn update(&mut self, m: IssueMsg, cx: &Ctx) -> (Task<IssueMsg>, Out) {
        let s = cx.settings;
        match m {
            IssueMsg::PickScan | IssueMsg::PickOut => {
                let scan = matches!(m, IssueMsg::PickScan);
                return (
                    Task::perform(
                        async move {
                            rfd::AsyncFileDialog::new()
                                .set_title(if scan { "Folder to scan for chart issues" } else { "Where to save the spreadsheet" })
                                .pick_folder()
                                .await
                                .map(|h| h.path().to_path_buf())
                        },
                        move |p| IssueMsg::Picked(scan, p),
                    ),
                    Out::None,
                );
            }
            IssueMsg::Picked(_, None) => {}
            IssueMsg::Picked(scan, Some(p)) => {
                let (a, b) = if scan {
                    (Some(p), s.spreadsheet_output_path.clone())
                } else {
                    (s.issue_scan_path.clone(), Some(p))
                };
                return (Task::none(), Out::SetIssuePaths(a, b));
            }
            IssueMsg::Start => {
                let (Some(root), Some(out)) = (s.issue_scan_path.clone(), s.spreadsheet_output_path.clone()) else {
                    return (Task::none(), Out::None);
                };
                let (task, handle) = Task::run(scan_stream(root, out), IssueMsg::Event).abortable();
                self.progress = Some((0, 0, handle));
                self.result = None;
                return (task, Out::None);
            }
            IssueMsg::Cancel => {
                if let Some((_, _, h)) = self.progress.take() {
                    h.abort();
                }
            }
            IssueMsg::Event(ScanEvent::Found(n)) => {
                if let Some(p) = &mut self.progress {
                    p.1 = n;
                }
            }
            IssueMsg::Event(ScanEvent::Scanned(n)) => {
                if let Some(p) = &mut self.progress {
                    p.0 = n;
                }
            }
            IssueMsg::Event(ScanEvent::Done(r)) => {
                self.progress = None;
                self.top.clear();
                self.only = None;
                self.result = Some(r.map(|(charts, rows, path)| {
                    let mut kinds: HashMap<String, usize> = HashMap::new();
                    for r in &rows {
                        *kinds.entry(r.issue.clone()).or_default() += 1;
                    }
                    let mut top: Vec<_> = kinds.into_iter().collect();
                    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                    self.top = top;
                    let n = rows.len();
                    // Mandatory fixes first, then by chart.
                    let mut rows = rows;
                    rows.sort_by(|a, b| b.mandatory.cmp(&a.mandatory).then(a.path.cmp(&b.path)));
                    self.rows = rows;
                    (charts, n, path)
                }));
                self.filter_rows();
            }
            IssueMsg::OpenReport(p) => return (Task::none(), Out::Open(p)),
            IssueMsg::Only(k) => {
                self.only = if self.only == k { None } else { k };
                self.filter_rows();
            }
            IssueMsg::Scrolled(vp) => self.list.on_scroll(vp),
        }
        (Task::none(), Out::None)
    }

    fn filter_rows(&mut self) {
        self.list.set_id("issue-list");
        self.shown = (0..self.rows.len()).filter(|&i| self.only.as_ref().is_none_or(|k| &self.rows[i].issue == k)).collect();
        self.list.reset();
    }

    fn view<'a>(&'a self, cx: &Ctx<'a>) -> Element<'a, IssueMsg> {
        let s = cx.settings;
        let path_row = |label: &'static str, p: &Option<PathBuf>, msg: IssueMsg| {
            row![
                text(label).size(13).width(170),
                container(
                    text(p.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Not chosen".into()))
                        .size(13)
                        .wrapping(text::Wrapping::WordOrGlyph),
                )
                .width(Length::Fill),
                button(text("Choose…").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(msg),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
        };
        let ready = s.issue_scan_path.is_some() && s.spreadsheet_output_path.is_some() && self.progress.is_none();
        let mut col = column![
            heading("Chart issue scanner"),
            note("Checks every chart in a folder for the problems Chorus Encore checks for (missing files, bad metadata, broken notes, sustain gaps…) and saves a spreadsheet. Charts are only read, never changed."),
            path_row("Folder to scan", &s.issue_scan_path, IssueMsg::PickScan),
            path_row("Save spreadsheet in", &s.spreadsheet_output_path, IssueMsg::PickOut),
            button(text("Scan for issues").size(14)).padding([8, 16]).on_press_maybe(ready.then_some(IssueMsg::Start)),
        ]
        .spacing(12)
        .max_width(860);
        if let Some((done, total, _)) = &self.progress {
            let label = if *total == 0 { "Finding charts…".to_string() } else { format!("Scanned {done} of {total} charts…") };
            col = col.push(
                row![
                    column![text(label).size(13), progress_bar(0.0..=(*total).max(1) as f32, *done as f32).girth(6)]
                        .spacing(4)
                        .width(Length::Fill),
                    button(text("Stop").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(IssueMsg::Cancel),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            );
        }
        let mut table: Option<Element<'a, IssueMsg>> = None;
        match &self.result {
            Some(Ok((charts, n, path))) => {
                col = col.push(text(format!("{n} issues found in {charts} charts.")).size(14).style(text::success));
                if let Some(p) = path {
                    let folder = p.parent().map(|f| f.to_path_buf()).unwrap_or_default();
                    col = col.push(
                        row![
                            text(p.display().to_string()).size(12).width(Length::Fill).wrapping(text::Wrapping::WordOrGlyph),
                            button(text("Open spreadsheet").size(13)).padding([6, 12]).on_press(IssueMsg::OpenReport(p.clone())),
                            button(text("Show in folder").size(13)).padding([6, 12]).style(crate::style::secondary).on_press(IssueMsg::OpenReport(folder)),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    );
                }
                let mut chips = row![toggle_chip("All", self.only.is_none(), IssueMsg::Only(None))].spacing(6);
                for (k, n) in &self.top {
                    let active = self.only.as_deref() == Some(k.as_str());
                    chips = chips.push(
                        button(text(format!("{k} {n}")).size(12))
                            .padding([3, 10])
                            .style(move |t: &Theme, s| {
                                let mut b = if active { button::primary(t, s) } else { button::secondary(t, s) };
                                b.border = Border::default().rounded(if crate::style::refined() { 5 } else { 12 });
                                b
                            })
                            .on_press(IssueMsg::Only(Some(k.clone()))),
                    );
                }
                col = col.push(chips.wrap());
                let rows = &self.rows;
                table = Some(
                    column![
                        row![
                            text("Chart").size(12).font(crate::BOLD).width(Length::FillPortion(3)),
                            text("Issue").size(12).font(crate::BOLD).width(Length::FillPortion(2)),
                            text("Details").size(12).font(crate::BOLD).width(Length::FillPortion(6)),
                            text("Must fix").size(12).font(crate::BOLD).width(60),
                        ]
                        .spacing(10)
                        .padding([6, 20]),
                        rule::horizontal(1).style(crate::style::rule),
                        self.list.view(
                            self.shown.len(),
                            move |r| {
                                let row_ = &rows[self.shown[r]];
                                let chart = if row_.artist.is_empty() && row_.name.is_empty() {
                                    row_.path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
                                } else {
                                    format!("{} - {}", row_.artist, row_.name)
                                };
                                let cell = |s: String, p: u16| {
                                    container(text(s).size(12).wrapping(text::Wrapping::None)).width(Length::FillPortion(p)).clip(true)
                                };
                                container(
                                    row![
                                        cell(chart, 3),
                                        cell(row_.issue.clone(), 2),
                                        cell(row_.description.clone(), 6),
                                        text(if row_.mandatory { "yes" } else { "" }).size(12).width(60).style(text::danger),
                                    ]
                                    .spacing(10)
                                    .align_y(Alignment::Center),
                                )
                                .padding([0, 20])
                                .height(Length::Fill)
                                .align_y(Alignment::Center)
                                .style(move |t: &Theme| container::Style {
                                    background: (r % 2 == 1).then(|| crate::style::weakest(t).into()),
                                    ..Default::default()
                                })
                                .into()
                            },
                            IssueMsg::Scrolled,
                        ),
                    ]
                    .height(Length::Fill)
                    .into(),
                );
            }
            Some(Err(e)) => col = col.push(text(format!("Scan failed: {e}")).size(13).style(text::danger)),
            None => {}
        }
        let top = container(col).padding(20);
        match table {
            Some(t) => column![top, t].height(Length::Fill).into(),
            None => scrollable(top).height(Length::Fill).into(),
        }
    }
}

// ---------- Page ----------

#[derive(Default)]
pub struct Page {
    pub tab: Tab,
    issues: IssueTool,
    scenes: crate::scenes_ui::ScenesTool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Tab(Tab),
    Issues(IssueMsg),
    Scenes(crate::scenes_ui::Msg),
}

impl Page {
    pub fn update(&mut self, m: Msg, cx: &Ctx) -> (Task<Msg>, Out) {
        let (task, out) = match m {
            Msg::Tab(Tab::Issues) if !ISSUE_SCANNER_ENABLED => (Task::none(), Out::None),
            Msg::Tab(t) => {
                self.tab = t;
                (Task::none(), Out::None)
            }
            Msg::Issues(m) => {
                let (t, o) = self.issues.update(m, cx);
                (t.map(Msg::Issues), o)
            }
            Msg::Scenes(m) => (self.scenes.update(m).map(Msg::Scenes), Out::None),
        };
        (Task::batch([task, self.issues.list.take_scroll_task()]), out)
    }

    /// Runs on every page: the scene switcher's OBS connection.
    pub fn background(&self) -> Subscription<Msg> {
        self.scenes.background().map(Msg::Scenes)
    }

    pub fn subscription(&self) -> Subscription<Msg> {
        if self.tab == Tab::Scenes {
            return self.scenes.subscription().map(Msg::Scenes);
        }
        Subscription::none()
    }

    pub fn view<'a>(&'a self, cx: &Ctx<'a>) -> Element<'a, Msg> {
        let tab = |label: &'static str, t: Tab| crate::style::tab(label, self.tab == t, 14, Msg::Tab(t));
        let mut tab_row = row![tab("Scene Switcher", Tab::Scenes)];
        if ISSUE_SCANNER_ENABLED {
            tab_row = tab_row.push(tab("Issue scanner", Tab::Issues));
        }
        let tabs = container(tab_row.spacing(6).align_y(Alignment::Center)).padding([8, 12]);
        let body: Element<'a, Msg> = match self.tab {
            Tab::Issues => self.issues.view(cx).map(Msg::Issues),
            Tab::Scenes => self.scenes.view().map(Msg::Scenes),
        };
        column![tabs, rule::horizontal(1).style(crate::style::rule), body].height(Length::Fill).into()
    }
}
