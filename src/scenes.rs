//! CH Scene Switcher, built into Truss: switches OBS scenes when Clone Hero starts or ends a
//! song (it watches `currentsong.txt`), shows the Ouvert song request queue, and runs "Next
//! song" through Ouvert or a Twitch chat command.
//!
//! Settings are shared with the standalone CH Scene Switcher app:
//! `%APPDATA%\CH Scene Switcher\settings.ini`, with the API keys and Twitch tokens encrypted by
//! Windows (DPAPI) so only this Windows user can read them.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;

// ---------- Settings ----------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub host: String,
    pub port: String,
    pub password: String,
    pub song_scene: String,
    pub menu_scene: String,
    pub mp_song_scene: String,
    pub mp_menu_scene: String,
    /// 0 singleplayer, 1 multiplayer.
    pub mode: u8,
    /// Seconds, as typed.
    pub menu_delay: String,
    pub song_file: String,
    pub ouvert_key: String,
    pub ouvert_private_key: String,
    pub twitch_access_token: String,
    pub twitch_refresh_token: String,
    pub twitch_user_id: String,
    pub twitch_login: String,
    pub next_chat_command: String,
    /// Truss connects to OBS when it starts (set after you click Connect in Truss).
    pub truss_connect: bool,
}

const SECRETS: [&str; 4] = ["ouvert_key", "ouvert_private_key", "twitch_access_token", "twitch_refresh_token"];

impl Config {
    fn fields(&mut self) -> [(&'static str, &mut String); 16] {
        [
            ("host", &mut self.host),
            ("port", &mut self.port),
            ("password", &mut self.password),
            ("song_scene", &mut self.song_scene),
            ("menu_scene", &mut self.menu_scene),
            ("menu_delay", &mut self.menu_delay),
            ("song_file", &mut self.song_file),
            ("mp_song_scene", &mut self.mp_song_scene),
            ("mp_menu_scene", &mut self.mp_menu_scene),
            ("ouvert_key", &mut self.ouvert_key),
            ("ouvert_private_key", &mut self.ouvert_private_key),
            ("twitch_access_token", &mut self.twitch_access_token),
            ("twitch_refresh_token", &mut self.twitch_refresh_token),
            ("twitch_user_id", &mut self.twitch_user_id),
            ("twitch_login", &mut self.twitch_login),
            ("next_chat_command", &mut self.next_chat_command),
        ]
    }

    /// The song and menu scenes of the active mode.
    pub fn scenes(&self, mode: u8) -> (&str, &str) {
        if mode == 0 { (&self.song_scene, &self.menu_scene) } else { (&self.mp_song_scene, &self.mp_menu_scene) }
    }

    pub fn delay_ms(&self) -> Option<u64> {
        let s = self.menu_delay.trim();
        if s.is_empty() {
            return Some(0);
        }
        let v: f64 = s.parse().ok()?;
        (v.is_finite() && v >= 0.0 && v < 86_400.0).then(|| (v * 1000.0) as u64)
    }

    pub fn port_ok(&self) -> bool {
        matches!(self.port.trim().parse::<u32>(), Ok(1..=65535))
    }
}

fn dir() -> PathBuf {
    let base = std::env::var_os("TRUSS_TEST_CHSS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("CH Scene Switcher"));
    base
}

fn ini_path() -> PathBuf {
    dir().join("settings.ini")
}

/// Values as Windows' profile functions read them: surrounding quotes removed.
fn ini_read() -> Vec<(String, String)> {
    let Ok(bytes) = std::fs::read(ini_path()) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    let mut out = Vec::new();
    let mut in_settings = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_settings = l.eq_ignore_ascii_case("[settings]");
            continue;
        }
        if !in_settings {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            let v = v.trim();
            let v = if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') { &v[1..v.len() - 1] } else { v };
            out.push((k.trim().to_string(), v.to_string()));
        }
    }
    out
}

/// Loads the settings, with the standalone app's defaults, and finds the song file.
pub fn load() -> Config {
    let mut c = Config {
        host: "localhost".into(),
        port: "4455".into(),
        menu_delay: "0".into(),
        next_chat_command: "!playnext".into(),
        ..Default::default()
    };
    let saved = ini_read();
    let mut plain_secret = false;
    for (k, v) in &saved {
        match k.as_str() {
            "mode" => c.mode = (v == "1") as u8,
            "truss_connect" => c.truss_connect = v == "1",
            _ => {}
        }
    }
    for (key, field) in c.fields() {
        let Some((_, v)) = saved.iter().find(|(k, _)| k == key) else { continue };
        if SECRETS.contains(&key) {
            match v.strip_prefix("dpapi:") {
                Some(enc) => *field = dpapi::decrypt(enc).unwrap_or_default(),
                None => {
                    plain_secret |= !v.is_empty();
                    *field = v.clone();
                }
            }
        } else {
            *field = v.clone();
        }
    }
    if c.song_file.is_empty() || !std::path::Path::new(&c.song_file).is_file() {
        let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default());
        let tries = [
            home.join(r"Documents\Clone Hero\PlayerData\currentsong.txt"),
            home.join(r"Documents\Clone Hero\currentsong.txt"),
            home.join(r"OneDrive\Documents\Clone Hero\PlayerData\currentsong.txt"),
        ];
        if let Some(p) = tries.iter().find(|p| p.is_file()) {
            c.song_file = p.to_string_lossy().into_owned();
        } else if c.song_file.is_empty() {
            c.song_file = tries[0].to_string_lossy().into_owned();
        }
    }
    if plain_secret {
        let _ = save(&c); // an older version saved a key in plain text: store it encrypted
    }
    c
}

/// Saves the settings, keeping any other lines in the file (such as the standalone app's
/// window position).
pub fn save(c: &Config) -> Result<(), String> {
    let mut c = c.clone();
    let mut vals: Vec<(String, String)> = c
        .fields()
        .into_iter()
        .map(|(k, v)| {
            let v = if SECRETS.contains(&k) && !v.is_empty() {
                // Never write a secret in plain text: if encryption fails, write nothing.
                dpapi::encrypt(v).map(|e| format!("dpapi:{e}")).unwrap_or_default()
            } else {
                v.clone()
            };
            (k.to_string(), v)
        })
        .collect();
    vals.push(("mode".into(), c.mode.to_string()));
    vals.push(("truss_connect".into(), if c.truss_connect { "1" } else { "0" }.into()));

    let old = std::fs::read(ini_path()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut in_settings = false;
    let mut had_section = false;
    for line in old.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            if in_settings {
                lines.extend(vals.drain(..).map(|(k, v)| format!("{k}=\"{v}\"")));
            }
            in_settings = l.eq_ignore_ascii_case("[settings]");
            had_section |= in_settings;
            lines.push(line.to_string());
            continue;
        }
        if in_settings && let Some((k, _)) = l.split_once('=') {
            let k = k.trim();
            if let Some(i) = vals.iter().position(|(n, _)| n == k) {
                let (k, v) = vals.remove(i);
                lines.push(format!("{k}=\"{v}\""));
                continue;
            }
        }
        lines.push(line.to_string());
    }
    if !had_section {
        lines.push("[settings]".into());
    }
    lines.extend(vals.drain(..).map(|(k, v)| format!("{k}=\"{v}\"")));
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let tmp = ini_path().with_extension("ini.tmp");
    std::fs::write(&tmp, lines.join("\r\n") + "\r\n").map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, ini_path()).map_err(|e| e.to_string())
}

/// Windows DPAPI: encrypts for this Windows user only.
mod dpapi {
    use super::B64;
    use base64::Engine;

    #[repr(C)]
    struct Blob {
        cb: u32,
        pb: *mut u8,
    }

    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(i: *const Blob, d: *const u16, e: *const Blob, r: *const u8, p: *const u8, f: u32, o: *mut Blob) -> i32;
        fn CryptUnprotectData(i: *const Blob, d: *mut *mut u16, e: *const Blob, r: *const u8, p: *const u8, f: u32, o: *mut Blob) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(p: *mut u8) -> *mut u8;
    }

    const UI_FORBIDDEN: u32 = 1;

    pub fn encrypt(s: &str) -> Option<String> {
        let input = Blob { cb: s.len() as u32, pb: s.as_ptr() as *mut u8 };
        let mut out = Blob { cb: 0, pb: std::ptr::null_mut() };
        // SAFETY: valid blobs; the output is freed with LocalFree as documented.
        unsafe {
            if CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), UI_FORBIDDEN, &mut out) == 0 {
                return None;
            }
            let enc = B64.encode(std::slice::from_raw_parts(out.pb, out.cb as usize));
            LocalFree(out.pb);
            Some(enc)
        }
    }

    pub fn decrypt(b64: &str) -> Option<String> {
        let mut bin = B64.decode(b64.trim()).ok()?;
        let input = Blob { cb: bin.len() as u32, pb: bin.as_mut_ptr() };
        let mut out = Blob { cb: 0, pb: std::ptr::null_mut() };
        // SAFETY: as above.
        unsafe {
            if CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), UI_FORBIDDEN, &mut out) == 0 {
                return None;
            }
            let s = String::from_utf8_lossy(std::slice::from_raw_parts(out.pb, out.cb as usize)).into_owned();
            LocalFree(out.pb);
            Some(s)
        }
    }
}

// ---------- OBS worker ----------

/// What the page tells the worker.
#[derive(Debug, Clone)]
pub enum Cmd {
    /// (Re)connect with these settings.
    Connect(Config),
    Disconnect,
    /// Settings changed (scenes, file, delay): used from the next check on.
    Update(Config),
    /// The active mode changed: move OBS to the new mode's scene.
    ModeChanged(Config),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conn {
    #[default]
    Off,
    Connecting,
    Connected,
    Failed,
}

/// What the worker tells the page.
#[derive(Debug, Clone)]
pub enum Event {
    Ready(mpsc::UnboundedSender<Cmd>),
    Conn(Conn, String),
    /// The latest song / switch line.
    Status(String),
    Scenes(Vec<String>),
}

const POLL: Duration = Duration::from_millis(250);
const RETRY: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(3);

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Obs {
    ws: Ws,
    next_id: u64,
}

impl Obs {
    async fn recv(&mut self) -> Result<Value, String> {
        loop {
            let m = tokio::time::timeout(TIMEOUT, self.ws.next())
                .await
                .map_err(|_| "OBS stopped answering".to_string())?
                .ok_or("OBS closed the connection")?
                .map_err(|e| e.to_string())?;
            match m {
                WsMsg::Text(t) => return serde_json::from_str(&t).map_err(|e| e.to_string()),
                WsMsg::Close(f) => {
                    return Err(match f {
                        Some(f) if !f.reason.is_empty() => format!("OBS closed the connection: {}", f.reason),
                        _ => "OBS closed the connection".into(),
                    });
                }
                _ => {}
            }
        }
    }

    async fn send(&mut self, v: Value) -> Result<(), String> {
        tokio::time::timeout(TIMEOUT, self.ws.send(WsMsg::text(v.to_string())))
            .await
            .map_err(|_| "OBS stopped answering".to_string())?
            .map_err(|e| e.to_string())
    }

    /// Connects and logs in (OBS WebSocket 5).
    async fn connect(c: &Config) -> Result<Obs, String> {
        let url = format!("ws://{}:{}", c.host.trim(), c.port.trim());
        let (ws, _) = tokio::time::timeout(TIMEOUT, tokio_tungstenite::connect_async(url.as_str()))
            .await
            .map_err(|_| format!("no response from {}:{}", c.host.trim(), c.port.trim()))?
            .map_err(|e| format!("no response from {}:{} ({e})", c.host.trim(), c.port.trim()))?;
        let mut obs = Obs { ws, next_id: 1 };
        let hello = obs.recv().await?;
        let mut identify = json!({"rpcVersion": 1, "eventSubscriptions": 0});
        let auth = &hello["d"]["authentication"];
        if let (Some(ch), Some(salt)) = (auth["challenge"].as_str(), auth["salt"].as_str()) {
            // secret = base64(sha256(password + salt)), auth = base64(sha256(secret + challenge))
            let secret = B64.encode(Sha256::digest(format!("{}{salt}", c.password).as_bytes()));
            identify["authentication"] = B64.encode(Sha256::digest(format!("{secret}{ch}").as_bytes())).into();
        }
        obs.send(json!({"op": 1, "d": identify})).await?;
        let r = obs.recv().await?;
        if r["op"] != 2 {
            return Err("OBS didn't accept the login (check the password)".into());
        }
        Ok(obs)
    }

    /// Sends a request -> its response data, or why it failed.
    async fn request(&mut self, ty: &str, data: Value) -> Result<Value, String> {
        let id = self.next_id.to_string();
        self.next_id += 1;
        self.send(json!({"op": 6, "d": {"requestType": ty, "requestId": id, "requestData": data}})).await?;
        loop {
            let r = self.recv().await?;
            if r["op"] == 7 && r["d"]["requestId"] == id.as_str() {
                let st = &r["d"]["requestStatus"];
                if st["result"] == true {
                    return Ok(r["d"]["responseData"].clone());
                }
                return Err(st["comment"].as_str().unwrap_or("OBS refused that").to_string());
            }
        }
    }

    /// Scene names in the order of OBS's Scenes dock (OBS lists them bottom to top).
    async fn scenes(&mut self) -> Result<Vec<String>, String> {
        let r = self.request("GetSceneList", json!({})).await?;
        let mut v: Vec<String> = r["scenes"]
            .as_array()
            .map(|a| a.iter().filter_map(|s| s["sceneName"].as_str().map(String::from)).collect())
            .unwrap_or_default();
        v.reverse();
        Ok(v)
    }

    async fn switch(&mut self, scene: &str) -> Result<(), String> {
        self.request("SetCurrentProgramScene", json!({"sceneName": scene})).await.map(|_| ())
    }
}

/// The song file, trimmed, and when it was last written.
fn read_song(path: &str) -> (String, Option<SystemTime>) {
    let text = std::fs::read(path).map(|b| String::from_utf8_lossy(&b).trim().to_string()).unwrap_or_default();
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    (text, mtime)
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

/// The OBS worker, run as a subscription so it keeps going on every page.
pub fn worker() -> impl futures::Stream<Item = Event> {
    iced::stream::channel(32, async move |mut out: futures::channel::mpsc::Sender<Event>| {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _ = out.send(Event::Ready(tx)).await;
        let mut cfg: Option<Config> = None; // Some while connecting is wanted
        let mut obs: Option<Obs> = None;
        let mut retry_at = Instant::now();
        let (mut last, mut last_mtime) = (String::new(), None);
        let (mut playing, mut pending, mut due) = (false, false, Instant::now());
        macro_rules! emit {
            ($e:expr) => {
                let _ = out.send($e).await;
            };
        }
        loop {
            let cmd = tokio::time::timeout(POLL, rx.recv()).await;
            let mut mode_changed = false;
            match cmd {
                Ok(None) => return,
                Ok(Some(Cmd::Connect(c))) => {
                    cfg = Some(c);
                    obs = None;
                    retry_at = Instant::now();
                }
                Ok(Some(Cmd::Disconnect)) => {
                    cfg = None;
                    if let Some(mut o) = obs.take() {
                        let _ = o.ws.close(None).await;
                    }
                    emit!(Event::Conn(Conn::Off, "Not connected".into()));
                    continue;
                }
                Ok(Some(Cmd::Update(c))) => {
                    if cfg.is_some() {
                        cfg = Some(c);
                    }
                }
                Ok(Some(Cmd::ModeChanged(c))) => {
                    if cfg.is_some() {
                        cfg = Some(c);
                        mode_changed = true;
                    }
                }
                Err(_) => {}
            }
            let Some(c) = cfg.clone() else { continue };
            let (song, menu) = c.scenes(c.mode);
            let (song, menu) = (song.to_string(), menu.to_string());

            if obs.is_none() {
                if Instant::now() < retry_at {
                    continue;
                }
                let at = format!("{}:{}", c.host.trim(), c.port.trim());
                emit!(Event::Conn(Conn::Connecting, format!("Connecting to {at}…")));
                let connected = match Obs::connect(&c).await {
                    Ok(mut o) => match o.scenes().await {
                        Ok(s) => {
                            emit!(Event::Scenes(s));
                            Ok(o)
                        }
                        Err(_) => Err("OBS didn't send its scene list".to_string()),
                    },
                    Err(e) => Err(e),
                };
                match connected {
                    Ok(o) => {
                        obs = Some(o);
                        emit!(Event::Conn(Conn::Connected, format!("Connected to OBS at {at}")));
                        // Start from the file as it is now, without switching.
                        (last, last_mtime) = read_song(&c.song_file);
                        pending = false;
                        playing = !last.is_empty();
                        let msg = if song.is_empty() {
                            "Pick the scene to show while a song is playing.".to_string()
                        } else if last.is_empty() {
                            "Waiting for a song…".to_string()
                        } else {
                            format!("Song: {}", first_line(&last))
                        };
                        emit!(Event::Status(msg));
                    }
                    Err(e) => {
                        emit!(Event::Conn(Conn::Failed, format!("Can't connect: {e} — retrying every 5 s")));
                        retry_at = Instant::now() + RETRY;
                    }
                }
                continue;
            }

            // What to switch to this time, if anything: (menu?, message).
            let mut switch: Option<(bool, String)> = None;
            if mode_changed && (playing || !pending) {
                let name = if c.mode == 0 { "Singleplayer" } else { "Multiplayer" };
                switch = Some((!playing, name.to_string()));
            }
            let (text, mtime) = read_song(&c.song_file);
            if mtime != last_mtime || text != last {
                last_mtime = mtime;
                last = text;
                if !last.is_empty() {
                    let msg = format!("Song: {}", first_line(&last));
                    pending = false;
                    if !playing {
                        playing = true;
                        switch = Some((false, msg));
                    } else {
                        emit!(Event::Status(msg)); // already on the song scene
                    }
                } else if playing {
                    playing = false;
                    pending = true;
                    due = Instant::now() + Duration::from_millis(c.delay_ms().unwrap_or(0));
                    emit!(Event::Status("Song ended".into()));
                }
            }
            if switch.is_none() && pending && Instant::now() >= due {
                pending = false;
                switch = Some((true, "Song ended".into()));
            }
            let Some((to_menu, msg)) = switch else { continue };
            let scene = if to_menu { &menu } else { &song };
            if scene.is_empty() {
                emit!(Event::Status(msg));
                continue;
            }
            let mut ok = obs.as_mut().unwrap().switch(scene).await.is_ok();
            if !ok {
                // Reconnect once, then try again.
                emit!(Event::Conn(Conn::Connecting, "Connection to OBS lost — reconnecting…".into()));
                match Obs::connect(&c).await {
                    Ok(mut o) => {
                        ok = o.switch(scene).await.is_ok();
                        obs = Some(o);
                        emit!(Event::Conn(Conn::Connected, format!("Connected to OBS at {}:{}", c.host.trim(), c.port.trim())));
                    }
                    Err(_) => obs = None,
                }
            }
            if ok {
                emit!(Event::Status(format!("{msg}  →  {scene}")));
            } else {
                emit!(Event::Status(format!("{msg}  (couldn't switch to '{scene}')")));
                obs = None;
                retry_at = Instant::now();
            }
        }
    })
}

// ---------- Ouvert ----------

#[cfg(not(test))]
const OUVERT: &str = "https://ouvert.dev";
#[cfg(test)]
const OUVERT: &str = "http://127.0.0.1:4467";

static HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!("truss/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(8))
        .build()
        .expect("http client")
});

/// The request queue as display lines ("Artist: Song   -   requester   (2:19)").
pub async fn ouvert_queue(key: String) -> Result<Vec<String>, String> {
    if key.trim().is_empty() {
        return Err("Enter your Ouvert public API key in Settings.".into());
    }
    let r = HTTP
        .get(format!("{OUVERT}/API/SongRequests"))
        .header("X-API-KEY", key.trim())
        .send()
        .await
        .map_err(|e| format!("Can't reach Ouvert ({e}). Retrying…"))?;
    let v: Value = r.json().await.map_err(|_| "Ouvert sent something unexpected.".to_string())?;
    if v["status"] != true {
        return Err("Ouvert didn't accept that key, or sent something unexpected.".into());
    }
    Ok(v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .take(100)
                .map(|r| {
                    let mut s = r["request"].as_str().unwrap_or("?").to_string();
                    if let Some(n) = r["requester_name"].as_str().filter(|n| !n.is_empty()) {
                        s += &format!("   -   {n}");
                    }
                    let secs = r["length"].as_u64().or_else(|| r["length"].as_str().and_then(|x| x.parse().ok()));
                    if let Some(t) = secs {
                        s += &format!("   ({}:{:02})", t / 60, t % 60);
                    }
                    s
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Next song through Ouvert, with the private key. Never retried: it may have worked even if
/// the answer got lost.
async fn ouvert_next(key: &str) -> String {
    if key.trim().is_empty() {
        return "Enter your private API key in Settings (or sign in to Twitch).".into();
    }
    let r = match HTTP.get(format!("{OUVERT}/API/SongRequests/Next")).header("X-API-KEY", key.trim()).send().await {
        Ok(r) => r,
        Err(e) => return format!("Can't reach Ouvert ({e})"),
    };
    let v: Value = r.json().await.unwrap_or_default();
    match v["message"].as_str().filter(|m| !m.is_empty()) {
        Some(m) => m.to_string(),
        None if v["status"] == true => "Moved to the next song.".into(),
        None => "Ouvert refused that.".into(),
    }
}

// ---------- Twitch ----------

#[cfg(not(test))]
const TW_ID: &str = "https://id.twitch.tv";
#[cfg(not(test))]
const TW_API: &str = "https://api.twitch.tv";
#[cfg(test)]
const TW_ID: &str = "http://127.0.0.1:4468";
#[cfg(test)]
const TW_API: &str = "http://127.0.0.1:4468";
/// CH Scene Switcher's public Twitch Client ID (not a secret).
const CLIENT_ID: &str = "0f04ohpjdjce8fktpx6wjnsx05g5sh";
const SCOPE: &str = "user:write:chat";

#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: Duration,
    pub expires: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub id: String,
    pub login: String,
}

pub async fn twitch_start() -> Result<DeviceCode, String> {
    let r = HTTP
        .post(format!("{TW_ID}/oauth2/device"))
        .form(&[("client_id", CLIENT_ID), ("scopes", SCOPE)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("Twitch returned {}", r.status()));
    }
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    let s = |k: &str| v[k].as_str().map(String::from).ok_or("Twitch sent something unexpected.".to_string());
    Ok(DeviceCode {
        device_code: s("device_code")?,
        user_code: s("user_code")?,
        verification_uri: s("verification_uri")?,
        // never faster than Twitch's documented 5 seconds
        interval: Duration::from_secs(v["interval"].as_u64().unwrap_or(5).max(5)),
        expires: Instant::now() + Duration::from_secs(v["expires_in"].as_u64().unwrap_or(1800)),
    })
}

/// Ok(None) while the code is waiting to be approved.
pub async fn twitch_poll(device_code: String) -> Result<Option<Tokens>, String> {
    let r = HTTP
        .post(format!("{TW_ID}/oauth2/token"))
        .form(&[
            ("client_id", CLIENT_ID),
            ("scopes", SCOPE),
            ("device_code", device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await;
    let Ok(r) = r else { return Ok(None) }; // network trouble: try again next time
    let ok = r.status().is_success();
    let v: Value = r.json().await.unwrap_or_default();
    if !ok {
        return if v.to_string().contains("authorization_pending") { Ok(None) } else { Err("Twitch sign-in failed. Try again.".into()) };
    }
    let mut t = Tokens {
        access: v["access_token"].as_str().ok_or("Twitch sign-in failed. Try again.")?.into(),
        refresh: v["refresh_token"].as_str().unwrap_or("").into(),
        ..Default::default()
    };
    let (id, login) = twitch_user(&t.access).await.ok_or("Twitch sign-in failed. Try again.")?;
    t.id = id;
    t.login = login;
    Ok(Some(t))
}

async fn twitch_user(access: &str) -> Option<(String, String)> {
    let r = HTTP.get(format!("{TW_API}/helix/users")).bearer_auth(access).header("Client-Id", CLIENT_ID).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: Value = r.json().await.ok()?;
    let u = &v["data"][0];
    Some((u["id"].as_str()?.into(), u["login"].as_str()?.into()))
}

/// Tells Twitch to cancel the token (sign out).
pub async fn twitch_revoke(access: String) {
    if !access.is_empty() {
        let _ = HTTP.post(format!("{TW_ID}/oauth2/revoke")).form(&[("client_id", CLIENT_ID), ("token", access.as_str())]).send().await;
    }
}

async fn twitch_refresh(refresh: &str) -> Option<(String, String)> {
    if refresh.is_empty() {
        return None;
    }
    let r = HTTP
        .post(format!("{TW_ID}/oauth2/token"))
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", CLIENT_ID)])
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    let v: Value = r.json().await.ok()?;
    Some((v["access_token"].as_str()?.into(), v["refresh_token"].as_str().unwrap_or(refresh).into()))
}

async fn post_chat(t: &Tokens, message: &str) -> Result<(u16, Value), String> {
    let r = HTTP
        .post(format!("{TW_API}/helix/chat/messages"))
        .bearer_auth(&t.access)
        .header("Client-Id", CLIENT_ID)
        .json(&json!({"broadcaster_id": t.id, "sender_id": t.id, "message": message}))
        .send()
        .await
        .map_err(|e| format!("Couldn't send to Twitch ({e})"))?;
    let status = r.status().as_u16();
    Ok((status, r.json().await.unwrap_or_default()))
}

/// The result of Next song: the message to show, and new Twitch tokens if they were refreshed
/// (Some(empty) = the sign-in expired).
#[derive(Debug, Clone)]
pub struct NextResult {
    pub message: String,
    pub tokens: Option<Tokens>,
    /// The bot was asked in chat, so the queue will change in a moment.
    pub via_chat: bool,
}

/// Next song: signed in to Twitch, sends the bot's chat command (so the bot announces the
/// song); otherwise asks Ouvert with the private key.
pub async fn next_song(c: Config) -> NextResult {
    if c.twitch_access_token.is_empty() {
        return NextResult { message: ouvert_next(&c.ouvert_private_key).await, tokens: None, via_chat: false };
    }
    let cmd = c.next_chat_command.trim().to_string();
    if cmd.is_empty() {
        return NextResult { message: "Set a chat command in Settings.".into(), tokens: None, via_chat: false };
    }
    let mut t = Tokens {
        access: c.twitch_access_token.clone(),
        refresh: c.twitch_refresh_token.clone(),
        id: c.twitch_user_id.clone(),
        login: c.twitch_login.clone(),
    };
    let mut new_tokens = None;
    let mut res = post_chat(&t, &cmd).await;
    if matches!(res, Ok((401, _))) {
        // access tokens last about 4 hours
        match twitch_refresh(&t.refresh).await {
            Some((a, r)) => {
                t.access = a;
                t.refresh = r;
                new_tokens = Some(t.clone());
                res = post_chat(&t, &cmd).await;
            }
            None => {
                return NextResult {
                    message: "Twitch sign-in expired. Sign in again in Settings.".into(),
                    tokens: Some(Tokens::default()),
                    via_chat: false,
                };
            }
        }
    }
    let (message, via_chat) = match res {
        Err(e) => (e, false),
        Ok((200, v)) if v["data"][0]["is_sent"] == true => (format!("Sent {cmd} to chat"), true),
        Ok((status, v)) => {
            let why = v["data"][0]["drop_reason"]["message"].as_str().or(v["message"].as_str()).map(String::from);
            (why.unwrap_or_else(|| format!("Couldn't send to Twitch (error {status})")), false)
        }
    };
    NextResult { message, tokens: new_tokens, via_chat }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_parsing() {
        let mut c = Config { menu_delay: "1.5".into(), ..Default::default() };
        assert_eq!(c.delay_ms(), Some(1500));
        c.menu_delay = "".into();
        assert_eq!(c.delay_ms(), Some(0));
        c.menu_delay = "abc".into();
        assert_eq!(c.delay_ms(), None);
    }

    #[test]
    fn dpapi_round_trip() {
        let e = dpapi::encrypt("secret key").unwrap();
        assert_eq!(dpapi::decrypt(&e).as_deref(), Some("secret key"));
    }

    /// The worker against a fake OBS (with a password): logs in, lists scenes, switches to the
    /// song scene when the song file gets text and to the menu scene after the delay.
    #[test]
    fn worker_switches_scenes() {
        use tokio::net::TcpListener;
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (sw_tx, mut sw_rx) = mpsc::unbounded_channel::<String>();
            tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
                let (salt, ch) = ("c2FsdA==", "Y2hhbGxlbmdl");
                ws.send(WsMsg::text(json!({"op":0,"d":{"rpcVersion":1,"authentication":{"challenge":ch,"salt":salt}}}).to_string())).await.unwrap();
                let id: Value = serde_json::from_str(&ws.next().await.unwrap().unwrap().into_text().unwrap()).unwrap();
                let secret = B64.encode(Sha256::digest(format!("hunter2{salt}").as_bytes()));
                let want = B64.encode(Sha256::digest(format!("{secret}{ch}").as_bytes()));
                if id["d"]["authentication"] != want.as_str() {
                    ws.close(None).await.ok();
                    return;
                }
                ws.send(WsMsg::text(json!({"op":2,"d":{"negotiatedRpcVersion":1}}).to_string())).await.unwrap();
                while let Some(Ok(m)) = ws.next().await {
                    let Ok(t) = m.into_text() else { continue };
                    let r: Value = serde_json::from_str(&t).unwrap();
                    let d = &r["d"];
                    let data = match d["requestType"].as_str().unwrap() {
                        "GetSceneList" => json!({"scenes":[{"sceneName":"Menus"},{"sceneName":"Song"}]}),
                        "SetCurrentProgramScene" => {
                            sw_tx.send(d["requestData"]["sceneName"].as_str().unwrap().to_string()).unwrap();
                            json!({})
                        }
                        _ => json!({}),
                    };
                    let resp = json!({"op":7,"d":{"requestType":d["requestType"],"requestId":d["requestId"],"requestStatus":{"result":true,"code":100},"responseData":data}});
                    ws.send(WsMsg::text(resp.to_string())).await.unwrap();
                }
            });

            let dir = std::env::temp_dir().join(format!("truss-chss-test-{port}"));
            std::fs::create_dir_all(&dir).unwrap();
            let song = dir.join("currentsong.txt");
            std::fs::write(&song, "").unwrap();
            let cfg = Config {
                host: "127.0.0.1".into(),
                port: port.to_string(),
                password: "hunter2".into(),
                song_scene: "Song".into(),
                menu_scene: "Menus".into(),
                menu_delay: "0.3".into(),
                song_file: song.to_string_lossy().into_owned(),
                ..Default::default()
            };
            let mut events = Box::pin(worker());
            let Some(Event::Ready(tx)) = events.next().await else { panic!("no Ready") };
            tx.send(Cmd::Connect(cfg)).unwrap();
            let mut got_scenes = None;
            loop {
                match tokio::time::timeout(Duration::from_secs(5), events.next()).await.unwrap().unwrap() {
                    Event::Scenes(s) => got_scenes = Some(s),
                    Event::Conn(Conn::Connected, _) => break,
                    Event::Conn(Conn::Failed, e) => panic!("{e}"),
                    _ => {}
                }
            }
            assert_eq!(got_scenes.unwrap(), ["Song", "Menus"], "dock order (reversed)");
            // Drain events in the background so the worker never blocks.
            tokio::spawn(async move { while events.next().await.is_some() {} });

            std::fs::write(&song, "Artist
Song name
Charter").unwrap();
            let s = tokio::time::timeout(Duration::from_secs(3), sw_rx.recv()).await.unwrap().unwrap();
            assert_eq!(s, "Song");
            let t0 = Instant::now();
            std::fs::write(&song, "").unwrap();
            let s = tokio::time::timeout(Duration::from_secs(3), sw_rx.recv()).await.unwrap().unwrap();
            assert_eq!(s, "Menus");
            assert!(t0.elapsed() >= Duration::from_millis(300), "waited for the menu delay");
            drop(tx);
            std::fs::remove_dir_all(&dir).ok();
        });
    }

    /// A tiny HTTP server: answers each request with `respond(request text)` -> (status, body).
    const CRLF: &str = "\u{000D}\u{000A}";
    const CRLF2: &str = "\u{000D}\u{000A}\u{000D}\u{000A}";

    async fn fake_http(port: u16, respond: fn(&str) -> (u16, String)) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = l.accept().await.unwrap();
                let mut buf = vec![0u8; 65536];
                let mut n = 0;
                // read headers, then the body by Content-Length
                loop {
                    n += s.read(&mut buf[n..]).await.unwrap();
                    let t = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(h) = t.find(CRLF2) {
                        let len = t.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                        if n >= h + 4 + len {
                            break;
                        }
                    }
                }
                let (code, body) = respond(&String::from_utf8_lossy(&buf[..n]));
                let head = ["HTTP/1.1 ", &code.to_string(), " X", CRLF, "Content-Type: application/json", CRLF].concat();
                let resp = [head, format!("Content-Length: {}", body.len()), CRLF.into(), "Connection: close".into(), CRLF2.into(), body].concat();
                s.write_all(resp.as_bytes()).await.unwrap();
            }
        });
    }

    #[test]
    fn ouvert_and_twitch() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            fake_http(4467, |req| {
                if !req.contains("x-api-key: k1") {
                    return (401, r#"{"status":false}"#.into());
                }
                if req.starts_with("GET /API/SongRequests/Next") {
                    return (200, r#"{"status":true,"message":"Now playing: B"}"#.into());
                }
                (200, r#"{"status":true,"data":[{"request":"A: One","requester_name":"viewer","length":139},{"request":"B: Two","requester_name":"","length":"61"}]}"#.into())
            })
            .await;
            fake_http(4468, |req| {
                if req.starts_with("POST /oauth2/token") && req.contains("grant_type=refresh_token") {
                    return (200, r#"{"access_token":"new","refresh_token":"r2"}"#.into());
                }
                if req.starts_with("POST /helix/chat/messages") {
                    if req.contains("Bearer old") {
                        return (401, r#"{"message":"Invalid OAuth token"}"#.into());
                    }
                    assert!(req.contains(r#""message":"!playnext""#));
                    return (200, r#"{"data":[{"message_id":"1","is_sent":true}]}"#.into());
                }
                (404, "{}".into())
            })
            .await;
            let q = ouvert_queue("k1".into()).await.unwrap();
            assert_eq!(q, ["A: One   -   viewer   (2:19)", "B: Two   (1:01)"]);
            assert!(ouvert_queue("bad".into()).await.is_err());

            let mut c = Config { ouvert_private_key: "k1".into(), next_chat_command: "!playnext".into(), ..Default::default() };
            assert_eq!(next_song(c.clone()).await.message, "Now playing: B");
            // Signed in with an expired token: refreshed, then sent.
            c.twitch_access_token = "old".into();
            c.twitch_refresh_token = "r1".into();
            c.twitch_user_id = "42".into();
            let r = next_song(c).await;
            assert_eq!(r.message, "Sent !playnext to chat");
            assert!(r.via_chat);
            let t = r.tokens.unwrap();
            assert_eq!((t.access.as_str(), t.refresh.as_str()), ("new", "r2"));
        });
    }

    /// Settings round-trip in a scratch folder; other lines in the file are kept.
    #[test]
    fn settings_round_trip() {
        let Some(d) = std::env::var_os("TRUSS_TEST_CHSS_DIR") else {
            eprintln!("Skipping: set TRUSS_TEST_CHSS_DIR to a scratch folder");
            return;
        };
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(ini_path(), "[settings]\r\nhost=\"1.2.3.4\"\r\nwindow=\"1,2,3,4,0\"\r\nouvert_key=\"plain\"\r\n").unwrap();
        let mut c = load();
        assert_eq!(c.host, "1.2.3.4");
        assert_eq!(c.ouvert_key, "plain");
        let saved = std::fs::read_to_string(ini_path()).unwrap();
        assert!(saved.contains("ouvert_key=\"dpapi:"), "plain secret re-saved encrypted");
        assert!(saved.contains("window=\"1,2,3,4,0\""));
        c.song_scene = "Song \"A\"".into();
        c.mode = 1;
        save(&c).unwrap();
        let c2 = load();
        assert_eq!(c2.song_scene, "Song \"A\"");
        assert_eq!(c2.mode, 1);
        assert_eq!(c2.ouvert_key, "plain");
    }
}
