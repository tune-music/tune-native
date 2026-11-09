//! Без цензуры — по открытому списку FckCensor (github.com/Hazzz895/FckCensorData), как в
//! Tune Client. Для треков из списка вместо файла Яндекса играет версия без цензуры, у
//! треков и альбомов подменяются название, исполнители, обложка и длительность, удалённые
//! Яндексом треки снова играют и возвращаются в альбомы.
//!
//! Список лежит на GitHub; если он недоступен — на зеркале jsDelivr. Он сохраняется на диск
//! и при запуске читается оттуда сразу, а свежий скачивается в фоне и раз в час.
//!
//! Как в FckCensor, любой трек, альбом или исполнителя можно подменить вручную (окно
//! «Подменить…»): название, исполнители, обложка, свой аудиофайл; автоподмену из списка можно
//! выключить для отдельного id. Свои подмены важнее списка и работают, даже когда «Без
//! цензуры» выключено: это явный выбор пользователя.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

const MIRRORS: [&str; 2] = ["https://raw.githubusercontent.com/Hazzz895/FckCensorData/refs/heads/main/", "https://cdn.jsdelivr.net/gh/Hazzz895/FckCensorData@main/"];
/// Авторы FckCensor дополняют список: перечитываем его раз в час, пока Tune открыт.
const REFRESH: Duration = Duration::from_secs(3600);
/// Не скачался — повтор через десять минут.
const RETRY: Duration = Duration::from_secs(600);
/// Поля, которые не подменяются напрямую: служебные и список треков альбома.
const SKIP: [&str; 5] = ["__fckCensor", "volumes", "id", "audio", "noAuto"];
/// Свой файл трека или обложки лежит на диске: `local:` и путь.
pub const LOCAL: &str = "local:";

/// Что подменяется: ключи в файле своих подмен — как у Tune Client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Track,
    Album,
    Artist,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Kind::Track => "track",
            Kind::Album => "album",
            Kind::Artist => "artist",
        }
    }
}

#[derive(Default)]
struct List {
    /// id трека → ссылка на файл без цензуры.
    files: HashMap<String, String>,
    tracks: HashMap<String, Map<String, Value>>,
    albums: HashMap<String, Map<String, Value>>,
    artists: HashMap<String, Map<String, Value>>,
    /// Зеркало, с которого пришёл список: с него же качаются файлы.
    mirror: usize,
}

static LIST: RwLock<Option<Arc<List>>> = RwLock::new(None);
static ENABLED: AtomicBool = AtomicBool::new(true);
/// Свои подмены: { track | album | artist: { id: поля } }.
static OWN: RwLock<Option<Value>> = RwLock::new(None);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

fn current() -> Option<Arc<List>> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    LIST.read().unwrap().clone()
}

fn own_path() -> std::path::PathBuf {
    crate::api::data_dir().join("spoofs.json")
}

/// Папка своих файлов: аудио и обложки копируются сюда, чтобы не зависеть от исходных.
pub fn own_dir() -> std::path::PathBuf {
    crate::api::data_dir().join("spoof")
}

fn read_own() {
    let own = std::fs::read(own_path()).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()).filter(Value::is_object);
    *OWN.write().unwrap() = Some(own.unwrap_or_else(|| json!({})));
}

/// Своя подмена: поля как у FckCensor (title / name, artists, coverUri) и свои (audio, noAuto).
pub fn own(kind: Kind, id: &str) -> Map<String, Value> {
    OWN.read().unwrap().as_ref().and_then(|own| own[kind.key()][id].as_object().cloned()).unwrap_or_default()
}

/// Сохраняет свою подмену; пустая — убирает её.
pub fn set_own(kind: Kind, id: &str, fields: Map<String, Value>) -> Result<(), String> {
    let mut guard = OWN.write().unwrap();
    let own = guard.get_or_insert_with(|| json!({}));
    let Some(root) = own.as_object_mut() else { return Err("Файл подмен повреждён".into()) };
    let list = root.entry(kind.key()).or_insert_with(|| json!({}));
    let Some(list) = list.as_object_mut() else { return Err("Файл подмен повреждён".into()) };
    if fields.is_empty() {
        list.remove(id);
    } else {
        list.insert(id.to_string(), Value::Object(fields));
    }
    std::fs::create_dir_all(crate::api::data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(own_path(), serde_json::to_string_pretty(own).unwrap_or_default()).map_err(|e| format!("Не удалось сохранить подмену: {e}"))
}

/// Автоподмена из списка для этого id не выключена вручную.
fn auto_on(kind: Kind, id: &str) -> bool {
    own(kind, id).get("noAuto").and_then(Value::as_bool) != Some(true)
}

/// Список, если для этого id автоподмена не выключена.
fn auto(kind: Kind, id: &str) -> Option<Arc<List>> {
    current().filter(|_| auto_on(kind, id))
}

/// Есть ли у id запись в списке FckCensor — тогда в окне подмены есть выключатель автоподмены.
pub fn listed(kind: Kind, id: &str) -> bool {
    current().is_some_and(|list| match kind {
        Kind::Track => list.tracks.contains_key(id) || list.files.contains_key(id),
        Kind::Album => list.albums.contains_key(id),
        Kind::Artist => list.artists.contains_key(id),
    })
}

/// Поля подмены: из списка, поверх — свои.
fn spoof(kind: Kind, id: &str) -> Option<Map<String, Value>> {
    let mut fields = auto(kind, id)
        .and_then(|list| match kind {
            Kind::Track => list.tracks.get(id).cloned(),
            Kind::Album => list.albums.get(id).cloned(),
            Kind::Artist => list.artists.get(id).cloned(),
        })
        .unwrap_or_default();
    fields.extend(own(kind, id));
    (!fields.is_empty()).then_some(fields)
}

/// Файл трека и его формат: свой из окна подмены, иначе версия без цензуры из списка.
pub fn file(id: &str) -> Option<(String, String)> {
    let mine = own(Kind::Track, id);
    if let Some(path) = mine.get("audio").and_then(|audio| audio["file"].as_str()) {
        let codec = mine["audio"]["codec"].as_str().unwrap_or("mp3");
        return Some((format!("{LOCAL}{path}"), codec.to_string()));
    }
    auto_file(id).map(|url| (url, "mp3".to_string()))
}

/// Ссылка на версию трека без цензуры из списка.
fn auto_file(id: &str) -> Option<String> {
    let list = auto(Kind::Track, id)?;
    let url = list.files.get(id)?;
    let full = if url.starts_with("http://") || url.starts_with("https://") { url.clone() } else { format!("https://{url}") };
    Some(match full.strip_prefix(MIRRORS[0]) {
        Some(rest) if list.mirror != 0 => format!("{}{rest}", MIRRORS[list.mirror]),
        _ => full,
    })
}

/// Треки, у которых в списке есть файл: среди них ищутся удалённые Яндексом для волны.
pub fn with_files() -> Vec<String> {
    current().map(|list| list.files.keys().filter(|id| auto_on(Kind::Track, id)).cloned().collect()).unwrap_or_default()
}

/// Треки со своим аудиофайлом из «Подменить…»: удалённые Яндексом тоже ищутся по ним.
pub fn own_audio_ids() -> Vec<String> {
    let guard = OWN.read().unwrap();
    let Some(tracks) = guard.as_ref().and_then(|own| own["track"].as_object()) else { return Vec::new() };
    tracks.iter().filter(|(_, fields)| fields.get("audio").is_some()).map(|(id, _)| id.clone()).collect()
}

/// Включено ли «Без цензуры».
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// У трека есть версия без цензуры: рядом с названием ставится значок.
pub fn has(id: &str) -> bool {
    auto(Kind::Track, id).is_some_and(|list| list.files.contains_key(id))
}

fn id_of(v: &Value) -> Option<String> {
    match &v["id"] {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Поля подмены поверх объекта. Пустой список исполнителей не подменяет настоящий.
fn apply(target: &mut Map<String, Value>, spoof: &Map<String, Value>) {
    for (key, value) in spoof {
        if SKIP.contains(&key.as_str()) || value.is_null() || (key == "artists" && !value.as_array().is_some_and(|list| !list.is_empty())) {
            continue;
        }
        if key == "coverUri" {
            // Обложка альбома в ответе Яндекса бывает и в `cover`: подменяем и её.
            if let Some(Value::Object(cover)) = target.get_mut("cover") {
                cover.insert("uri".into(), value.clone());
                cover.remove("prefix");
            }
            target.remove("derivedColors");
        }
        target.insert(key.clone(), value.clone());
    }
}

/// Трек из ответа Яндекса с подменой; `None` — подменять нечего.
pub fn patch_track(v: &Value) -> Option<Value> {
    let id = id_of(v)?;
    let spoof = spoof(Kind::Track, &id);
    let file = file(&id).is_some();
    if spoof.is_none() && !file {
        return None;
    }
    let mut out = v.as_object()?.clone();
    if let Some(spoof) = &spoof {
        apply(&mut out, spoof);
    }
    // Удалённый или закрытый в Яндексе трек с файлом снова играет.
    if file {
        out.insert("available".into(), Value::Bool(true));
        out.remove("error");
    }
    Some(Value::Object(out))
}

/// Альбом из ответа Яндекса с подменой названия, исполнителей и обложки.
pub fn patch_album(v: &Value) -> Option<Value> {
    patch(Kind::Album, v)
}

/// Исполнитель из ответа Яндекса с подменой имени и обложки.
pub fn patch_artist(v: &Value) -> Option<Value> {
    patch(Kind::Artist, v)
}

fn patch(kind: Kind, v: &Value) -> Option<Value> {
    let spoof = spoof(kind, &id_of(v)?)?;
    let mut out = v.as_object()?.clone();
    apply(&mut out, &spoof);
    Some(Value::Object(out))
}

/// Треки альбома по дискам, как в списке: туда возвращаются удалённые Яндексом треки.
pub fn album_volumes(id: &str) -> Option<Vec<Vec<String>>> {
    let list = auto(Kind::Album, id)?;
    let volumes = list.albums.get(id)?.get("volumes")?.as_array()?;
    let volumes: Vec<Vec<String>> = volumes.iter().map(|disk| disk.as_array().into_iter().flatten().filter_map(|track| if track.is_object() { id_of(track) } else { id_of(&json!({ "id": track })) }).filter(|id| is_id(id)).collect()).collect();
    volumes.iter().any(|disk| !disk.is_empty()).then_some(volumes)
}

fn is_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

/// Формат list_v2: в `sources` — подмены полей (tracks, albums) и хранилища файлов
/// (tracksStorages: trackIds по шаблону ссылки или tracks: id → номер файла / ссылка). Как
/// у FckCensor: поля — из первого источника, где они есть, файлы — из последнего.
fn parse(list: &Value, mirror: usize) -> List {
    let mut out = List { mirror, ..List::default() };
    for source in list["sources"].as_array().into_iter().flatten() {
        for (kind, target) in [("tracks", &mut out.tracks), ("albums", &mut out.albums), ("artists", &mut out.artists)] {
            for (id, fields) in source[kind].as_object().into_iter().flatten() {
                if let (true, Some(fields)) = (is_id(id), fields.as_object()) {
                    target.entry(id.clone()).or_insert_with(|| fields.clone());
                }
            }
        }
        for storage in source["tracksStorages"].as_array().into_iter().flatten() {
            let template = storage["urlTemplate"].as_str().unwrap_or_default();
            let mut duration = |id: &str, entry: &Value| {
                if let Some(ms) = entry["durationMs"].as_u64() {
                    out.tracks.entry(id.to_string()).or_default().insert("durationMs".into(), ms.into());
                }
            };
            if !template.is_empty() {
                for entry in storage["trackIds"].as_array().into_iter().flatten() {
                    let Some(id) = (if entry.is_object() { id_of(entry) } else { id_of(&json!({ "id": entry })) }).filter(|id| is_id(id)) else { continue };
                    duration(&id, entry);
                    out.files.insert(id.clone(), template.replace("%%", &id));
                }
            }
            for (id, entry) in storage["tracks"].as_object().into_iter().flatten() {
                if !is_id(id) {
                    continue;
                }
                let raw = if entry.is_object() { &entry["url"] } else { entry };
                let url = match raw {
                    Value::Number(n) if !template.is_empty() => template.replace("%%", &n.to_string()),
                    Value::String(s) if !s.contains("://") && !template.is_empty() => template.replace("%%", s),
                    Value::String(s) => s.clone(),
                    _ => continue,
                };
                duration(id, entry);
                if !url.is_empty() {
                    out.files.insert(id.clone(), url);
                }
            }
        }
    }
    // Треки подменённого альбома без своей обложки берут обложку альбома.
    let covers: Vec<(String, Value)> = out
        .albums
        .iter()
        .filter_map(|(id, album)| Some((id, album.get("coverUri")?.clone())))
        .flat_map(|(id, cover)| album_ids(&out.albums[id]).into_iter().map(move |track| (track, cover.clone())))
        .collect();
    for (track, cover) in covers {
        out.tracks.entry(track).or_default().entry("coverUri").or_insert(cover);
    }
    out
}

fn album_ids(album: &Map<String, Value>) -> Vec<String> {
    album.get("volumes").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_array).flatten().filter_map(|track| if track.is_object() { id_of(track) } else { id_of(&json!({ "id": track })) }).collect()
}

fn cache_path() -> std::path::PathBuf {
    crate::api::data_dir().join("uncensor.json")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Список с диска: сразу, ещё до первого ответа Яндекса. Вернёт, когда он скачан.
fn read_cache() -> Option<(u64, usize)> {
    let cached: Value = serde_json::from_slice(&std::fs::read(cache_path()).ok()?).ok()?;
    let mirror = cached["mirror"].as_u64().map_or(0, |m| m as usize).min(MIRRORS.len() - 1);
    *LIST.write().unwrap() = Some(Arc::new(parse(&cached["list"], mirror)));
    Some((cached["at"].as_u64().unwrap_or(0), mirror))
}

fn fetch(first: usize) -> Result<usize, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).build().into();
    let mut error = String::new();
    for mirror in std::iter::once(first).chain(0..MIRRORS.len()) {
        let url = format!("{}list_v2.json", MIRRORS[mirror]);
        let body = agent.get(&url).call().map_err(|e| e.to_string()).and_then(|response| response.into_body().with_config().limit(64 * 1024 * 1024).read_to_string().map_err(|e| e.to_string()));
        match body.and_then(|body| serde_json::from_str::<Value>(&body).map_err(|e| e.to_string())) {
            Ok(list) => {
                *LIST.write().unwrap() = Some(Arc::new(parse(&list, mirror)));
                let cached = json!({ "at": now(), "mirror": mirror, "list": list });
                let _ = std::fs::write(cache_path(), cached.to_string());
                return Ok(mirror);
            }
            Err(e) => error = format!("{url}: {e}"),
        }
    }
    Err(error)
}

/// Читает список с диска и запускает обновление в фоне. Вызывается один раз при запуске.
pub fn start() {
    read_own();
    let (at, mut mirror) = read_cache().unwrap_or((0, 0));
    std::thread::spawn(move || {
        let mut wait = Duration::from_secs(REFRESH.as_secs().saturating_sub(now().saturating_sub(at)));
        loop {
            std::thread::sleep(wait);
            wait = match fetch(mirror) {
                Ok(used) => {
                    mirror = used;
                    REFRESH
                }
                Err(e) => {
                    crate::api::log(&format!("Список FckCensor не скачался: {e}"));
                    RETRY
                }
            };
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fields_and_files() {
        let list = json!({ "sources": [
            { "tracks": { "1": { "title": "Без цензуры", "artists": [] } }, "albums": { "7": { "title": "Альбом", "coverUri": "c/%%", "volumes": [[1, { "id": 2 }]] } } },
            { "tracks": { "1": { "title": "Второй источник" } }, "tracksStorages": [
                { "urlTemplate": "https://example.org/%%.mp3", "trackIds": [1, { "id": 3, "durationMs": 1000 }] },
                { "urlTemplate": "https://example.org/f/%%.mp3", "tracks": { "2": 55, "4": "https://other.org/4.mp3" } }
            ] }
        ] });
        let parsed = parse(&list, 0);
        assert_eq!(parsed.tracks["1"]["title"], "Без цензуры");
        assert_eq!(parsed.files["1"], "https://example.org/1.mp3");
        assert_eq!(parsed.files["2"], "https://example.org/f/55.mp3");
        assert_eq!(parsed.files["4"], "https://other.org/4.mp3");
        assert_eq!(parsed.tracks["3"]["durationMs"], 1000);
        assert_eq!(parsed.tracks["2"]["coverUri"], "c/%%");
        let mut track = json!({ "id": 1, "title": "С цензурой", "artists": [{ "id": 5, "name": "Кто-то" }], "available": false }).as_object().unwrap().clone();
        apply(&mut track, &parsed.tracks["1"]);
        assert_eq!((track["title"].as_str(), track["artists"][0]["name"].as_str()), (Some("Без цензуры"), Some("Кто-то")));
    }
}
