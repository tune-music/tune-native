//! Ynison — служба Яндекса, через которую устройства одного аккаунта делят очередь, место в
//! треке и паузу. Подключение как у официального клиента: сначала распределитель сообщает
//! адрес узла, потом узел присылает общее состояние и принимает наши изменения.
//!
//! Связь живёт в своём потоке: окно только кладёт исходящие сообщения в канал и забирает
//! пришедшие состояния на каждом такте.

use serde_json::{json, Value};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const HOST: &str = "ynison.music.yandex.ru";
const REDIRECTOR: &str = "/redirector.YnisonRedirectService/GetRedirectToYnison";
const HUB: &str = "/ynison_state.YnisonStateService/PutYnisonState";
/// Так представляется официальный клиент для компьютера.
const APP_NAME: &str = "Desktop";
const APP_VERSION: &str = "5.120.0";
pub const TITLE: &str = "Tune";
const VOLUME_STEPS: u32 = 20;
/// Паузы между попытками подключиться, если узел не просил ждать дольше.
const RETRY: [u64; 5] = [2, 5, 15, 60, 300];
/// Как часто проверять канал исходящих, пока узел молчит.
const POLL: Duration = Duration::from_millis(150);
const PING: Duration = Duration::from_secs(30);

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// Устройство аккаунта, как его видит Ynison.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub title: String,
    /// WEB, ANDROID, IOS, SMART_SPEAKER и другие.
    pub kind: String,
    pub can_play: bool,
}

/// Трек в общей очереди.
#[derive(Clone, Debug, PartialEq)]
pub struct Playable {
    pub id: String,
    pub album: Option<String>,
    pub title: String,
    pub cover: Option<String>,
}

/// Общее состояние, пришедшее от узла.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shared {
    pub devices: Vec<Device>,
    pub active: Option<String>,
    pub queue: Vec<Playable>,
    pub index: Option<usize>,
    pub entity_id: String,
    pub entity_type: String,
    pub paused: bool,
    pub progress: Duration,
    pub duration: Duration,
    pub speed: f32,
    /// Какое устройство последним меняло паузу и место, и какое — очередь.
    pub status_by: String,
    pub queue_by: String,
}

/// Что видно окну: связь, ошибка и пришедшие состояния по порядку.
#[derive(Default)]
struct Inbox {
    connected: bool,
    error: Option<String>,
    states: Vec<Shared>,
}

/// Наше состояние для отправки: очередь, место и пауза.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Local {
    pub queue: Vec<Playable>,
    pub index: Option<usize>,
    pub entity_id: String,
    pub entity_type: String,
    pub from: String,
    pub paused: bool,
    pub progress: Duration,
    pub duration: Duration,
    pub speed: f32,
}

pub struct Ynison {
    device_id: String,
    out: Sender<String>,
    inbox: Arc<Mutex<Inbox>>,
    /// Состояние для первого сообщения после (пере)подключения.
    full: Arc<Mutex<Local>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Ynison {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Ynison {
    pub fn start(token: String, local: Local) -> Self {
        let device_id = device_id();
        let (out, rx) = mpsc::channel();
        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let full = Arc::new(Mutex::new(local));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = Worker { token, device_id: device_id.clone(), rx, inbox: inbox.clone(), full: full.clone(), stop: stop.clone() };
        std::thread::Builder::new().name("ynison".into()).spawn(move || worker.run()).ok();
        Self { device_id, out, inbox, full, stop }
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn connected(&self) -> bool {
        self.inbox.lock().unwrap().connected
    }

    /// Пришедшие с прошлого раза состояния.
    pub fn take(&self) -> Vec<Shared> {
        std::mem::take(&mut self.inbox.lock().unwrap().states)
    }

    /// Очередь и статус целиком. `intercept` — включили здесь: это устройство становится
    /// играющим, остальные встают на паузу.
    pub fn send_state(&self, local: &Local, intercept: bool) {
        *self.full.lock().unwrap() = local.clone();
        let state = player_state(local, &self.device_id);
        let _ = self.out.send(envelope(json!({ "update_player_state": { "player_state": state } }), intercept));
    }

    /// Только пауза, место и скорость.
    pub fn send_status(&self, local: &Local) {
        *self.full.lock().unwrap() = local.clone();
        let status = status(local, &self.device_id);
        let _ = self.out.send(envelope(json!({ "update_playing_status": { "playing_status": status } }), false));
    }

    /// Передать воспроизведение другому устройству; `None` — никому.
    pub fn send_active(&self, device: Option<&str>) {
        let _ = self.out.send(envelope(json!({ "update_active_device": { "device_id_optional": device } }), false));
    }

    /// Прочитать общий узел заново: после ошибки или долгой паузы.
    pub fn set_full(&self, local: &Local) {
        *self.full.lock().unwrap() = local.clone();
    }
}

struct Worker {
    token: String,
    device_id: String,
    rx: Receiver<String>,
    inbox: Arc<Mutex<Inbox>>,
    full: Arc<Mutex<Local>>,
    stop: Arc<AtomicBool>,
}

impl Worker {
    fn run(self) {
        let mut tries = 0usize;
        while !self.stop.load(Ordering::SeqCst) {
            let wait = match self.session() {
                Ok(()) => {
                    tries = 0;
                    Duration::from_secs(RETRY[0])
                }
                Err(Failure { message, go_away }) => {
                    crate::api::log(&format!("Ynison: {message}"));
                    let mut inbox = self.inbox.lock().unwrap();
                    inbox.connected = false;
                    inbox.error = Some(message);
                    drop(inbox);
                    let wait = go_away.unwrap_or(Duration::from_secs(RETRY[tries.min(RETRY.len() - 1)]));
                    tries += 1;
                    wait
                }
            };
            self.inbox.lock().unwrap().connected = false;
            // Ждём частями, чтобы закрытие программы не висело на паузе.
            let until = Instant::now() + wait;
            while Instant::now() < until && !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }

    /// Одно подключение: от распределителя до обрыва. `Ok` — связь закрылась сама.
    fn session(&self) -> Result<(), Failure> {
        let redirect = self.redirect()?;
        let host = redirect["host"].as_str().ok_or_else(|| Failure::new("распределитель не дал адрес узла"))?;
        let mut extra = serde_json::Map::new();
        extra.insert("Ynison-Redirect-Ticket".into(), redirect["redirect_ticket"].clone());
        let session = match &redirect["session_id"] {
            Value::String(id) => id.clone(),
            other => other.to_string(),
        };
        extra.insert("Ynison-Session-Id".into(), Value::String(session));
        let mut socket = self.open(&format!("{}{host}{HUB}", scheme()), extra)?;
        set_timeout(&mut socket, Some(POLL));
        let full = self.full.lock().unwrap().clone();
        let first = json!({
            "update_full_state": {
                "player_state": player_state(&full, &self.device_id),
                "device": device(&self.device_id),
                "is_currently_active": false,
                "sync_state_from_eov_optional": null,
            }
        });
        send(&mut socket, envelope(first, false))?;
        // Старые исходящие относятся к прошлому подключению: полное состояние их уже включает.
        while self.rx.try_recv().is_ok() {}
        let mut pinged = Instant::now();
        loop {
            if self.stop.load(Ordering::SeqCst) {
                let _ = socket.close(None);
                return Ok(());
            }
            while let Ok(text) = self.rx.try_recv() {
                send(&mut socket, text)?;
            }
            if pinged.elapsed() >= PING {
                socket.send(Message::Ping(Vec::new().into())).map_err(|e| Failure::new(&e.to_string()))?;
                pinged = Instant::now();
            }
            match socket.read() {
                Ok(Message::Text(text)) => self.receive(text.as_str())?,
                Ok(Message::Close(_)) => return Ok(()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => return Ok(()),
                Err(e) => return Err(Failure::new(&format!("связь с узлом оборвалась: {e}"))),
            }
        }
    }

    fn redirect(&self) -> Result<Value, Failure> {
        let mut socket = self.open(&format!("{}{}{REDIRECTOR}", scheme(), redirector_host()), serde_json::Map::new())?;
        set_timeout(&mut socket, Some(Duration::from_secs(15)));
        loop {
            match socket.read() {
                Ok(Message::Text(text)) => {
                    let _ = socket.close(None);
                    let v: Value = serde_json::from_str(text.as_str()).map_err(|_| Failure::new("распределитель ответил непонятно"))?;
                    return match v.get("error") {
                        Some(error) => Err(Failure::from_error(error)),
                        None => Ok(v),
                    };
                }
                Ok(Message::Close(_)) => return Err(Failure::new("распределитель закрыл связь")),
                Ok(_) => {}
                Err(e) => return Err(Failure::new(&format!("распределитель недоступен: {e}"))),
            }
        }
    }

    fn open(&self, url: &str, extra: serde_json::Map<String, Value>) -> Result<Socket, Failure> {
        let mut request = url.into_client_request().map_err(|e| Failure::new(&e.to_string()))?;
        let headers = request.headers_mut();
        let protocol = format!("Bearer, v2, {}", encode(&auth(&self.device_id, &self.token, extra)));
        headers.insert("Sec-WebSocket-Protocol", protocol.parse().map_err(|_| Failure::new("неверный заголовок"))?);
        headers.insert("Origin", "https://music.yandex.ru".parse().unwrap());
        match tungstenite::connect(request) {
            Ok((socket, _)) => Ok(socket),
            Err(tungstenite::Error::Http(response)) => {
                let body = response.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default().to_string();
                Err(Failure::new(&format!("узел ответил {}: {}", response.status(), body.chars().take(200).collect::<String>())))
            }
            Err(e) => Err(Failure::new(&format!("не удалось подключиться: {e}"))),
        }
    }

    fn receive(&self, text: &str) -> Result<(), Failure> {
        let v: Value = serde_json::from_str(text).map_err(|_| Failure::new("узел прислал непонятное"))?;
        if let Some(error) = v.get("error") {
            return Err(Failure::from_error(error));
        }
        let state = parse_state(&v);
        let mut inbox = self.inbox.lock().unwrap();
        inbox.connected = true;
        inbox.error = None;
        inbox.states.push(state);
        Ok(())
    }
}

struct Failure {
    message: String,
    go_away: Option<Duration>,
}

impl Failure {
    fn new(message: &str) -> Self {
        Self { message: message.to_string(), go_away: None }
    }

    /// Ошибка от Ynison: узел может попросить не возвращаться какое-то время.
    fn from_error(error: &Value) -> Self {
        let go_away = error["details"]["ynison-go-away-for-seconds"].as_str().and_then(|s| s.parse().ok()).or(error["details"]["ynison-go-away-for-seconds"].as_u64()).map(Duration::from_secs);
        let message = error["message"].as_str().unwrap_or("ошибка Ynison").to_string();
        Self { message: format!("{} ({})", message, error["http_code"]), go_away }
    }
}

fn send(socket: &mut Socket, text: String) -> Result<(), Failure> {
    socket.send(Message::text(text)).map_err(|e| Failure::new(&format!("не удалось отправить: {e}")))
}

fn set_timeout(socket: &mut Socket, timeout: Option<Duration>) {
    let tcp = match socket.get_mut() {
        MaybeTlsStream::Plain(tcp) => tcp,
        MaybeTlsStream::Rustls(tls) => tls.get_mut(),
        _ => return,
    };
    let _ = tcp.set_read_timeout(timeout);
}

/// Для проверки с поддельным узлом: `TUNE_YNISON=ws://127.0.0.1:port`.
fn scheme() -> String {
    match std::env::var("TUNE_YNISON") {
        Ok(url) if url.starts_with("ws://") => "ws://".into(),
        _ => "wss://".into(),
    }
}

fn redirector_host() -> String {
    std::env::var("TUNE_YNISON").ok().and_then(|url| url.split_once("://").map(|(_, host)| host.trim_end_matches('/').to_string())).unwrap_or_else(|| HOST.to_string())
}

/// Номер этого устройства: один на установку, как у официального клиента.
fn device_id() -> String {
    let path = crate::api::data_dir().join("ynison-device");
    if let Some(id) = std::fs::read_to_string(&path).ok().map(|id| id.trim().to_string()).filter(|id| !id.is_empty()) {
        return id;
    }
    let id = random_id();
    let _ = std::fs::create_dir_all(crate::api::data_dir());
    let _ = std::fs::write(&path, &id);
    id
}

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    let seed = now_ms() as u64 ^ (std::process::id() as u64) << 32;
    let mut x = seed | 1;
    for byte in &mut bytes {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *byte = x as u8;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

/// Заголовки, которые Ynison ждёт внутри `Sec-WebSocket-Protocol`.
fn auth(device_id: &str, token: &str, extra: serde_json::Map<String, Value>) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("Ynison-Device-Id".into(), device_id.into());
    map.insert("Ynison-Device-Info".into(), json!({ "app_name": APP_NAME, "app_version": APP_VERSION, "type": 1 }).to_string().into());
    map.extend(extra);
    map.insert("authorization".into(), format!("OAuth {token}").into());
    Value::Object(map)
}

/// Как `encodeURIComponent`.
fn encode(v: &Value) -> String {
    let mut out = String::new();
    for byte in v.to_string().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn version(device_id: &str) -> Value {
    let random = (now_ms() as u64).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 2;
    json!({ "device_id": device_id, "version": random, "timestamp_ms": now_ms() as u64 })
}

fn envelope(mut body: Value, intercept: bool) -> String {
    let id = random_id();
    let map = body.as_object_mut().unwrap();
    map.insert("rid".into(), id.into());
    map.insert("player_action_timestamp_ms".into(), (now_ms() as u64).into());
    map.insert("activity_interception_type".into(), if intercept { "INTERCEPT_EAGER" } else { "DO_NOT_INTERCEPT_BY_DEFAULT" }.into());
    body.to_string()
}

fn device(device_id: &str) -> Value {
    json!({
        "volume": 1.0,
        "capabilities": { "can_be_player": true, "can_be_remote_controller": false, "volume_granularity": VOLUME_STEPS },
        "info": { "app_name": APP_NAME, "app_version": APP_VERSION, "title": TITLE, "device_id": device_id, "type": "WEB_DESKTOP" },
        "volume_info": { "volume": 1.0, "version": null },
        "is_shadow": false,
    })
}

fn status(local: &Local, device_id: &str) -> Value {
    json!({
        "duration_ms": local.duration.as_millis() as u64,
        "paused": local.paused,
        "playback_speed": local.speed,
        "progress_ms": local.progress.as_millis() as u64,
        "version": version(device_id),
    })
}

fn player_state(local: &Local, device_id: &str) -> Value {
    let list: Vec<Value> = local
        .queue
        .iter()
        .map(|track| {
            json!({
                "album_id_optional": track.album,
                "from": local.from,
                "playable_id": track.id,
                "playable_type": "TRACK",
                "title": track.title,
                "cover_url_optional": track.cover,
                "navigation_id_optional": null,
                "playback_action_id_optional": null,
            })
        })
        .collect();
    let entity_type = if local.entity_type.is_empty() { "VARIOUS" } else { local.entity_type.as_str() };
    json!({
        "player_queue": {
            "current_playable_index": local.index.map_or(-1, |i| i as i64),
            "entity_id": local.entity_id,
            "entity_type": entity_type,
            "playable_list": list,
            "options": { "repeat_mode": "NONE" },
            "shuffle_optional": null,
            "entity_context": "BASED_ON_ENTITY_BY_DEFAULT",
            "version": version(device_id),
            "from_optional": local.from,
            "initial_entity_optional": null,
            "adding_options_optional": null,
            "queue": null,
        },
        "status": status(local, device_id),
        "player_queue_inject_optional": null,
    })
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn millis(v: &Value) -> Duration {
    let ms = v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)).unwrap_or(0);
    Duration::from_millis(ms)
}

/// Состояние от узла. Числа в нём бывают и строками.
pub fn parse_state(v: &Value) -> Shared {
    let devices = v["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| !d["is_shadow"].as_bool().unwrap_or(false))
        .filter_map(|d| {
            let info = &d["info"];
            Some(Device {
                id: text_of(&info["device_id"]),
                title: info["title"].as_str().unwrap_or("Устройство").to_string(),
                kind: info["type"].as_str().unwrap_or("").to_string(),
                can_play: d["capabilities"]["can_be_player"].as_bool().unwrap_or(true),
            })
            .filter(|device| !device.id.is_empty())
        })
        .collect();
    let player = &v["player_state"];
    let queue = &player["player_queue"];
    let list: Vec<Playable> = queue["playable_list"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| Playable {
            id: text_of(&p["playable_id"]),
            album: Some(text_of(&p["album_id_optional"])).filter(|a| !a.is_empty()),
            title: p["title"].as_str().unwrap_or("").to_string(),
            cover: p["cover_url_optional"].as_str().map(str::to_string),
        })
        .collect();
    let index = queue["current_playable_index"].as_i64().or_else(|| queue["current_playable_index"].as_str().and_then(|s| s.parse().ok())).and_then(|i| usize::try_from(i).ok()).filter(|i| *i < list.len());
    let status = &player["status"];
    Shared {
        devices,
        active: Some(text_of(&v["active_device_id_optional"])).filter(|id| !id.is_empty()),
        queue: list,
        index,
        entity_id: text_of(&queue["entity_id"]),
        entity_type: queue["entity_type"].as_str().unwrap_or("").to_string(),
        paused: status["paused"].as_bool().unwrap_or(true),
        progress: millis(&status["progress_ms"]),
        duration: millis(&status["duration_ms"]),
        speed: status["playback_speed"].as_f64().unwrap_or(1.0) as f32,
        status_by: text_of(&status["version"]["device_id"]),
        queue_by: text_of(&queue["version"]["device_id"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_browser() {
        assert_eq!(encode(&json!({"a": "OAuth x y"})), "%7B%22a%22%3A%22OAuth%20x%20y%22%7D");
    }

    #[test]
    fn parses_hub_state() {
        let v = json!({
            "devices": [
                {"info": {"device_id": "phone", "title": "Pixel", "type": "ANDROID"}, "capabilities": {"can_be_player": true}},
                {"info": {"device_id": "ghost", "title": "Shadow", "type": "WEB"}, "is_shadow": true}
            ],
            "active_device_id_optional": "phone",
            "player_state": {
                "player_queue": {"current_playable_index": 1, "entity_id": "123", "entity_type": "ALBUM",
                    "playable_list": [{"playable_id": "1", "album_id_optional": "123", "title": "A"}, {"playable_id": "2", "album_id_optional": null, "title": "B"}],
                    "version": {"device_id": "phone", "version": "5", "timestamp_ms": "1"}},
                "status": {"paused": false, "progress_ms": "61000", "duration_ms": 200000, "playback_speed": 1, "version": {"device_id": "phone"}}
            }
        });
        let state = parse_state(&v);
        assert_eq!(state.devices.len(), 1);
        assert_eq!(state.devices[0].title, "Pixel");
        assert_eq!(state.active.as_deref(), Some("phone"));
        assert_eq!(state.index, Some(1));
        assert_eq!(state.queue[0].album.as_deref(), Some("123"));
        assert_eq!(state.queue[1].album, None);
        assert_eq!(state.progress, Duration::from_secs(61));
        assert!(!state.paused);
        assert_eq!(state.status_by, "phone");
    }

    #[test]
    fn state_message_has_queue_and_status() {
        let local = Local { queue: vec![Playable { id: "7".into(), album: Some("8".into()), title: "T".into(), cover: None }], index: Some(0), paused: true, progress: Duration::from_millis(1500), speed: 1.0, ..Local::default() };
        let state = player_state(&local, "me");
        assert_eq!(state["player_queue"]["playable_list"][0]["playable_id"], "7");
        assert_eq!(state["player_queue"]["current_playable_index"], 0);
        assert_eq!(state["player_queue"]["entity_type"], "VARIOUS");
        assert_eq!(state["status"]["progress_ms"], 1500);
        assert_eq!(state["status"]["version"]["device_id"], "me");
        let text = envelope(json!({"update_player_state": {"player_state": state}}), true);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["activity_interception_type"], "INTERCEPT_EAGER");
        assert!(v["rid"].as_str().is_some_and(|rid| rid.len() == 36));
    }
}
