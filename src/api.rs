//! Яндекс Музыка: токен, поиск, библиотека, волна и ссылка на файл трека.
//!
//! Те же запросы делает оригинальный клиент. Данные приложения общеизвестны, их используют
//! открытые клиенты (MarshalX/yandex-music-api).

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

const API: &str = "https://api.music.yandex.net";
const CLIENT: &str = "YandexMusicDesktopAppWindows/5.120.0";
const SIGN_KEY: &[u8] = b"kzqU4XhfCaY6B6JTHODeq5";
const QUALITY: &str = "nq";
const CODECS: [&str; 5] = ["mp3", "aac", "aac-mp4", "he-aac", "he-aac-mp4"];
const TRANSPORT: &str = "encraw";
const WAVE_SEED: &str = "user:onyourwave";
const FROM: &str = "tune-native";

/// Ошибка: токен отозвали или он истёк.
pub const NEED_LOGIN: &str = "NEED_LOGIN";

#[derive(Clone, Debug)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artists: String,
    pub duration: Duration,
    pub cover: Option<String>,
    pub album: Option<String>,
    /// Пачка волны, из которой пришёл трек.
    pub batch: Option<String>,
}

impl Track {
    /// Так трек называют обратная связь и очередь волны: id:альбом.
    pub fn full_id(&self) -> String {
        match &self.album {
            Some(album) => format!("{}:{album}", self.id),
            None => self.id.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub uid: String,
    pub name: String,
    pub plus: bool,
}

/// Откуда берутся треки списка.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Playlist,
    Album,
    Artist,
}

#[derive(Clone, Debug)]
pub struct Playlist {
    pub source: Source,
    pub owner: String,
    pub kind: String,
    pub title: String,
    pub count: u64,
    pub cover: Option<String>,
}

/// Что пользователь закрепил в боковой панели.
#[derive(Clone, Debug)]
pub enum PinTarget {
    List(Playlist),
    Wave(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Pin {
    pub target: PinTarget,
    pub title: String,
    pub subtitle: &'static str,
    pub cover: Option<String>,
    pub round: bool,
}

#[derive(Clone, Debug)]
pub struct Wave {
    pub session: String,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug)]
pub struct FileInfo {
    pub url: String,
    pub key: Option<String>,
    pub codec: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).http_status_as_error(false).build().into()
}

fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("dev.tune.native")
}

fn token_path() -> PathBuf {
    data_dir().join("token")
}

pub fn load_token() -> Option<String> {
    let token = std::fs::read_to_string(token_path()).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

pub fn save_token(token: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir())?;
    std::fs::write(token_path(), token)
}

/// Куки страницы входа: без них Яндекс помнит аккаунт, как браузер.
pub fn login_data_dir() -> PathBuf {
    data_dir().join("login")
}

pub fn logout() {
    let _ = std::fs::remove_file(token_path());
    let _ = std::fs::remove_dir_all(login_data_dir());
}

fn get(path: &str, query: &[(&str, &str)], token: &str) -> Result<Value, String> {
    let mut req = agent()
        .get(format!("{API}/{path}"))
        .header("Authorization", &format!("OAuth {token}"))
        .header("X-Yandex-Music-Client", CLIENT);
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    let resp = req.call().map_err(|e| format!("Не удалось связаться с Яндексом: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        return Err(NEED_LOGIN.into());
    }
    let body = resp.into_body().read_to_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|_| format!("Яндекс ответил {status}"))?;
    if status != 200 {
        let name = v["error"]["name"].as_str().or(v["result"]["message"].as_str()).unwrap_or("ошибка");
        return Err(format!("Яндекс ответил {status}: {name}"));
    }
    Ok(v)
}

fn post(path: &str, content_type: &str, body: String, token: &str) -> Result<Value, String> {
    let resp = agent()
        .post(format!("{API}/{path}"))
        .header("Authorization", &format!("OAuth {token}"))
        .header("X-Yandex-Music-Client", CLIENT)
        .header("Content-Type", content_type)
        .send(body)
        .map_err(|e| format!("Не удалось связаться с Яндексом: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        return Err(NEED_LOGIN.into());
    }
    let body = resp.into_body().read_to_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|_| format!("Яндекс ответил {status}"))?;
    if status != 200 {
        let name = v["error"]["name"].as_str().or(v["result"]["message"].as_str()).unwrap_or("ошибка");
        return Err(format!("Яндекс ответил {status}: {name}"));
    }
    Ok(v)
}

/// Часть ответов лежит в result, часть — сразу в корне.
fn result(v: &Value) -> &Value {
    if v["result"].is_object() || v["result"].is_array() {
        &v["result"]
    } else {
        v
    }
}

fn parse_track(v: &Value) -> Option<Track> {
    let id = match &v["id"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let artists = v["artists"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect::<Vec<_>>().join(", ");
    Some(Track {
        id,
        title: v["title"].as_str()?.to_string(),
        artists,
        duration: Duration::from_millis(v["durationMs"].as_u64().unwrap_or(0)),
        cover: v["coverUri"].as_str().filter(|c| !c.is_empty()).map(str::to_string),
        album: v["albums"][0]["id"].as_u64().map(|a| a.to_string()),
        batch: None,
    })
}

pub fn search(text: &str, token: &str) -> Result<Vec<Track>, String> {
    let v = get("search", &[("text", text), ("type", "track"), ("page", "0")], token)?;
    Ok(v["result"]["tracks"]["results"].as_array().into_iter().flatten().filter_map(parse_track).collect())
}

/// Сырой ответ API для разбора новых запросов: `tune-native --api путь`.
pub fn raw(path: &str, token: &str) -> Result<String, String> {
    get(path, &[], token).map(|v| serde_json::to_string_pretty(&v).unwrap_or_default())
}

pub fn profile(token: &str) -> Result<Profile, String> {
    let v = get("account/status", &[], token)?;
    let v = result(&v);
    let account = &v["account"];
    let uid = account["uid"].as_u64().map(|u| u.to_string()).ok_or("Яндекс не сообщил номер аккаунта")?;
    let name = account["displayName"].as_str().or(account["firstName"].as_str()).or(account["login"].as_str()).unwrap_or_default();
    Ok(Profile { uid, name: name.to_string(), plus: v["plus"]["hasPlus"].as_bool().unwrap_or(false) })
}

/// Номера треков из «Мне нравится», сначала новые.
pub fn liked_ids(uid: &str, token: &str) -> Result<Vec<String>, String> {
    let v = get(&format!("users/{uid}/likes/tracks"), &[], token)?;
    let tracks = result(&v)["library"]["tracks"].as_array().cloned().unwrap_or_default();
    Ok(tracks
        .iter()
        .filter_map(|t| match &t["id"] {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .collect())
}

/// Сведения о треках в том же порядке, что и номера.
pub fn tracks_by_ids(ids: &[String], token: &str) -> Result<Vec<Track>, String> {
    let body = format!("track-ids={}", ids.join("%2C"));
    let v = post("tracks", "application/x-www-form-urlencoded", body, token)?;
    let found: Vec<Track> = v["result"].as_array().into_iter().flatten().filter_map(parse_track).collect();
    Ok(ids.iter().filter_map(|id| found.iter().find(|t| t.id == *id).cloned()).collect())
}

pub fn set_liked(uid: &str, track_id: &str, liked: bool, token: &str) -> Result<(), String> {
    let path = if liked {
        format!("users/{uid}/likes/tracks/add?track-id={track_id}")
    } else {
        format!("users/{uid}/likes/tracks/{track_id}/remove")
    };
    let v = post(&path, "application/x-www-form-urlencoded", String::new(), token)?;
    if result(&v)["revision"].is_null() {
        return Err("Яндекс не сохранил лайк".into());
    }
    Ok(())
}

/// Дизлайк: Яндекс больше не предложит этот трек.
pub fn set_disliked(uid: &str, track_id: &str, disliked: bool, token: &str) -> Result<(), String> {
    let path = if disliked {
        format!("users/{uid}/dislikes/tracks/add?track-id={track_id}")
    } else {
        format!("users/{uid}/dislikes/tracks/{track_id}/remove")
    };
    let v = post(&path, "application/x-www-form-urlencoded", String::new(), token)?;
    if result(&v)["revision"].is_null() {
        return Err("Яндекс не сохранил дизлайк".into());
    }
    Ok(())
}

pub fn disliked_ids(uid: &str, token: &str) -> Result<Vec<String>, String> {
    let v = get(&format!("users/{uid}/dislikes/tracks"), &[], token)?;
    let tracks = result(&v)["library"]["tracks"].as_array().cloned().unwrap_or_default();
    Ok(tracks
        .iter()
        .filter_map(|t| match &t["id"] {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .collect())
}

fn parse_playlist(v: &Value, uid: &str) -> Option<Playlist> {
    let kind = v["kind"].as_u64()?.to_string();
    let cover = v["cover"]["uri"].as_str().or(v["cover"]["itemsUri"][0].as_str()).or(v["ogImage"].as_str());
    Some(Playlist {
        source: Source::Playlist,
        owner: v["owner"]["uid"].as_u64().map_or_else(|| uid.to_string(), |u| u.to_string()),
        kind,
        title: v["title"].as_str()?.to_string(),
        count: v["trackCount"].as_u64().unwrap_or(0),
        cover: cover.filter(|c| !c.is_empty()).map(str::to_string),
    })
}

pub fn playlists(uid: &str, token: &str) -> Result<Vec<Playlist>, String> {
    let v = get(&format!("users/{uid}/playlists/list"), &[], token)?;
    Ok(v["result"].as_array().into_iter().flatten().filter_map(|p| parse_playlist(p, uid)).collect())
}

fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn parse_pin(item: &Value) -> Option<Pin> {
    let data = &item["data"];
    let list = |source, title: &str, cover: Option<&str>| Playlist {
        source,
        owner: String::new(),
        kind: id_text(&data["id"]).unwrap_or_default(),
        title: title.to_string(),
        count: 0,
        cover: cover.map(str::to_string),
    };
    match item["type"].as_str()? {
        "playlist_item" => {
            let cover = data["cover"]["uri"].as_str().or(data["cover"]["itemsUri"][0].as_str()).map(str::to_string);
            let owner = id_text(&data["uid"])?;
            let title = data["title"].as_str()?.to_string();
            let target = Playlist { source: Source::Playlist, owner, kind: id_text(&data["kind"])?, title: title.clone(), count: 0, cover: cover.clone() };
            Some(Pin { target: PinTarget::List(target), title, subtitle: "Плейлист", cover, round: false })
        }
        "wave_item" => {
            let seeds = data["seeds"].as_array()?.iter().filter_map(|s| s.as_str().map(str::to_string)).collect();
            let cover = data["backgroundImageUrl"].as_str().or(data["agent"]["cover"]["uri"].as_str()).map(str::to_string);
            let title = data["title"].as_str()?.to_string();
            Some(Pin { target: PinTarget::Wave(seeds), title, subtitle: "Моя волна", cover, round: true })
        }
        "album_item" => {
            let title = data["title"].as_str()?;
            let cover = data["coverUri"].as_str();
            Some(Pin { target: PinTarget::List(list(Source::Album, title, cover)), title: title.to_string(), subtitle: "Альбом", cover: cover.map(str::to_string), round: false })
        }
        "artist_item" => {
            let title = data["name"].as_str()?;
            let cover = data["cover"]["uri"].as_str().or(data["coverUri"].as_str());
            Some(Pin { target: PinTarget::List(list(Source::Artist, title, cover)), title: title.to_string(), subtitle: "Исполнитель", cover: cover.map(str::to_string), round: true })
        }
        _ => None,
    }
}

pub fn pins(token: &str) -> Result<Vec<Pin>, String> {
    let v = get("pins", &[], token)?;
    Ok(result(&v)["pins"].as_array().into_iter().flatten().filter_map(parse_pin).collect())
}

pub fn playlist_tracks(playlist: &Playlist, token: &str) -> Result<Vec<Track>, String> {
    match playlist.source {
        Source::Playlist => {}
        Source::Album => {
            let v = get(&format!("albums/{}/with-tracks", playlist.kind), &[], token)?;
            return Ok(result(&v)["volumes"].as_array().into_iter().flatten().flat_map(|disk| disk.as_array().into_iter().flatten()).filter_map(parse_track).collect());
        }
        Source::Artist => {
            let v = get(&format!("artists/{}/tracks", playlist.kind), &[("page-size", "100")], token)?;
            return Ok(result(&v)["tracks"].as_array().into_iter().flatten().filter_map(parse_track).collect());
        }
    }
    let v = get(&format!("users/{}/playlists/{}", playlist.owner, playlist.kind), &[("rich-tracks", "true")], token)?;
    Ok(result(&v)["tracks"].as_array().into_iter().flatten().filter_map(|item| parse_track(&item["track"])).collect())
}

fn parse_sequence(v: &Value) -> Vec<Track> {
    let batch = v["batchId"].as_str().map(str::to_string);
    v["sequence"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| parse_track(&item["track"]))
        .map(|track| Track { batch: batch.clone(), ..track })
        .collect()
}

pub const MY_WAVE: &str = WAVE_SEED;

pub fn wave_new(seeds: &[String], token: &str) -> Result<Wave, String> {
    let body = json!({ "seeds": seeds, "includeTracksInResponse": true, "allowExplicit": true }).to_string();
    let v = post("rotor/session/new", "application/json", body, token)?;
    let v = result(&v);
    let session = v["radioSessionId"].as_str().ok_or("Яндекс не запустил волну")?;
    Ok(Wave { session: session.to_string(), tracks: parse_sequence(v) })
}

/// Следующая пачка волны; `queue` — уже показанные треки.
pub fn wave_more(session: &str, queue: &[String], token: &str) -> Result<Vec<Track>, String> {
    let body = json!({ "queue": queue, "allowExplicit": true }).to_string();
    let v = post(&format!("rotor/session/{session}/tracks"), "application/json", body, token)?;
    Ok(parse_sequence(result(&v)))
}

/// Время в UTC, как у toISOString в оригинале.
fn timestamp(unix_ms: u64) -> String {
    let (secs, ms) = (unix_ms / 1000, unix_ms % 1000);
    // Дни от 1970 в дату: алгоритм civil_from_days.
    let z = (secs / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let rest = secs % 86400;
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z", rest / 3600, rest / 60 % 60, rest % 60)
}

fn now() -> String {
    timestamp(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64))
}

/// Что волна узнаёт о вкусе: запуск, начало, конец, пропуск, лайк.
pub fn wave_feedback(session: &str, kind: &str, track: Option<&Track>, played: Option<f32>, token: &str) -> Result<(), String> {
    let mut event = json!({ "type": kind, "timestamp": now() });
    let mut body = json!({ "from": FROM });
    match track {
        Some(track) => {
            event["trackId"] = json!(track.full_id());
            if let Some(batch) = &track.batch {
                body["batchId"] = json!(batch);
            }
            if let Some(played) = played {
                event["totalPlayedSeconds"] = json!(played);
            }
        }
        None => event["from"] = json!(FROM),
    }
    body["event"] = event;
    post(&format!("rotor/session/{session}/feedback/"), "application/json", body.to_string(), token).map(|_| ())
}

fn sign(data: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SIGN_KEY).expect("ключ HMAC любой длины");
    mac.update(data.as_bytes());
    let mut s = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    // Как в оригинале: подпись без последнего символа.
    s.pop();
    s
}

pub fn file_info(track_id: &str, token: &str) -> Result<FileInfo, String> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs().to_string();
    let signature = sign(&format!("{ts}{track_id}{QUALITY}{}{TRANSPORT}", CODECS.concat()));
    let codecs = CODECS.join(",");
    let v = get(
        "get-file-info",
        &[("ts", &ts), ("trackId", track_id), ("quality", QUALITY), ("codecs", &codecs), ("transports", TRANSPORT), ("sign", &signature)],
        token,
    )?;
    let info = if v["result"].is_object() { &v["result"]["downloadInfo"] } else { &v["downloadInfo"] };
    let url = info["url"].as_str().or(info["urls"][0].as_str()).ok_or("Яндекс не дал файл трека")?;
    Ok(FileInfo {
        url: url.to_string(),
        key: info["key"].as_str().filter(|k| !k.is_empty()).map(str::to_string),
        codec: info["codec"].as_str().unwrap_or("mp3").to_string(),
    })
}

/// Обложка нужного размера, например 100x100. Нет сети — останется заглушка.
pub fn cover(uri: &str, size: &str) -> Option<Vec<u8>> {
    let url = format!("https://{}", uri.replace("%%", size));
    let resp = agent().get(url).call().ok()?;
    if resp.status().as_u16() != 200 {
        return None;
    }
    resp.into_body().read_to_vec().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pins_of_each_type() {
        let v: Value = serde_json::from_str(r#"{"pins":[
            {"type":"playlist_item","data":{"kind":3,"uid":5,"title":"Мне нравится","cover":{"uri":"a/%%"}}},
            {"type":"wave_item","data":{"seeds":["user:onyourwave"],"title":"Моя волна","backgroundImageUrl":"b/%%"}},
            {"type":"album_item","data":{"id":7,"title":"Альбом","coverUri":"c/%%"}},
            {"type":"artist_item","data":{"id":8,"name":"Имя","cover":{"uri":"d/%%"}}},
            {"type":"unknown","data":{}}]}"#).unwrap();
        let pins: Vec<Pin> = v["pins"].as_array().unwrap().iter().filter_map(parse_pin).collect();
        assert_eq!(pins.len(), 4);
        assert!(matches!(&pins[1].target, PinTarget::Wave(seeds) if seeds[0] == "user:onyourwave"));
        assert!(matches!(&pins[2].target, PinTarget::List(list) if list.source == Source::Album && list.kind == "7"));
        assert!(pins[3].round);
    }

    #[test]
    fn playlist_uses_owner_and_cover() {
        let v: Value = serde_json::from_str(r#"{"kind":3,"title":"P","trackCount":2,"owner":{"uid":8},"cover":{"itemsUri":["a/%%"]}}"#).unwrap();
        let p = parse_playlist(&v, "1").unwrap();
        assert_eq!((p.owner.as_str(), p.kind.as_str(), p.count, p.cover.as_deref()), ("8", "3", 2, Some("a/%%")));
    }

    #[test]
    fn timestamp_is_iso() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(timestamp(1_709_210_096_789), "2024-02-29T12:34:56.789Z");
    }

    #[test]
    fn sequence_keeps_batch_and_album() {
        let v: Value = serde_json::from_str(r#"{"batchId":"b","sequence":[{"track":{"id":"7","title":"T","albums":[{"id":9}]}}]}"#).unwrap();
        let tracks = parse_sequence(&v);
        assert_eq!(tracks[0].full_id(), "7:9");
        assert_eq!(tracks[0].batch.as_deref(), Some("b"));
    }

    #[test]
    fn sign_drops_last_char() {
        // HMAC-SHA256 в base64 — 44 символа, в подписи остаётся 43.
        assert_eq!(sign("data").len(), 43);
    }

    #[test]
    fn parses_numeric_and_string_ids() {
        let v: Value = serde_json::from_str(r#"{"id":5,"title":"T","artists":[{"name":"A"},{"name":"B"}],"durationMs":1000,"coverUri":""}"#).unwrap();
        let t = parse_track(&v).unwrap();
        assert_eq!((t.id.as_str(), t.artists.as_str(), t.cover), ("5", "A, B", None));
        let v: Value = serde_json::from_str(r#"{"id":"abc","title":"T"}"#).unwrap();
        assert_eq!(parse_track(&v).unwrap().id, "abc");
    }
}
