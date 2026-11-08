//! Статус в Discord — «Слушает Tune Music», как у Tune Client.
//!
//! Работает в своём потоке: приложение присылает трек, поток держит связь с Discord на этом
//! компьютере (именованный канал `discord-ipc-N`) и сам подключается заново, если Discord
//! запустили позже или перезапустили. Своё приложение Discord регистрировать не нужно:
//! название активности задаётся полем `name`, картинки — ссылками.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::api;

/// Приложение Discord по умолчанию — то же, что у Tune Client. Его название не видно:
/// Tune подставляет своё.
const DEFAULT_APP_ID: &str = "1300258490815741952";
pub const TUNE_SITE: &str = "https://tune-music.github.io/tune/";
/// Значок Tune: Discord берёт картинки только по ссылке.
const TUNE_ICON: &str = "https://raw.githubusercontent.com/tune-music/tune/main/src-tauri/icons/128x128@2x.png";

/// Куда ведёт клик по названию, исполнителю, обложке или значку.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Track,
    Album,
    Artist,
    Tune,
    Nothing,
}

impl Target {
    pub const ALL: [Target; 5] = [Target::Track, Target::Album, Target::Artist, Target::Tune, Target::Nothing];

    fn code(self) -> &'static str {
        match self {
            Target::Track => "track",
            Target::Album => "album",
            Target::Artist => "artist",
            Target::Tune => "tune",
            Target::Nothing => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Target::Track => "Трек",
            Target::Album => "Альбом",
            Target::Artist => "Исполнитель",
            Target::Tune => "Сайт Tune",
            Target::Nothing => "Никуда",
        }
    }

    fn parse(value: &Value, base: Target) -> Target {
        Target::ALL.into_iter().find(|target| value.as_str() == Some(target.code())).unwrap_or(base)
    }
}

/// Что видно рядом с ником в списке участников.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    Name,
    Track,
    Artist,
}

impl Display {
    pub const ALL: [Display; 3] = [Display::Name, Display::Track, Display::Artist];

    fn code(self) -> &'static str {
        match self {
            Display::Name => "name",
            Display::Track => "details",
            Display::Artist => "state",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Display::Name => "Название",
            Display::Track => "Трек",
            Display::Artist => "Исполнитель",
        }
    }
}

/// Настройки статуса, как во вкладке «Discord» у Tune Client.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub enabled: bool,
    /// Название активности: «Слушает <name>».
    pub name: String,
    pub display: Display,
    pub title_link: Target,
    pub artist_link: Target,
    pub cover_link: Target,
    pub icon_link: Target,
    /// Кнопка «Tune Music» со ссылкой на сайт Tune.
    pub site_button: bool,
    pub show_cover: bool,
    pub show_progress: bool,
    /// На паузе: показывать «Пауза» или убирать статус.
    pub show_paused: bool,
    pub small_icon: bool,
    /// Своя картинка значка (https). Пусто — значок Tune.
    pub icon_url: String,
    /// Своё приложение Discord. Пусто — встроенное.
    pub app_id: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            enabled: true,
            name: "Tune Music".to_string(),
            display: Display::Name,
            title_link: Target::Track,
            artist_link: Target::Artist,
            cover_link: Target::Track,
            icon_link: Target::Tune,
            site_button: true,
            show_cover: true,
            show_progress: true,
            show_paused: false,
            small_icon: true,
            icon_url: String::new(),
            app_id: String::new(),
        }
    }
}

fn parse(v: &Value) -> Prefs {
    let base = Prefs::default();
    let flag = |key: &str, base: bool| v[key].as_bool().unwrap_or(base);
    let line = |key: &str, base: &str| v[key].as_str().map_or_else(|| base.to_string(), str::to_string);
    Prefs {
        enabled: flag("enabled", base.enabled),
        name: line("name", &base.name),
        display: Display::ALL.into_iter().find(|display| v["statusDisplay"].as_str() == Some(display.code())).unwrap_or(base.display),
        title_link: Target::parse(&v["titleLink"], base.title_link),
        artist_link: Target::parse(&v["artistLink"], base.artist_link),
        cover_link: Target::parse(&v["coverLink"], base.cover_link),
        icon_link: Target::parse(&v["iconLink"], base.icon_link),
        site_button: flag("siteButton", base.site_button),
        show_cover: flag("showCover", base.show_cover),
        show_progress: flag("showProgress", base.show_progress),
        show_paused: flag("showPaused", base.show_paused),
        small_icon: flag("smallIcon", base.small_icon),
        icon_url: line("iconUrl", &base.icon_url),
        app_id: line("appId", &base.app_id),
    }
}

pub fn load() -> Prefs {
    let text = std::fs::read(api::data_dir().join("discord.json")).unwrap_or_default();
    parse(&serde_json::from_slice(&text).unwrap_or(Value::Null))
}

pub fn save(prefs: &Prefs) {
    let value = json!({
        "enabled": prefs.enabled, "name": prefs.name, "statusDisplay": prefs.display.code(),
        "titleLink": prefs.title_link.code(), "artistLink": prefs.artist_link.code(), "coverLink": prefs.cover_link.code(), "iconLink": prefs.icon_link.code(),
        "siteButton": prefs.site_button, "showCover": prefs.show_cover, "showProgress": prefs.show_progress, "showPaused": prefs.show_paused,
        "smallIcon": prefs.small_icon, "iconUrl": prefs.icon_url, "appId": prefs.app_id,
    });
    let dir = api::data_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("discord.json"), value.to_string());
    }
}

/// Что играет — ровно то, что нужно статусу.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover: String,
    pub url: String,
    pub album_url: String,
    pub artist_url: String,
    pub playing: bool,
    /// Секунды от начала трека.
    pub position: f64,
    /// Длительность в секундах, 0 — неизвестна.
    pub duration: f64,
}

impl Track {
    pub fn of(track: &api::Track, playing: bool, position: Duration) -> Track {
        let album = track.album.as_deref().filter(|id| !id.is_empty());
        let artist = track.artist_list.first().map(|(_, id)| id.as_str()).filter(|id| !id.is_empty());
        Track {
            title: track.title.clone(),
            artist: track.artists.clone(),
            album: track.album_title.clone(),
            cover: track.cover.as_deref().map(|uri| format!("https://{}", uri.trim_start_matches("https://").replace("%%", "400x400"))).unwrap_or_default(),
            url: album.map(|album| format!("https://music.yandex.ru/album/{album}/track/{}", track.id)).unwrap_or_else(|| format!("https://music.yandex.ru/track/{}", track.id)),
            album_url: album.map(|album| format!("https://music.yandex.ru/album/{album}")).unwrap_or_default(),
            artist_url: artist.map(|artist| format!("https://music.yandex.ru/artist/{artist}")).unwrap_or_default(),
            playing,
            position: position.as_secs_f64(),
            duration: track.duration.as_secs_f64(),
        }
    }
}

enum Command {
    Prefs(Prefs),
    Track(Option<Track>),
}

#[derive(Clone)]
pub struct Handle(Sender<Command>);

impl Handle {
    pub fn configure(&self, prefs: &Prefs) {
        let _ = self.0.send(Command::Prefs(prefs.clone()));
    }

    pub fn update(&self, track: Option<Track>) {
        let _ = self.0.send(Command::Track(track));
    }
}

fn app_id(prefs: &Prefs) -> String {
    let id = prefs.app_id.trim();
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) { id.to_string() } else { DEFAULT_APP_ID.to_string() }
}

pub fn spawn(prefs: Prefs) -> Handle {
    let (tx, rx) = mpsc::channel::<Command>();
    let _ = std::thread::Builder::new().name("discord".into()).spawn(move || {
        let mut prefs = prefs;
        let mut track: Option<Track> = None;
        let mut pipe: Option<Pipe> = None;
        let mut dirty = true;
        loop {
            match rx.recv_timeout(Duration::from_secs(15)) {
                Ok(Command::Prefs(next)) => {
                    if app_id(&next) != app_id(&prefs) || !next.enabled {
                        close(&mut pipe);
                    }
                    prefs = next;
                    dirty = true;
                }
                Ok(Command::Track(next)) => {
                    if next != track {
                        track = next;
                        dirty = true;
                    }
                }
                // Discord был закрыт — время от времени пробуем снова.
                Err(RecvTimeoutError::Timeout) => dirty |= pipe.is_none(),
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if !dirty || !prefs.enabled {
                continue;
            }
            if pipe.is_none() {
                pipe = Pipe::connect(&app_id(&prefs));
            }
            let Some(open) = pipe.as_mut() else { continue };
            let activity = match &track {
                Some(t) if !t.title.is_empty() && (t.playing || prefs.show_paused) => activity(t, &prefs),
                _ => Value::Null,
            };
            if open.set_activity(activity).is_ok() {
                dirty = false;
            } else {
                pipe = None;
            }
        }
        close(&mut pipe);
    });
    Handle(tx)
}

fn close(pipe: &mut Option<Pipe>) {
    if let Some(mut open) = pipe.take() {
        let _ = open.set_activity(Value::Null);
    }
}

/// Канал к Discord: кадры «код, длина, JSON».
struct Pipe {
    file: File,
    nonce: u64,
}

impl Pipe {
    fn connect(app_id: &str) -> Option<Pipe> {
        (0..10).find_map(|index| {
            let file = OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{index}")).ok()?;
            let mut pipe = Pipe { file, nonce: 0 };
            pipe.send(0, &json!({ "v": 1, "client_id": app_id })).ok()?;
            let (op, reply) = pipe.receive().ok()?;
            (op == 1 && reply["evt"] == "READY").then_some(pipe)
        })
    }

    fn send(&mut self, op: u32, value: &Value) -> std::io::Result<()> {
        let body = value.to_string();
        let mut frame = Vec::with_capacity(8 + body.len());
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
        frame.extend_from_slice(body.as_bytes());
        self.file.write_all(&frame)
    }

    fn receive(&mut self) -> std::io::Result<(u32, Value)> {
        let mut head = [0u8; 8];
        self.file.read_exact(&mut head)?;
        let op = u32::from_le_bytes(head[..4].try_into().unwrap());
        let length = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
        let mut body = vec![0; length.min(1 << 20)];
        self.file.read_exact(&mut body)?;
        Ok((op, serde_json::from_slice(&body).unwrap_or(Value::Null)))
    }

    /// Ставит или убирает (`Null`) статус и ждёт ответа: иначе ответы копятся в канале.
    fn set_activity(&mut self, activity: Value) -> std::io::Result<()> {
        self.nonce += 1;
        let request = json!({ "cmd": "SET_ACTIVITY", "args": { "pid": std::process::id(), "activity": activity }, "nonce": self.nonce.to_string() });
        self.send(1, &request)?;
        let (_, reply) = self.receive()?;
        if reply["evt"] == "ERROR" {
            api::log(&format!("Discord: {}", reply["data"]["message"].as_str().unwrap_or("ошибка")));
        }
        Ok(())
    }
}

/// Первые `max` байт строки, не разрезая символ.
fn clip_bytes(s: &str, max: usize) -> String {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i + c.len_utf8() > max {
            break;
        }
        end = i + c.len_utf8();
    }
    s[..end].trim_end().to_string()
}

/// Discord считает длину полей в байтах UTF-8: от 2 до 128. Русская буква — два байта.
fn fit(s: &str) -> String {
    let mut s = clip_bytes(s, 128);
    while s.chars().count() < 2 {
        s.push(' ');
    }
    s
}

fn web_url(u: &str) -> Option<String> {
    let u = u.trim();
    (u.starts_with("https://") || u.starts_with("http://")).then(|| u.to_string())
}

fn link(t: &Track, target: Target) -> Option<String> {
    match target {
        Target::Track => web_url(&t.url),
        Target::Album => web_url(&t.album_url),
        Target::Artist => web_url(&t.artist_url),
        Target::Tune => Some(TUNE_SITE.to_string()),
        Target::Nothing => None,
    }
}

fn activity(t: &Track, prefs: &Prefs) -> Value {
    let name = if prefs.name.trim().is_empty() { "Tune Music" } else { prefs.name.trim() };
    let icon = Some(prefs.icon_url.trim()).filter(|u| u.starts_with("https://")).unwrap_or(TUNE_ICON).to_string();
    let mut assets = json!({ "large_text": fit(if t.album.is_empty() { &t.title } else { &t.album }) });
    assets["large_image"] = json!(if prefs.show_cover && t.cover.starts_with("https://") { t.cover.clone() } else { icon.clone() });
    if let Some(url) = link(t, prefs.cover_link) {
        assets["large_url"] = json!(url);
    }
    if prefs.small_icon {
        assets["small_image"] = json!(icon);
        assets["small_text"] = json!(if t.playing { "Играет" } else { "Пауза" });
        if let Some(url) = link(t, prefs.icon_link) {
            assets["small_url"] = json!(url);
        }
    }
    let display = match prefs.display {
        Display::Name => 0,
        Display::Artist => 1,
        Display::Track => 2,
    };
    let state = if t.playing { fit(if t.artist.is_empty() { name } else { &t.artist }) } else { fit(&format!("Пауза · {}", t.artist)) };
    // Тип 2 — «Слушает».
    let mut activity = json!({ "name": fit(name), "type": 2, "status_display_type": display, "details": fit(&t.title), "state": state, "assets": assets });
    if let Some(url) = link(t, prefs.title_link) {
        activity["details_url"] = json!(url);
    }
    if let Some(url) = link(t, prefs.artist_link) {
        activity["state_url"] = json!(url);
    }
    // Подпись латиницей: лимит кнопки (32) Discord считает в байтах.
    if prefs.site_button {
        activity["buttons"] = json!([{ "label": "Tune Music", "url": TUNE_SITE }]);
    }
    if prefs.show_progress && t.playing && t.duration > 0.0 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let start = now - (t.position * 1000.0) as i64;
        activity["timestamps"] = json!({ "start": start, "end": start + (t.duration * 1000.0) as i64 });
    }
    activity
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> Track {
        Track {
            title: "Клей".into(),
            artist: "CUPSIZE".into(),
            cover: "https://avatars.yandex.net/x/400x400".into(),
            url: "https://music.yandex.ru/album/1/track/2".into(),
            album_url: "https://music.yandex.ru/album/1".into(),
            artist_url: "https://music.yandex.ru/artist/3".into(),
            playing: true,
            ..Default::default()
        }
    }

    #[test]
    fn activity_like_tune_client() {
        let json = activity(&track(), &Prefs::default());
        assert_eq!(json["type"], 2);
        assert_eq!(json["details_url"], "https://music.yandex.ru/album/1/track/2");
        assert_eq!(json["state_url"], "https://music.yandex.ru/artist/3");
        assert_eq!(json["assets"]["large_url"], "https://music.yandex.ru/album/1/track/2");
        assert_eq!(json["assets"]["small_url"], TUNE_SITE);
        assert_eq!(json["buttons"][0]["label"], "Tune Music");
        let paused = activity(&Track { playing: false, ..track() }, &Prefs::default());
        assert_eq!(paused["state"], "Пауза · CUPSIZE");
        assert!(paused.get("timestamps").is_none());
    }

    #[test]
    fn prefs_round_trip() {
        let prefs = Prefs { name: "Яндекс Музыка".into(), display: Display::Track, cover_link: Target::Album, show_paused: true, ..Prefs::default() };
        let saved = json!({ "name": prefs.name, "statusDisplay": "details", "coverLink": "album", "showPaused": true });
        assert_eq!(parse(&saved), prefs);
        assert_eq!(parse(&Value::Null), Prefs::default());
    }

    #[test]
    fn texts_fit_in_bytes() {
        assert_eq!(clip_bytes("Слушать в Яндекс Музыке", 32), "Слушать в Яндекс");
        assert!(fit(&"ё".repeat(100)).len() <= 128);
        assert_eq!(fit("a"), "a ");
    }
}
