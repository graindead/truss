//! Tools > Player Stats: any Clone Hero player's card and stats (your own to start with), and two
//! players side by side. The fetching, ratings and maths are ported from Truss Stats.

mod card_png;
mod fetch;
mod ratings;
mod stats;
mod versus;
mod view;

use futures::SinkExt;
use futures::channel::mpsc::Sender;
use iced::Task;
use iced::widget::image;
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Asking the server for the same player again sooner than this gives the kept copy.
const REFRESH_AFTER_SECS: i64 = 120;

/// A player, ready to show.
#[derive(Debug)]
pub struct Loaded {
    name: String,
    stats: Value,
    picture: Option<image::Handle>,
    /// The same picture's pixels (width, height, RGBA), for the saved card.
    picture_rgba: Option<(u32, u32, Vec<u8>)>,
    /// Their best run of each chart, to compare with another player's.
    runs: versus::Runs,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// Pages of scores read, and how many there are (0: not known yet).
    Progress(u32, u32),
    Done(Result<Arc<Loaded>, String>),
}

/// The player shown, or the one they're compared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Main,
    Other,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Query(Side, String),
    Find(Side),
    /// (search number, what was found)
    Found(Side, u64, Result<Vec<(u64, String)>, String>),
    Pick(Side, u64),
    Event(Side, u64, Event),
    Refresh(Side),
    RefreshBoth,
    /// Back to your own card.
    Me,
    /// Compare the player shown with you.
    WithMe,
    StopCompare,
    Open(String),
    /// Save the card of the player shown as a picture.
    SaveCard,
    CardSaved(Option<Result<std::path::PathBuf, String>>),
    ShowSaved,
}

enum State {
    Loading(u32, u32),
    Ready(Arc<Loaded>),
    Failed(String),
}

struct Slot {
    id: u64,
    state: State,
    /// Stops the loading when the slot goes.
    _load: Option<iced::task::Handle>,
}

impl Slot {
    fn ready(&self) -> Option<&Arc<Loaded>> {
        match &self.state {
            State::Ready(l) => Some(l),
            _ => None,
        }
    }
}

/// A search box and what it found.
#[derive(Default)]
struct Search {
    text: String,
    /// The newest search (older answers are dropped), and whether it's running.
    number: u64,
    running: bool,
    /// Players to choose from.
    picks: Vec<(u64, String)>,
    note: Option<String>,
}

#[derive(Default)]
pub struct PlayersTool {
    main: Option<Slot>,
    other: Option<Slot>,
    find: Search,
    find_other: Search,
    /// A card being saved, and where the last one went.
    saving: bool,
    saved: Option<Result<std::path::PathBuf, String>>,
}

async fn search(name: String) -> Result<Vec<(u64, String)>, String> {
    let users = fetch::fetcher()?.search(&name).await?;
    Ok(users.into_iter().map(|u| (u.account_id, ratings::strip_tags(&u.username))).collect())
}

/// The numbers and picture of a fetched player.
async fn prepare(p: fetch::Player) -> Result<Arc<Loaded>, String> {
    let picture = match (&p.profile.profile_picture_url, fetch::fetcher()) {
        (Some(url), Ok(f)) => f.avatar(url).await,
        _ => None,
    };
    tokio::task::spawn_blocking(move || {
        let picture_rgba = picture.and_then(|b| ::image::load_from_memory(&b).ok()).map(|i| {
            let i = i.thumbnail(256, 256).to_rgba8();
            (i.width(), i.height(), i.into_raw())
        });
        let picture = picture_rgba.as_ref().map(|(w, h, px)| image::Handle::from_rgba(*w, *h, px.clone()));
        let stats = stats::build(&p, &ratings::Ratings::bundled());
        let runs = versus::runs(&p.scores);
        let name = ratings::strip_tags(&p.profile.username);
        Arc::new(Loaded { name, stats, picture, picture_rgba, runs })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Loads a player: the kept copy, unless there's none or `refresh`.
fn load(id: u64, refresh: bool) -> impl futures::Stream<Item = Event> {
    iced::stream::channel(16, async move |mut tx: Sender<Event>| {
        let r = async {
            let f = fetch::fetcher()?;
            let fresh = |p: &fetch::Player| {
                chrono::DateTime::parse_from_rfc3339(&p.fetched_at)
                    .is_ok_and(|t| chrono::Utc::now().timestamp() - t.timestamp() < REFRESH_AFTER_SECS)
            };
            let player = match f.cached(id).await.filter(|p| !refresh || fresh(p)) {
                Some(p) => p,
                None => {
                    let progress = Mutex::new(tx.clone());
                    f.player(id, |page, pages| {
                        if let Ok(mut tx) = progress.lock() {
                            let _ = tx.try_send(Event::Progress(page, pages));
                        }
                    })
                    .await?
                }
            };
            prepare(player).await
        }
        .await;
        let _ = tx.send(Event::Done(r)).await;
    })
}

impl PlayersTool {
    /// The tab was opened: your own card, unless a player is showing already.
    pub fn opened(&mut self, me: Option<u64>) -> Task<Msg> {
        match me {
            Some(id) if self.main.is_none() => self.start(Side::Main, id, false),
            _ => Task::none(),
        }
    }

    fn slot(&mut self, side: Side) -> &mut Option<Slot> {
        match side {
            Side::Main => &mut self.main,
            Side::Other => &mut self.other,
        }
    }

    fn search(&mut self, side: Side) -> &mut Search {
        match side {
            Side::Main => &mut self.find,
            Side::Other => &mut self.find_other,
        }
    }

    /// Loads a player into a side.
    fn start(&mut self, side: Side, id: u64, refresh: bool) -> Task<Msg> {
        if side == Side::Main {
            self.saved = None;
        }
        let s = self.search(side);
        s.picks.clear();
        s.note = None;
        s.running = false;
        s.number += 1;
        let (task, handle) = Task::run(load(id, refresh), move |e| Msg::Event(side, id, e)).abortable();
        *self.slot(side) = Some(Slot { id, state: State::Loading(0, 0), _load: Some(handle.abort_on_drop()) });
        task
    }

    /// A player picked for a side: nobody is compared with themselves.
    fn pick(&mut self, side: Side, id: u64) -> Task<Msg> {
        if side == Side::Other && self.main.as_ref().is_some_and(|m| m.id == id) {
            let s = self.search(side);
            s.picks.clear();
            s.note = Some("That's the player already shown.".into());
            return Task::none();
        }
        if side == Side::Main {
            self.other = None;
        }
        self.search(side).text.clear();
        self.start(side, id, false)
    }

    /// Returns a web page to open, if one was asked for.
    pub fn update(&mut self, m: Msg, me: Option<u64>) -> (Task<Msg>, Option<String>) {
        let task = match m {
            Msg::Query(side, t) => {
                let s = self.search(side);
                s.text = t.chars().take(64).collect();
                s.note = None;
                Task::none()
            }
            Msg::Find(side) => {
                let s = self.search(side);
                let name = s.text.trim().to_string();
                if name.is_empty() {
                    return (Task::none(), None);
                }
                s.number += 1;
                s.running = true;
                s.picks.clear();
                s.note = None;
                let n = s.number;
                Task::perform(search(name), move |r| Msg::Found(side, n, r))
            }
            Msg::Found(side, n, r) => {
                let s = self.search(side);
                if n != s.number {
                    return (Task::none(), None);
                }
                s.running = false;
                match r {
                    Ok(users) => {
                        let want = s.text.trim().to_lowercase();
                        let exact: Vec<u64> = users.iter().filter(|u| u.1.to_lowercase() == want).map(|u| u.0).collect();
                        if let [id] = exact[..] {
                            return (self.pick(side, id), None);
                        }
                        if users.is_empty() {
                            s.note = Some("No player with that name on Clone Hero's leaderboards.".into());
                        }
                        s.picks = users;
                    }
                    Err(e) => s.note = Some(e),
                }
                Task::none()
            }
            Msg::Pick(side, id) => self.pick(side, id),
            Msg::Event(side, id, e) => {
                if let Some(s) = self.slot(side).as_mut().filter(|s| s.id == id) {
                    s.state = match e {
                        Event::Progress(a, b) => State::Loading(a, b),
                        Event::Done(Ok(l)) => State::Ready(l),
                        Event::Done(Err(e)) => State::Failed(e),
                    };
                }
                Task::none()
            }
            Msg::Refresh(side) => match self.slot(side).as_ref().map(|s| s.id) {
                Some(id) => self.start(side, id, true),
                None => Task::none(),
            },
            Msg::RefreshBoth => {
                let ids: Vec<_> = [Side::Main, Side::Other].into_iter().filter_map(|s| Some((s, self.slot(s).as_ref()?.id))).collect();
                Task::batch(ids.into_iter().map(|(s, id)| self.start(s, id, true)).collect::<Vec<_>>())
            }
            Msg::Me => match me {
                Some(id) => self.pick(Side::Main, id),
                None => Task::none(),
            },
            // You on the left, them on the right.
            Msg::WithMe => match me {
                Some(id) => {
                    self.other = self.main.take();
                    // Their loading (if it still is) reports to the old side: start it over.
                    let them = match &self.other {
                        Some(s) if s.ready().is_none() => Some(s.id),
                        _ => None,
                    };
                    let again = them.map_or(Task::none(), |id| self.start(Side::Other, id, false));
                    Task::batch([again, self.start(Side::Main, id, false)])
                }
                None => Task::none(),
            },
            Msg::StopCompare => {
                self.other = None;
                self.find_other = Search { number: self.find_other.number + 1, ..Default::default() };
                Task::none()
            }
            Msg::Open(url) => return (Task::none(), Some(url)),
            Msg::SaveCard => {
                let Some(l) = self.main.as_ref().and_then(Slot::ready).cloned() else { return (Task::none(), None) };
                self.saving = true;
                self.saved = None;
                let name: String = l.name.chars().map(|c| if c.is_alphanumeric() || " ._-".contains(c) { c } else { '_' }).collect();
                let name = format!("{} - player card.png", if name.trim().is_empty() { "player" } else { name.trim() });
                Task::perform(
                    async move {
                        let file = rfd::AsyncFileDialog::new().set_title("Save player card").set_file_name(name).add_filter("PNG image", &["png"]).save_file().await?;
                        let path = file.path().to_path_buf();
                        let p = path.clone();
                        Some(tokio::task::spawn_blocking(move || card_png::save(&l, &p)).await.map_err(|e| e.to_string()).and_then(|r| r).map(|_| path))
                    },
                    Msg::CardSaved,
                )
            }
            Msg::CardSaved(r) => {
                self.saving = false;
                self.saved = r;
                Task::none()
            }
            Msg::ShowSaved => match &self.saved {
                Some(Ok(p)) => return (Task::none(), p.parent().map(|d| d.display().to_string())),
                _ => Task::none(),
            },
        };
        (task, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(t: &mut PlayersTool, side: Side, name: &str, users: &[(u64, &str)]) {
        let _ = t.update(Msg::Query(side, name.into()), None);
        let _ = t.update(Msg::Find(side), None);
        let n = t.search(side).number;
        let users = users.iter().map(|(i, u)| (*i, u.to_string())).collect();
        let _ = t.update(Msg::Found(side, n, Ok(users)), None);
    }

    #[test]
    fn finding_players() {
        let mut t = PlayersTool::default();
        // One exact name goes straight to it, whatever its capitals.
        found(&mut t, Side::Main, "someone", &[(1, "SomeOne"), (2, "Someone Else")]);
        assert_eq!(t.main.as_ref().map(|s| s.id), Some(1));
        // Otherwise there's a choice.
        found(&mut t, Side::Other, "some", &[(1, "SomeOne"), (2, "Someone Else")]);
        assert!(t.other.is_none() && t.find_other.picks.len() == 2);
        // Never compared with themselves.
        let _ = t.update(Msg::Pick(Side::Other, 1), None);
        assert!(t.other.is_none() && t.find_other.note.is_some());
        let _ = t.update(Msg::Pick(Side::Other, 2), None);
        assert_eq!(t.other.as_ref().map(|s| s.id), Some(2));
        // An older search's answer is dropped.
        let _ = t.update(Msg::Found(Side::Main, 0, Ok(vec![(9, "x".into())])), None);
        assert!(t.find.picks.is_empty());
    }

    #[test]
    fn comparing_with_me() {
        let mut t = PlayersTool::default();
        let _ = t.update(Msg::Pick(Side::Main, 5), Some(1));
        let _ = t.update(Msg::WithMe, Some(1));
        assert_eq!((t.main.as_ref().map(|s| s.id), t.other.as_ref().map(|s| s.id)), (Some(1), Some(5)));
        let _ = t.update(Msg::StopCompare, Some(1));
        assert!(t.other.is_none());
    }
}
