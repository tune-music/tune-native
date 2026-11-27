//! Без цензуры и дополнительный каталог Tune (`tune-music/tune-catalog-data`).
//!
//! Для треков из списка FckCensorData вместо файла Яндекса играет версия без цензуры, у
//! треков и альбомов подменяются название, исполнители, обложка и длительность, удалённые
//! Яндексом треки снова играют и возвращаются в альбомы.
//!
//! Дополнительно подтягивается `list_v2.json` из `tune-music/tune-catalog-data` — новые треки,
//! альбомы, исполнители и плейлисты доступны в поиске и открываются прямо в Tune Native.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

const MIRRORS: [&str; 2] = [
    "https://raw.githubusercontent.com/Hazzz895/FckCensorData/refs/heads/main/",
    "https://cdn.jsdelivr.net/gh/Hazzz895/FckCensorData@main/",
];
const CATALOG_MIRRORS: [&str; 2] = [
    "https://raw.githubusercontent.com/tune-music/tune-catalog-data/refs/heads/main/",
    "https://cdn.jsdelivr.net/gh/tune-music/tune-catalog-data@main/",
];

pub const CATALOG_OWNER: &str = "tune-catalog";

/// Авторы FckCensor дополняют список: перечитываем его раз в час, пока Tune открыт.
const REFRESH: Duration = Duration::from_secs(3600);
/// Каталог Tune обновляется каждые 2 минуты (и сразу при запуске), чтобы новые загрузки появлялись сразу.
const CATALOG_REFRESH: Duration = Duration::from_secs(120);
/// Не скачался — повтор через десять минут (для каталога Tune — через минуту).
const RETRY: Duration = Duration::from_secs(600);
const CATALOG_RETRY: Duration = Duration::from_secs(60);
/// Поля, которые не подменяются напрямую: служебные и список треков альбома.
const SKIP: [&str; 6] = ["__fckCensor", "volumes", "id", "audio", "noAuto", "insert"];
/// Свой файл трека или обложки лежит на диске: `local:` и путь.
pub const LOCAL: &str = "local:";

const SYNTHETIC_TRACK_BASE: u64 = 9_000_000_000;
const SYNTHETIC_ALBUM_BASE: u64 = 900_000_000;
const SYNTHETIC_ARTIST_BASE: u64 = 90_000_000;
const SYNTHETIC_PLAYLIST_BASE: u64 = 9_000_000;
const SYNC_ID_MAX_DELTA: u64 = 999_999_999;
const SYNC_EPOCH_OFFSET: u64 = 1_700_000_000;
const SYNC_TOMBSTONE_TTL_SECS: u64 = 2_592_000;
pub const SYNC_DESCRIPTION_MAX_CHARS: usize = 490;
const SYNC_TAG_PREFIX: &str = "[tune-sync:v1:";

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
    /// id трека → ссылка на аудиофайл.
    files: HashMap<String, String>,
    tracks: HashMap<String, Map<String, Value>>,
    albums: HashMap<String, Map<String, Value>>,
    artists: HashMap<String, Map<String, Value>>,
    playlists: HashMap<String, Map<String, Value>>,
    /// Исполнитель → треки и альбомы, добавленные в его профиль (`artistsInsertions`).
    inserts: HashMap<String, (Vec<String>, Vec<String>)>,
    /// Трек → `(album_id, album_title)` из `albums[*].volumes`.
    track_albums: HashMap<String, (String, String)>,
    /// Зеркало FckCensor, с которого пришёл список: с него же качаются файлы.
    mirror: usize,
}

static FCK_RAW: RwLock<Option<(Value, usize)>> = RwLock::new(None);
static CATALOG_RAW: RwLock<Option<(Value, usize)>> = RwLock::new(None);
static LIST_MERGED: RwLock<Option<Arc<List>>> = RwLock::new(None);
static LIST_FCK: RwLock<Option<Arc<List>>> = RwLock::new(None);
static LIST_CATALOG: RwLock<Option<Arc<List>>> = RwLock::new(None);
static ENABLED: AtomicBool = AtomicBool::new(true);
static CATALOG_ENABLED: AtomicBool = AtomicBool::new(true);
static VERSION: AtomicU64 = AtomicU64::new(0);
/// Свои подмены: { track | album | artist: { id: поля } }.
static OWN: RwLock<Option<Value>> = RwLock::new(None);
static LOCAL_LIB: RwLock<Option<LocalLibrary>> = RwLock::new(None);
static REMOTE_SYNC_GEN: AtomicU64 = AtomicU64::new(0);
static LAST_LOCAL_TS: AtomicU64 = AtomicU64::new(0);

pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    VERSION.fetch_add(1, Ordering::Relaxed);
}

pub fn set_catalog_enabled(on: bool) {
    CATALOG_ENABLED.store(on, Ordering::Relaxed);
    VERSION.fetch_add(1, Ordering::Relaxed);
}

pub fn catalog_enabled() -> bool {
    CATALOG_ENABLED.load(Ordering::Relaxed)
}

fn current() -> Option<Arc<List>> {
    match (
        ENABLED.load(Ordering::Relaxed),
        CATALOG_ENABLED.load(Ordering::Relaxed),
    ) {
        (true, true) => LIST_MERGED.read().unwrap().clone(),
        (true, false) => LIST_FCK.read().unwrap().clone(),
        (false, true) => LIST_CATALOG.read().unwrap().clone(),
        (false, false) => None,
    }
}

fn catalog_only() -> Option<Arc<List>> {
    if !CATALOG_ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    LIST_CATALOG.read().unwrap().clone()
}

pub fn is_synthetic_track_id(id: &str) -> bool {
    id.parse::<u64>().is_ok_and(|n| n > SYNTHETIC_TRACK_BASE)
}

pub fn is_synthetic_album_id(id: &str) -> bool {
    id.parse::<u64>()
        .is_ok_and(|n| n > SYNTHETIC_ALBUM_BASE && n < SYNTHETIC_TRACK_BASE)
}

pub fn is_synthetic_artist_id(id: &str) -> bool {
    id.parse::<u64>()
        .is_ok_and(|n| n > SYNTHETIC_ARTIST_BASE && n < SYNTHETIC_ALBUM_BASE)
}

fn rebuild_lists() {
    let fck = FCK_RAW.read().unwrap().clone();
    let cat = CATALOG_RAW.read().unwrap().clone();

    let fck_list = fck.as_ref().map(|(v, m)| Arc::new(parse_sources(&[v], *m)));
    *LIST_FCK.write().unwrap() = fck_list;

    let catalog_list = cat
        .as_ref()
        .map(|(v, m)| Arc::new(parse_sources(&[v], *m)));
    *LIST_CATALOG.write().unwrap() = catalog_list;

    let merged = match (cat.as_ref(), fck.as_ref()) {
        (Some((cv, _)), Some((fv, fm))) => Some(Arc::new(parse_merged(cv, fv, *fm))),
        (Some((cv, cm)), None) => Some(Arc::new(parse_sources(&[cv], *cm))),
        (None, Some((fv, fm))) => Some(Arc::new(parse_sources(&[fv], *fm))),
        (None, None) => None,
    };
    *LIST_MERGED.write().unwrap() = merged;
    VERSION.fetch_add(1, Ordering::Relaxed);
}

fn own_path() -> std::path::PathBuf {
    crate::api::data_dir().join("spoofs.json")
}

/// Папка своих файлов: аудио и обложки копируются сюда, чтобы не зависеть от исходных.
pub fn own_dir() -> std::path::PathBuf {
    crate::api::data_dir().join("spoof")
}

fn read_own() {
    let own = std::fs::read(own_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(Value::is_object);
    *OWN.write().unwrap() = Some(own.unwrap_or_else(|| json!({})));
}

/// Своя подмена: поля как у FckCensor (title / name, artists, coverUri) и свои (audio, noAuto).
pub fn own(kind: Kind, id: &str) -> Map<String, Value> {
    OWN.read()
        .unwrap()
        .as_ref()
        .and_then(|own| own[kind.key()][id].as_object().cloned())
        .unwrap_or_default()
}

/// Сохраняет свою подмену; пустая — убирает её.
pub fn set_own(kind: Kind, id: &str, fields: Map<String, Value>) -> Result<(), String> {
    let mut guard = OWN.write().unwrap();
    let own = guard.get_or_insert_with(|| json!({}));
    let Some(root) = own.as_object_mut() else {
        return Err("Файл подмен повреждён".into());
    };
    let list = root.entry(kind.key()).or_insert_with(|| json!({}));
    let Some(list) = list.as_object_mut() else {
        return Err("Файл подмен повреждён".into());
    };
    if fields.is_empty() {
        list.remove(id);
    } else {
        list.insert(id.to_string(), Value::Object(fields));
    }
    std::fs::create_dir_all(crate::api::data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(own_path(), serde_json::to_string_pretty(own).unwrap_or_default())
        .map_err(|e| format!("Не удалось сохранить подмену: {e}"))
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
        Kind::Artist => list.artists.contains_key(id) || list.inserts.contains_key(id),
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

/// Файл трека и его формат: свой из окна подмены, иначе версия без цензуры / из каталога Tune.
pub fn file(id: &str) -> Option<(String, String)> {
    let mine = own(Kind::Track, id);
    if let Some(path) = mine.get("audio").and_then(|audio| audio["file"].as_str()) {
        let codec = mine["audio"]["codec"].as_str().unwrap_or("mp3");
        return Some((format!("{LOCAL}{path}"), codec.to_string()));
    }
    auto_file(id).map(|url| (url, "mp3".to_string()))
}

/// Ссылка на аудиофайл трека из списка.
fn auto_file(id: &str) -> Option<String> {
    let list = auto(Kind::Track, id)?;
    let url = list.files.get(id)?;
    let full = if url.starts_with("http://") || url.starts_with("https://") {
        url.clone()
    } else {
        format!("https://{url}")
    };
    Some(match full.strip_prefix(MIRRORS[0]) {
        Some(rest) if list.mirror != 0 => format!("{}{rest}", MIRRORS[list.mirror]),
        _ => full,
    })
}

/// Треки, у которых в списке есть файл: среди них ищутся удалённые Яндексом для волны.
pub fn with_files() -> Vec<String> {
    current()
        .map(|list| {
            list.files
                .keys()
                .filter(|id| !is_synthetic_track_id(id) && auto_on(Kind::Track, id))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Треки со своим аудиофайлом из «Подменить…»: удалённые Яндексом тоже ищутся по ним.
pub fn own_audio_ids() -> Vec<String> {
    let guard = OWN.read().unwrap();
    let Some(tracks) = guard.as_ref().and_then(|own| own["track"].as_object()) else {
        return Vec::new();
    };
    tracks
        .iter()
        .filter(|(_, fields)| fields.get("audio").is_some())
        .map(|(id, _)| id.clone())
        .collect()
}

/// Включено ли «Без цензуры».
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Трек взят из дополнительного каталога Tune: рядом с названием ставится значок «T».
pub fn is_catalog_track(id: &str) -> bool {
    CATALOG_ENABLED.load(Ordering::Relaxed)
        && auto_on(Kind::Track, id)
        && LIST_CATALOG
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|list| list.files.contains_key(id))
}

/// У трека есть версия без цензуры из FckCensor: рядом с названием ставится значок замка.
pub fn has(id: &str) -> bool {
    ENABLED.load(Ordering::Relaxed)
        && !is_catalog_track(id)
        && auto_on(Kind::Track, id)
        && LIST_FCK
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|list| list.files.contains_key(id))
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
        if SKIP.contains(&key.as_str())
            || value.is_null()
            || (key == "artists" && !value.as_array().is_some_and(|list| !list.is_empty()))
        {
            continue;
        }
        if key == "coverUri" {
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

/// Треки альбома по дискам: свой список из «Подменить…», иначе из списка — туда
/// возвращаются удалённые Яндексом треки или треки альбомов каталога Tune.
pub fn album_volumes(id: &str) -> Option<Vec<Vec<String>>> {
    if let Some(volumes) = own(Kind::Album, id).get("volumes") {
        return volumes_of(volumes);
    }
    let list = auto(Kind::Album, id)?;
    volumes_of(list.albums.get(id)?.get("volumes")?)
}

/// Треки альбома из списка FckCensor, без своих правок.
pub fn listed_volumes(id: &str) -> Option<Vec<Vec<String>>> {
    volumes_of(current()?.albums.get(id)?.get("volumes")?)
}

fn volumes_of(volumes: &Value) -> Option<Vec<Vec<String>>> {
    let volumes: Vec<Vec<String>> = volumes
        .as_array()?
        .iter()
        .map(|disk| {
            disk.as_array()
                .into_iter()
                .flatten()
                .filter_map(|track| {
                    if track.is_object() {
                        id_of(track)
                    } else {
                        id_of(&json!({ "id": track }))
                    }
                })
                .filter(|id| is_id(id))
                .collect()
        })
        .filter(|disk: &Vec<String>| !disk.is_empty())
        .collect();
    (!volumes.is_empty()).then_some(volumes)
}

/// Треки и альбомы, добавленные в профиль исполнителя: свои из «Подменить…», иначе из списка.
pub fn artist_inserts(id: &str) -> (Vec<String>, Vec<String>) {
    if let Some(insert) = own(Kind::Artist, id).get("insert") {
        return (id_list(&insert["tracks"]), id_list(&insert["albums"]));
    }
    auto(Kind::Artist, id)
        .and_then(|list| list.inserts.get(id).cloned())
        .unwrap_or_default()
}

/// Вставки исполнителя из списка FckCensor, без своих правок.
pub fn listed_inserts(id: &str) -> (Vec<String>, Vec<String>) {
    current()
        .and_then(|list| list.inserts.get(id).cloned())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Синтетические сущности и поиск по каталогу Tune
// ---------------------------------------------------------------------------

fn build_catalog_track_value(id: &str, list: &List) -> Option<Value> {
    let fields = list.tracks.get(id)?;
    let title = fields.get("title").and_then(Value::as_str)?;
    if title.trim().is_empty() {
        return None;
    }
    let mut obj = Map::new();
    obj.insert("id".into(), json!(id));
    obj.insert("available".into(), json!(true));
    apply(&mut obj, fields);
    if !obj.contains_key("albums") {
        if let Some((album_id, album_title)) = list.track_albums.get(id) {
            let album_num = album_id
                .parse::<u64>()
                .map_or_else(|_| json!(album_id), |n| json!(n));
            obj.insert(
                "albums".into(),
                json!([{ "id": album_num, "title": album_title }]),
            );
        }
    }
    Some(Value::Object(obj))
}

pub fn catalog_track_json(id: &str) -> Option<Value> {
    let list = current().or_else(catalog_only)?;
    if !list.files.contains_key(id) && !list.tracks.contains_key(id) {
        return None;
    }
    build_catalog_track_value(id, &list)
}

pub fn catalog_album_json(id: &str) -> Option<Value> {
    let list = current().or_else(catalog_only)?;
    let fields = list.albums.get(id)?;
    let mut obj = fields.clone();
    obj.insert("id".into(), json!(id));
    Some(Value::Object(obj))
}

pub fn catalog_artist_json(id: &str) -> Option<(Value, Vec<String>, Vec<String>)> {
    let list = current().or_else(catalog_only)?;
    let fields = list.artists.get(id)?;
    let mut obj = fields.clone();
    obj.insert("id".into(), json!(id));

    let mut seen_tracks = HashSet::new();
    let mut track_ids = Vec::new();
    let mut seen_albums = HashSet::new();
    let mut album_ids = Vec::new();

    if let Some((ins_tracks, ins_albums)) = list.inserts.get(id) {
        for tid in ins_tracks {
            if seen_tracks.insert(tid.clone()) {
                track_ids.push(tid.clone());
            }
        }
        for aid in ins_albums {
            if seen_albums.insert(aid.clone()) {
                album_ids.push(aid.clone());
            }
        }
    }

    let has_artist = |obj: &Map<String, Value>| -> bool {
        obj.get("artists")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().any(|a| id_of(a).as_deref() == Some(id)))
    };

    for (tid, t_fields) in &list.tracks {
        if has_artist(t_fields) && seen_tracks.insert(tid.clone()) {
            track_ids.push(tid.clone());
        }
    }
    for (aid, a_fields) in &list.albums {
        if has_artist(a_fields) && seen_albums.insert(aid.clone()) {
            album_ids.push(aid.clone());
        }
    }

    Some((Value::Object(obj), track_ids, album_ids))
}

pub fn catalog_playlist(kind: &str) -> Option<(Map<String, Value>, Vec<String>)> {
    let list = current().or_else(catalog_only)?;
    let fields = list.playlists.get(kind)?;
    let track_ids = id_list(fields.get("tracks").unwrap_or(&Value::Null));
    Some((fields.clone(), track_ids))
}

fn search_tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('ё', "е")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn matches_query(query: &str, fields: &[&str]) -> bool {
    let terms = search_tokens(query);
    if terms.is_empty() {
        return false;
    }
    let allow_prefix = terms.iter().map(String::len).sum::<usize>() >= 2;
    let hay: Vec<String> = fields.iter().flat_map(|s| search_tokens(s)).collect();
    terms.iter().all(|term| {
        if allow_prefix {
            hay.iter().any(|t| t.starts_with(term))
        } else {
            hay.iter().any(|t| t == term)
        }
    })
}

pub fn search_tracks(query: &str) -> Vec<Value> {
    let Some(list) = catalog_only() else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for id in list.files.keys() {
        let Some(val) = build_catalog_track_value(id, &list) else {
            continue;
        };
        if catalog_track_matches_val(&val, query) {
            hits.push(val);
        }
    }
    hits.sort_by(|a, b| {
        let ta = a["title"].as_str().unwrap_or("");
        let tb = b["title"].as_str().unwrap_or("");
        ta.cmp(tb)
    });
    hits.truncate(25);
    hits
}

fn catalog_track_matches_val(val: &Value, query: &str) -> bool {
    let title = val["title"].as_str().unwrap_or("");
    let artists = val["artists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["name"].as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let album = val["albums"][0]["title"].as_str().unwrap_or("");
    matches_query(query, &[title, &artists, album])
}

pub fn track_matches_query(id: &str, query: &str) -> bool {
    let Some(list) = catalog_only() else {
        return false;
    };
    build_catalog_track_value(id, &list)
        .is_some_and(|val| catalog_track_matches_val(&val, query))
}

pub fn search_albums(query: &str) -> Vec<Value> {
    let Some(list) = catalog_only() else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for (id, fields) in &list.albums {
        let title = fields.get("title").and_then(Value::as_str).unwrap_or("");
        if title.is_empty() {
            continue;
        }
        let artists = fields
            .get("artists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| a["name"].as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if matches_query(query, &[title, &artists]) {
            let mut obj = fields.clone();
            obj.insert("id".into(), json!(id));
            hits.push(Value::Object(obj));
        }
    }
    hits.truncate(15);
    hits
}

pub fn search_artists(query: &str) -> Vec<Value> {
    let Some(list) = catalog_only() else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for (id, fields) in &list.artists {
        let name = fields.get("name").and_then(Value::as_str).unwrap_or("");
        if !name.is_empty() && matches_query(query, &[name]) {
            let mut obj = fields.clone();
            obj.insert("id".into(), json!(id));
            hits.push(Value::Object(obj));
        }
    }
    hits.truncate(15);
    hits
}

pub fn search_playlists(query: &str) -> Vec<(String, Map<String, Value>)> {
    let Some(list) = catalog_only() else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for (id, fields) in &list.playlists {
        let title = fields.get("title").and_then(Value::as_str).unwrap_or("");
        let desc = fields.get("description").and_then(Value::as_str).unwrap_or("");
        if !title.is_empty() && matches_query(query, &[title, desc]) {
            hits.push((id.clone(), fields.clone()));
        }
    }
    hits.truncate(15);
    hits
}

/// Номера из списка: числа, строки или `{ releaseId | id }`, как в `artistsInsertions`.
fn id_list(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = if entry.is_object() {
                if entry["releaseId"].is_null() {
                    &entry["id"]
                } else {
                    &entry["releaseId"]
                }
            } else {
                entry
            };
            match entry {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }
        })
        .filter(|id| is_id(id))
        .collect()
}

fn is_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

fn parse_metadata_into(list: &Value, out: &mut List) {
    for source in list["sources"].as_array().into_iter().flatten() {
        for (kind, target) in [
            ("tracks", &mut out.tracks),
            ("albums", &mut out.albums),
            ("artists", &mut out.artists),
            ("playlists", &mut out.playlists),
        ] {
            for (id, fields) in source[kind].as_object().into_iter().flatten() {
                if let (true, Some(fields)) = (is_id(id), fields.as_object()) {
                    target.entry(id.clone()).or_insert_with(|| fields.clone());
                }
            }
        }
        for (id, insert) in source["artistsInsertions"].as_object().into_iter().flatten() {
            if is_id(id) && insert.is_object() {
                let entry = out.inserts.entry(id.clone()).or_default();
                for tid in id_list(&insert["tracks"]) {
                    if !entry.0.contains(&tid) {
                        entry.0.push(tid);
                    }
                }
                for aid in id_list(&insert["albums"]) {
                    if !entry.1.contains(&aid) {
                        entry.1.push(aid);
                    }
                }
            }
        }
    }
}

fn parse_storages_into(list: &Value, out: &mut List) {
    for source in list["sources"].as_array().into_iter().flatten() {
        for storage in source["tracksStorages"].as_array().into_iter().flatten() {
            let template = storage["urlTemplate"].as_str().unwrap_or_default();
            let mut duration = |id: &str, entry: &Value| {
                if let Some(ms) = entry["durationMs"].as_u64() {
                    out.tracks
                        .entry(id.to_string())
                        .or_default()
                        .insert("durationMs".into(), ms.into());
                }
            };
            if !template.is_empty() {
                for entry in storage["trackIds"].as_array().into_iter().flatten() {
                    let Some(id) = (if entry.is_object() {
                        id_of(entry)
                    } else {
                        id_of(&json!({ "id": entry }))
                    })
                    .filter(|id| is_id(id)) else {
                        continue;
                    };
                    duration(&id, entry);
                    out.files.insert(id.clone(), template.replace("%%", &id));
                }
            }
            for (id, entry) in storage["tracks"].as_object().into_iter().flatten() {
                if !is_id(id) {
                    continue;
                }
                let raw = if entry.is_object() {
                    &entry["url"]
                } else {
                    entry
                };
                let url = match raw {
                    Value::Number(n) if !template.is_empty() => template.replace("%%", &n.to_string()),
                    Value::String(s) if !s.contains("://") && !template.is_empty() => {
                        template.replace("%%", s)
                    }
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
}

fn finalize_album_links(out: &mut List) {
    let mut covers: Vec<(String, Value)> = Vec::new();
    let mut links: Vec<(String, String, String)> = Vec::new();
    for (album_id, album) in &out.albums {
        let title = album
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let cover = album.get("coverUri").cloned();
        for tid in album_ids(album) {
            if let Some(ref c) = cover {
                covers.push((tid.clone(), c.clone()));
            }
            if !title.is_empty() {
                links.push((tid, album_id.clone(), title.clone()));
            }
        }
    }
    for (track, cover) in covers {
        out.tracks
            .entry(track)
            .or_default()
            .entry("coverUri")
            .or_insert(cover);
    }
    for (track, album_id, album_title) in links {
        out.track_albums
            .entry(track)
            .or_insert((album_id, album_title));
    }
}

fn parse_sources(lists: &[&Value], mirror: usize) -> List {
    let mut out = List {
        mirror,
        ..List::default()
    };
    for list in lists {
        parse_metadata_into(list, &mut out);
    }
    for list in lists {
        parse_storages_into(list, &mut out);
    }
    finalize_album_links(&mut out);
    out
}

fn parse_merged(catalog_list: &Value, fck_list: &Value, fck_mirror: usize) -> List {
    let mut out = List {
        mirror: fck_mirror,
        ..List::default()
    };
    // Метаданные каталога Tune имеют приоритет над FckCensorData;
    // аудиопотоки — наоборот: Tune Catalog пишется вторым, перезаписывая FckCensorData.
    parse_metadata_into(catalog_list, &mut out);
    parse_metadata_into(fck_list, &mut out);
    parse_storages_into(fck_list, &mut out);
    parse_storages_into(catalog_list, &mut out);
    finalize_album_links(&mut out);
    out
}

fn album_ids(album: &Map<String, Value>) -> Vec<String> {
    album
        .get("volumes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|track| {
            if track.is_object() {
                id_of(track)
            } else {
                id_of(&json!({ "id": track }))
            }
        })
        .collect()
}

fn cache_path() -> std::path::PathBuf {
    crate::api::data_dir().join("uncensor.json")
}

fn catalog_cache_path() -> std::path::PathBuf {
    crate::api::data_dir().join("tune-catalog.json")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn read_cache_into(
    path: std::path::PathBuf,
    target: &RwLock<Option<(Value, usize)>>,
    mirrors_len: usize,
) -> Option<(u64, usize)> {
    let cached: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let mirror = cached["mirror"]
        .as_u64()
        .map_or(0, |m| m as usize)
        .min(mirrors_len.saturating_sub(1));
    *target.write().unwrap() = Some((cached["list"].clone(), mirror));
    Some((cached["at"].as_u64().unwrap_or(0), mirror))
}

fn fetch_from(
    mirrors: &[&str],
    first: usize,
    target: &RwLock<Option<(Value, usize)>>,
    cache: std::path::PathBuf,
    bust_cache: bool,
) -> Result<usize, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut error = String::new();
    for mirror in std::iter::once(first).chain(0..mirrors.len()) {
        let base_url = format!("{}list_v2.json", mirrors[mirror]);
        let url = if bust_cache && base_url.starts_with("https://raw.githubusercontent.com/") {
            format!("{base_url}?t={}", now())
        } else {
            base_url
        };
        let mut req = agent.get(&url);
        if bust_cache {
            req = req.header("Cache-Control", "no-cache");
        }
        let body = req
            .call()
            .map_err(|e| e.to_string())
            .and_then(|response| {
                response
                    .into_body()
                    .with_config()
                    .limit(64 * 1024 * 1024)
                    .read_to_string()
                    .map_err(|e| e.to_string())
            });
        match body.and_then(|body| serde_json::from_str::<Value>(&body).map_err(|e| e.to_string())) {
            Ok(list) => {
                *target.write().unwrap() = Some((list.clone(), mirror));
                rebuild_lists();
                let cached = json!({ "at": now(), "mirror": mirror, "list": list });
                let _ = std::fs::write(&cache, cached.to_string());
                return Ok(mirror);
            }
            Err(e) => error = format!("{url}: {e}"),
        }
    }
    Err(error)
}

// ---------------------------------------------------------------------------
// Локальная медиатека для синтетических треков/альбомов/артистов/плейлистов каталога Tune
// и кросс-девайсная синхронизация (`tune-sync:v1`)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct LwwEntry {
    active: bool,
    ts: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PlaylistLwwEntry {
    tracks: Vec<String>,
    ts: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct LocalLibrary {
    liked_tracks: Vec<String>,
    disliked_tracks: Vec<String>,
    liked_albums: Vec<String>,
    liked_artists: Vec<String>,
    disliked_artists: Vec<String>,
    liked_playlists: Vec<String>,
    playlist_tracks: HashMap<String, Vec<String>>,
    pins: Vec<String>,
    liked_tracks_lww: HashMap<String, LwwEntry>,
    disliked_tracks_lww: HashMap<String, LwwEntry>,
    liked_albums_lww: HashMap<String, LwwEntry>,
    liked_artists_lww: HashMap<String, LwwEntry>,
    disliked_artists_lww: HashMap<String, LwwEntry>,
    liked_playlists_lww: HashMap<String, LwwEntry>,
    playlist_tracks_lww: HashMap<String, PlaylistLwwEntry>,
    pins_lww: HashMap<String, LwwEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncShardInfo {
    pub kind: String,
    pub title: String,
    pub description: String,
    pub has_custom_cover: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedShards {
    Empty,
    Complete {
        primary_payload: String,
        extra_payloads: Vec<String>,
        canonical_kinds: HashMap<usize, String>,
        canonical_titles: HashMap<usize, String>,
        canonical_descriptions: HashMap<usize, String>,
        duplicates_to_delete: Vec<String>,
        extra_kinds_to_delete: Vec<String>,
        missing_cover_kinds: Vec<String>,
    },
    PartialOrCorrupted {
        canonical_kinds: HashMap<usize, String>,
        canonical_titles: HashMap<usize, String>,
        canonical_descriptions: HashMap<usize, String>,
        duplicates_to_delete: Vec<String>,
        missing_cover_kinds: Vec<String>,
        extra_payloads: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncShardTarget {
    pub index: usize,
    pub existing_kind: Option<String>,
    pub existing_title: Option<String>,
    pub existing_description: Option<String>,
    pub needs_cover: bool,
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPushPlan {
    pub targets: Vec<SyncShardTarget>,
    pub delete_kinds: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPullOutcome {
    pub active_changed: bool,
    pub needs_push: bool,
}

fn next_local_ts(min_ts: u64) -> u64 {
    let wall = now();
    let mut prev = LAST_LOCAL_TS.load(Ordering::Relaxed);
    let base = loop {
        let candidate = wall.max(prev.saturating_add(1));
        match LAST_LOCAL_TS.compare_exchange_weak(prev, candidate, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break candidate,
            Err(actual) => prev = actual,
        }
    };
    base.max(min_ts)
}

fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).unwrap()
}

fn from_base36(tok: &str) -> Option<u64> {
    if tok.is_empty() || !tok.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    u64::from_str_radix(tok, 36).ok()
}

pub fn encode_sync_id(raw: &str, base: u64) -> String {
    if !raw.is_empty()
        && !raw.starts_with('0')
        && raw.bytes().all(|b| b.is_ascii_digit())
    {
        if let Ok(n) = raw.parse::<u64>() {
            if n > base && n <= base + SYNC_ID_MAX_DELTA {
                return to_base36(n - base);
            }
        }
    }
    format!("~{raw}")
}

pub fn decode_sync_id(tok: &str, base: u64) -> Option<String> {
    if let Some(raw) = tok.strip_prefix('~') {
        return (!raw.is_empty()).then(|| raw.to_string());
    }
    let delta = from_base36(tok)?;
    if delta == 0 || delta > SYNC_ID_MAX_DELTA {
        return None;
    }
    base.checked_add(delta).map(|n| n.to_string())
}

pub fn encode_sync_ts(epoch_sec: u64) -> String {
    to_base36(epoch_sec.saturating_sub(SYNC_EPOCH_OFFSET))
}

pub fn decode_sync_ts(tok: &str) -> Option<u64> {
    from_base36(tok).and_then(|d| SYNC_EPOCH_OFFSET.checked_add(d))
}

pub fn sync_playlist_title(shard_index: usize) -> String {
    if shard_index == 0 {
        "Tune Sync".to_string()
    } else {
        format!("Tune Sync {}", shard_index + 1)
    }
}

pub fn is_sync_playlist(title: &str, description: &str) -> bool {
    if description.contains(SYNC_TAG_PREFIX) || title == "Tune Sync" {
        return true;
    }
    if let Some(rest) = title.strip_prefix("Tune Sync ") {
        let mut bytes = rest.bytes();
        if let Some(first) = bytes.next() {
            return matches!(first, b'2'..=b'9') && bytes.all(|b| b.is_ascii_digit());
        }
    }
    false
}

fn sync_header(part: usize) -> String {
    if part <= 1 {
        "Служебный плейлист Tune — синхронизация лайков и медиатеки Каталога Tune между устройствами. Не удаляйте его (в Tune он скрыт).".to_string()
    } else {
        format!("Служебный плейлист Tune (часть {part}) — синхронизация Каталога Tune. Не удаляйте.")
    }
}

pub fn split_sync_descriptions(payload: &str) -> Vec<String> {
    for total in 1usize..=1000 {
        let mut offset = 0usize;
        let mut descriptions = Vec::with_capacity(total);
        let mut fits = true;
        for idx in 0..total {
            let part = idx + 1;
            let prefix = format!("{}\n\n[tune-sync:v1:{part}/{total}:", sync_header(part));
            let overhead = prefix.chars().count() + 1;
            let cap = SYNC_DESCRIPTION_MAX_CHARS.saturating_sub(overhead);
            if cap == 0 {
                fits = false;
                break;
            }
            let end = (offset + cap).min(payload.len());
            // Полезная нагрузка — чистый ASCII, но на всякий случай идём по границам символов.
            let mut safe_end = end;
            while safe_end > offset && !payload.is_char_boundary(safe_end) {
                safe_end -= 1;
            }
            let chunk = &payload[offset..safe_end];
            offset = safe_end;
            descriptions.push(format!("{prefix}{chunk}]"));
        }
        if fits && offset >= payload.len() {
            return descriptions;
        }
    }
    vec![format!(
        "{}\n\n[tune-sync:v1:1/1:]",
        sync_header(1)
    )]
}

fn is_valid_payload_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'_' | b'+' | b'@' | b'.' | b',' | b'=' | b':' | b';' | b'~' | b'-'
        )
}

fn parse_sync_tag(description: &str) -> Option<(usize, usize, &str)> {
    let start = description.find(SYNC_TAG_PREFIX)?;
    let after = &description[start + SYNC_TAG_PREFIX.len()..];
    let end = after.find(']')?;
    let inner = &after[..end];
    let (counts, chunk) = inner.split_once(':')?;
    let (part_str, total_str) = counts.split_once('/')?;
    let part: usize = part_str.parse().ok()?;
    let total: usize = total_str.parse().ok()?;
    if part == 0 || total == 0 || part > total {
        return None;
    }
    if !chunk.bytes().all(is_valid_payload_byte) {
        return None;
    }
    Some((part, total, chunk))
}

fn shard_index_from_title(title: &str) -> Option<usize> {
    if title == "Tune Sync" {
        return Some(0);
    }
    let rest = title.strip_prefix("Tune Sync ")?;
    let first = rest.bytes().next()?;
    if !matches!(first, b'2'..=b'9') || !rest.bytes().skip(1).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let part: usize = rest.parse().ok()?;
    part.checked_sub(1)
}

pub fn parse_sync_shards(shards: &[SyncShardInfo]) -> ParsedShards {
    if shards.is_empty() {
        return ParsedShards::Empty;
    }

    let mut by_index: HashMap<usize, Vec<&SyncShardInfo>> = HashMap::new();
    let mut unindexed = Vec::new();
    for shard in shards {
        let tag = parse_sync_tag(&shard.description);
        let idx = tag
            .map(|(part, _, _)| part - 1)
            .or_else(|| shard_index_from_title(&shard.title));
        match idx {
            Some(i) => by_index.entry(i).or_default().push(shard),
            None => unindexed.push(shard),
        }
    }

    let mut canonical_kinds = HashMap::new();
    let mut canonical_titles = HashMap::new();
    let mut canonical_descriptions = HashMap::new();
    let mut duplicates_to_delete = Vec::new();
    let mut missing_cover_kinds = Vec::new();
    let mut extra_payloads = Vec::new();

    let mut indices: Vec<usize> = by_index.keys().copied().collect();
    indices.sort_unstable();

    for idx in &indices {
        let group = by_index.get_mut(idx).unwrap();
        group.sort_by(|a, b| {
            let ka = a.kind.parse::<u64>();
            let kb = b.kind.parse::<u64>();
            match (ka, kb) {
                (Ok(na), Ok(nb)) => na.cmp(&nb).then_with(|| a.kind.cmp(&b.kind)),
                (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                (Err(_), Err(_)) => a.kind.cmp(&b.kind),
            }
        });
        let canon = group[0];
        canonical_kinds.insert(*idx, canon.kind.clone());
        canonical_titles.insert(*idx, canon.title.clone());
        canonical_descriptions.insert(*idx, canon.description.clone());
        if !canon.has_custom_cover {
            missing_cover_kinds.push(canon.kind.clone());
        }
        for dup in &group[1..] {
            duplicates_to_delete.push(dup.kind.clone());
            if let Some((1, 1, chunk)) = parse_sync_tag(&dup.description) {
                extra_payloads.push(chunk.to_string());
            }
        }
    }
    for u in &unindexed {
        duplicates_to_delete.push(u.kind.clone());
    }

    if !unindexed.is_empty() && canonical_kinds.is_empty() {
        return ParsedShards::PartialOrCorrupted {
            canonical_kinds,
            canonical_titles,
            canonical_descriptions,
            duplicates_to_delete,
            missing_cover_kinds,
            extra_payloads,
        };
    }

    let expected_total = by_index
        .get(&0)
        .and_then(|group| parse_sync_tag(&group[0].description).map(|(_, total, _)| total));

    let Some(total) = expected_total else {
        return ParsedShards::PartialOrCorrupted {
            canonical_kinds,
            canonical_titles,
            canonical_descriptions,
            duplicates_to_delete,
            missing_cover_kinds,
            extra_payloads,
        };
    };

    let mut primary_payload = String::new();
    for part in 1..=total {
        let idx = part - 1;
        let Some(group) = by_index.get(&idx) else {
            return ParsedShards::PartialOrCorrupted {
                canonical_kinds,
                canonical_titles,
                canonical_descriptions,
                duplicates_to_delete,
                missing_cover_kinds,
                extra_payloads,
            };
        };
        let Some((p, t, chunk)) = parse_sync_tag(&group[0].description) else {
            return ParsedShards::PartialOrCorrupted {
                canonical_kinds,
                canonical_titles,
                canonical_descriptions,
                duplicates_to_delete,
                missing_cover_kinds,
                extra_payloads,
            };
        };
        if p != part || t != total {
            return ParsedShards::PartialOrCorrupted {
                canonical_kinds,
                canonical_titles,
                canonical_descriptions,
                duplicates_to_delete,
                missing_cover_kinds,
                extra_payloads,
            };
        }
        primary_payload.push_str(chunk);
    }

    let mut extra_kinds_to_delete = Vec::new();
    for idx in indices {
        if idx >= total {
            if let Some(kind) = canonical_kinds.get(&idx) {
                extra_kinds_to_delete.push(kind.clone());
            }
        }
    }
    missing_cover_kinds.retain(|k| !extra_kinds_to_delete.contains(k));

    ParsedShards::Complete {
        primary_payload,
        extra_payloads,
        canonical_kinds,
        canonical_titles,
        canonical_descriptions,
        duplicates_to_delete,
        extra_kinds_to_delete,
        missing_cover_kinds,
    }
}

fn encode_pin_key(key: &str) -> Option<(String, String)> {
    if let Some(id) = key.strip_prefix("album:") {
        return Some(("album".to_string(), encode_sync_id(id, SYNTHETIC_ALBUM_BASE)));
    }
    if let Some(id) = key.strip_prefix("artist:") {
        return Some(("artist".to_string(), encode_sync_id(id, SYNTHETIC_ARTIST_BASE)));
    }
    if let Some(id) = key.strip_prefix("track:") {
        return Some(("track".to_string(), encode_sync_id(id, SYNTHETIC_TRACK_BASE)));
    }
    if let Some(rest) = key.strip_prefix("playlist:") {
        let raw = rest
            .strip_prefix(&format!("{CATALOG_OWNER}:"))
            .unwrap_or(rest);
        return Some((
            "playlist".to_string(),
            encode_sync_id(raw, SYNTHETIC_PLAYLIST_BASE),
        ));
    }
    None
}

fn decode_pin_key(pin_kind: &str, enc_id: &str) -> Option<String> {
    match pin_kind {
        "album" => Some(format!("album:{}", decode_sync_id(enc_id, SYNTHETIC_ALBUM_BASE)?)),
        "artist" => Some(format!("artist:{}", decode_sync_id(enc_id, SYNTHETIC_ARTIST_BASE)?)),
        "track" => Some(format!("track:{}", decode_sync_id(enc_id, SYNTHETIC_TRACK_BASE)?)),
        "playlist" => {
            let id = decode_sync_id(enc_id, SYNTHETIC_PLAYLIST_BASE)?;
            if id.contains(':') {
                Some(format!("playlist:{id}"))
            } else {
                Some(format!("playlist:{CATALOG_OWNER}:{id}"))
            }
        }
        _ => None,
    }
}

impl LocalLibrary {
    fn str_list(v: &Value) -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect()
    }

    fn sorted_active(map: &HashMap<String, LwwEntry>) -> Vec<String> {
        let mut items: Vec<(&String, u64)> = map
            .iter()
            .filter(|(_, e)| e.active)
            .map(|(id, e)| (id, e.ts))
            .collect();
        items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        items.into_iter().map(|(id, _)| id.clone()).collect()
    }

    fn rebuild_active_lists(&mut self) {
        self.liked_tracks = Self::sorted_active(&self.liked_tracks_lww);
        self.disliked_tracks = Self::sorted_active(&self.disliked_tracks_lww);
        self.liked_albums = Self::sorted_active(&self.liked_albums_lww);
        self.liked_artists = Self::sorted_active(&self.liked_artists_lww);
        self.disliked_artists = Self::sorted_active(&self.disliked_artists_lww);
        self.liked_playlists = Self::sorted_active(&self.liked_playlists_lww);
        self.pins = Self::sorted_active(&self.pins_lww);

        self.playlist_tracks.clear();
        for (k, entry) in &self.playlist_tracks_lww {
            if !entry.tracks.is_empty() {
                self.playlist_tracks.insert(k.clone(), entry.tracks.clone());
            }
        }
    }

    fn lww_map_from_value(v: &Value) -> HashMap<String, LwwEntry> {
        let mut out = HashMap::new();
        if let Some(obj) = v.as_object() {
            for (k, entry) in obj {
                if let (Some(active), Some(ts)) = (entry["active"].as_bool(), entry["ts"].as_u64()) {
                    out.insert(k.clone(), LwwEntry { active, ts });
                }
            }
        }
        out
    }

    fn lww_map_to_value(map: &HashMap<String, LwwEntry>) -> Value {
        let mut obj = Map::new();
        for (k, entry) in map {
            obj.insert(
                k.clone(),
                json!({ "active": entry.active, "ts": entry.ts }),
            );
        }
        Value::Object(obj)
    }

    fn playlist_lww_map_from_value(v: &Value) -> HashMap<String, PlaylistLwwEntry> {
        let mut out = HashMap::new();
        if let Some(obj) = v.as_object() {
            for (k, entry) in obj {
                if let Some(ts) = entry["ts"].as_u64() {
                    let tracks = Self::str_list(&entry["tracks"]);
                    out.insert(k.clone(), PlaylistLwwEntry { tracks, ts });
                }
            }
        }
        out
    }

    fn playlist_lww_map_to_value(map: &HashMap<String, PlaylistLwwEntry>) -> Value {
        let mut obj = Map::new();
        for (k, entry) in map {
            obj.insert(
                k.clone(),
                json!({ "tracks": entry.tracks, "ts": entry.ts }),
            );
        }
        Value::Object(obj)
    }

    fn seed_legacy_map(ids: Vec<String>, now_sec: u64) -> HashMap<String, LwwEntry> {
        let mut map = HashMap::new();
        for (idx, id) in ids.into_iter().enumerate() {
            map.entry(id).or_insert(LwwEntry {
                active: true,
                ts: now_sec.saturating_sub(idx as u64),
            });
        }
        map
    }

    fn from_value_at(v: &Value, now_sec: u64) -> (Self, bool) {
        if let Some(lww) = v["lww"].as_object() {
            let get_lww = |key: &str| {
                lww.get(key)
                    .map(Self::lww_map_from_value)
                    .unwrap_or_default()
            };
            let playlist_tracks_lww = lww
                .get("playlistTracks")
                .map(Self::playlist_lww_map_from_value)
                .unwrap_or_default();
            let mut lib = Self {
                liked_tracks_lww: get_lww("likedTracks"),
                disliked_tracks_lww: get_lww("dislikedTracks"),
                liked_albums_lww: get_lww("likedAlbums"),
                liked_artists_lww: get_lww("likedArtists"),
                disliked_artists_lww: get_lww("dislikedArtists"),
                liked_playlists_lww: get_lww("likedPlaylists"),
                playlist_tracks_lww,
                pins_lww: get_lww("pins"),
                ..Self::default()
            };
            lib.rebuild_active_lists();
            return (lib, false);
        }

        let liked_tracks_lww = Self::seed_legacy_map(Self::str_list(&v["likedTracks"]), now_sec);
        let disliked_tracks_lww =
            Self::seed_legacy_map(Self::str_list(&v["dislikedTracks"]), now_sec);
        let liked_albums_lww = Self::seed_legacy_map(Self::str_list(&v["likedAlbums"]), now_sec);
        let liked_artists_lww = Self::seed_legacy_map(Self::str_list(&v["likedArtists"]), now_sec);
        let disliked_artists_lww =
            Self::seed_legacy_map(Self::str_list(&v["dislikedArtists"]), now_sec);
        let liked_playlists_lww =
            Self::seed_legacy_map(Self::str_list(&v["likedPlaylists"]), now_sec);
        let pins_lww = Self::seed_legacy_map(Self::str_list(&v["pins"]), now_sec);

        let mut playlist_tracks_lww = HashMap::new();
        if let Some(obj) = v["playlistTracks"].as_object() {
            for (k, arr) in obj {
                let ids = Self::str_list(arr);
                if !ids.is_empty() {
                    playlist_tracks_lww.insert(
                        k.clone(),
                        PlaylistLwwEntry {
                            tracks: ids,
                            ts: now_sec,
                        },
                    );
                }
            }
        }

        let mut lib = Self {
            liked_tracks_lww,
            disliked_tracks_lww,
            liked_albums_lww,
            liked_artists_lww,
            disliked_artists_lww,
            liked_playlists_lww,
            playlist_tracks_lww,
            pins_lww,
            ..Self::default()
        };
        lib.rebuild_active_lists();
        let migrated = lib.has_active_state(None);
        (lib, migrated)
    }

    #[cfg(test)]
    fn from_value(v: &Value) -> Self {
        Self::from_value_at(v, now()).0
    }

    fn to_value(&self) -> Value {
        json!({
            "version": 2,
            "likedTracks": self.liked_tracks,
            "dislikedTracks": self.disliked_tracks,
            "likedAlbums": self.liked_albums,
            "likedArtists": self.liked_artists,
            "dislikedArtists": self.disliked_artists,
            "likedPlaylists": self.liked_playlists,
            "playlistTracks": self.playlist_tracks,
            "pins": self.pins,
            "lww": {
                "likedTracks": Self::lww_map_to_value(&self.liked_tracks_lww),
                "dislikedTracks": Self::lww_map_to_value(&self.disliked_tracks_lww),
                "likedAlbums": Self::lww_map_to_value(&self.liked_albums_lww),
                "likedArtists": Self::lww_map_to_value(&self.liked_artists_lww),
                "dislikedArtists": Self::lww_map_to_value(&self.disliked_artists_lww),
                "likedPlaylists": Self::lww_map_to_value(&self.liked_playlists_lww),
                "playlistTracks": Self::playlist_lww_map_to_value(&self.playlist_tracks_lww),
                "pins": Self::lww_map_to_value(&self.pins_lww),
            }
        })
    }

    fn has_active_state(&self, uid: Option<&str>) -> bool {
        if !self.liked_tracks.is_empty()
            || !self.disliked_tracks.is_empty()
            || !self.liked_albums.is_empty()
            || !self.liked_artists.is_empty()
            || !self.disliked_artists.is_empty()
            || !self.liked_playlists.is_empty()
            || !self.pins.is_empty()
        {
            return true;
        }
        match uid {
            Some(u) if !u.is_empty() => {
                let prefix = format!("{u}:");
                self.playlist_tracks
                    .iter()
                    .any(|(k, v)| !v.is_empty() && (k.starts_with(&prefix) || !k.contains(':')))
            }
            _ => self.playlist_tracks.values().any(|v| !v.is_empty()),
        }
    }

    fn set_entry(map: &mut HashMap<String, LwwEntry>, id: &str, active: bool, ts: u64) {
        map.insert(id.to_string(), LwwEntry { active, ts });
    }

    fn deactivate_if_active(map: &mut HashMap<String, LwwEntry>, id: &str, ts: u64) {
        if map.get(id).is_some_and(|e| e.active) {
            map.insert(id.to_string(), LwwEntry { active: false, ts });
        }
    }

    fn set_track_liked_at(&mut self, id: &str, liked: bool, ts: u64) {
        if liked {
            Self::deactivate_if_active(&mut self.disliked_tracks_lww, id, ts);
        }
        Self::set_entry(&mut self.liked_tracks_lww, id, liked, ts);
        self.rebuild_active_lists();
    }

    fn set_track_liked(&mut self, id: &str, liked: bool) {
        let min = self
            .liked_tracks_lww
            .get(id)
            .map_or(0, |e| e.ts.saturating_add(1))
            .max(
                self.disliked_tracks_lww
                    .get(id)
                    .map_or(0, |e| e.ts.saturating_add(1)),
            );
        let ts = next_local_ts(min);
        self.set_track_liked_at(id, liked, ts);
    }

    fn set_track_disliked_at(&mut self, id: &str, disliked: bool, ts: u64) {
        if disliked {
            Self::deactivate_if_active(&mut self.liked_tracks_lww, id, ts);
        }
        Self::set_entry(&mut self.disliked_tracks_lww, id, disliked, ts);
        self.rebuild_active_lists();
    }

    fn set_track_disliked(&mut self, id: &str, disliked: bool) {
        let min = self
            .disliked_tracks_lww
            .get(id)
            .map_or(0, |e| e.ts.saturating_add(1))
            .max(
                self.liked_tracks_lww
                    .get(id)
                    .map_or(0, |e| e.ts.saturating_add(1)),
            );
        let ts = next_local_ts(min);
        self.set_track_disliked_at(id, disliked, ts);
    }

    fn set_album_liked_at(&mut self, id: &str, liked: bool, ts: u64) {
        Self::set_entry(&mut self.liked_albums_lww, id, liked, ts);
        self.rebuild_active_lists();
    }

    fn set_album_liked(&mut self, id: &str, liked: bool) {
        let min = self
            .liked_albums_lww
            .get(id)
            .map_or(0, |e| e.ts.saturating_add(1));
        self.set_album_liked_at(id, liked, next_local_ts(min));
    }

    fn set_artist_liked_at(&mut self, id: &str, liked: bool, ts: u64) {
        if liked {
            Self::deactivate_if_active(&mut self.disliked_artists_lww, id, ts);
        }
        Self::set_entry(&mut self.liked_artists_lww, id, liked, ts);
        self.rebuild_active_lists();
    }

    fn set_artist_liked(&mut self, id: &str, liked: bool) {
        let min = self
            .liked_artists_lww
            .get(id)
            .map_or(0, |e| e.ts.saturating_add(1))
            .max(
                self.disliked_artists_lww
                    .get(id)
                    .map_or(0, |e| e.ts.saturating_add(1)),
            );
        let ts = next_local_ts(min);
        self.set_artist_liked_at(id, liked, ts);
    }

    fn set_artist_disliked_at(&mut self, id: &str, disliked: bool, ts: u64) {
        if disliked {
            Self::deactivate_if_active(&mut self.liked_artists_lww, id, ts);
        }
        Self::set_entry(&mut self.disliked_artists_lww, id, disliked, ts);
        self.rebuild_active_lists();
    }

    fn set_artist_disliked(&mut self, id: &str, disliked: bool) {
        let min = self
            .disliked_artists_lww
            .get(id)
            .map_or(0, |e| e.ts.saturating_add(1))
            .max(
                self.liked_artists_lww
                    .get(id)
                    .map_or(0, |e| e.ts.saturating_add(1)),
            );
        let ts = next_local_ts(min);
        self.set_artist_disliked_at(id, disliked, ts);
    }

    fn set_playlist_liked_at(&mut self, kind: &str, liked: bool, ts: u64) {
        Self::set_entry(&mut self.liked_playlists_lww, kind, liked, ts);
        self.rebuild_active_lists();
    }

    fn set_playlist_liked(&mut self, kind: &str, liked: bool) {
        let min = self
            .liked_playlists_lww
            .get(kind)
            .map_or(0, |e| e.ts.saturating_add(1));
        self.set_playlist_liked_at(kind, liked, next_local_ts(min));
    }

    fn add_playlist_track_at(&mut self, key: &str, track_id: &str, ts: u64) {
        let entry = self
            .playlist_tracks_lww
            .entry(key.to_string())
            .or_default();
        entry.tracks.retain(|x| x != track_id);
        entry.tracks.insert(0, track_id.to_string());
        entry.ts = ts;
        self.rebuild_active_lists();
    }

    fn add_playlist_track(&mut self, key: &str, track_id: &str) {
        let min = self
            .playlist_tracks_lww
            .get(key)
            .map_or(0, |e| e.ts.saturating_add(1));
        self.add_playlist_track_at(key, track_id, next_local_ts(min));
    }

    fn remove_playlist_track_at(&mut self, key: &str, track_id: &str, ts: u64) -> bool {
        let Some(entry) = self.playlist_tracks_lww.get_mut(key) else {
            return false;
        };
        let before = entry.tracks.len();
        entry.tracks.retain(|x| x != track_id);
        if entry.tracks.len() == before {
            return false;
        }
        entry.ts = ts;
        self.rebuild_active_lists();
        true
    }

    fn remove_playlist_track(&mut self, key: &str, track_id: &str) -> bool {
        let min = self
            .playlist_tracks_lww
            .get(key)
            .map_or(0, |e| e.ts.saturating_add(1));
        self.remove_playlist_track_at(key, track_id, next_local_ts(min))
    }

    fn move_playlist_track(&mut self, key: &str, track_id: &str, from: usize, to: usize) -> bool {
        let min = self
            .playlist_tracks_lww
            .get(key)
            .map_or(0, |e| e.ts.saturating_add(1));
        let Some(entry) = self.playlist_tracks_lww.get_mut(key) else {
            return false;
        };
        let Some(at) = entry.tracks.iter().position(|x| x == track_id) else {
            return false;
        };
        let max_idx = entry.tracks.len().saturating_sub(1);
        let target =
            (at as isize + to as isize - from as isize).clamp(0, max_idx as isize) as usize;
        if target == at {
            return false;
        }
        let item = entry.tracks.remove(at);
        entry.tracks.insert(target, item);
        entry.ts = next_local_ts(min);
        self.rebuild_active_lists();
        true
    }

    fn clear_playlist_tracks(&mut self, key: &str) -> bool {
        let Some(entry) = self.playlist_tracks_lww.get_mut(key) else {
            return false;
        };
        let ts = next_local_ts(entry.ts.saturating_add(1));
        let had = !entry.tracks.is_empty();
        entry.tracks.clear();
        entry.ts = ts;
        self.rebuild_active_lists();
        had
    }

    fn set_pin_at(&mut self, key: &str, pinned: bool, ts: u64) {
        Self::set_entry(&mut self.pins_lww, key, pinned, ts);
        self.rebuild_active_lists();
    }

    fn set_pin(&mut self, key: &str, pinned: bool) {
        let min = self
            .pins_lww
            .get(key)
            .map_or(0, |e| e.ts.saturating_add(1));
        self.set_pin_at(key, pinned, next_local_ts(min));
    }

    fn prune_expired_tombstones(&mut self, now_sec: u64) {
        let keep = |e: &LwwEntry| e.active || now_sec.saturating_sub(e.ts) <= SYNC_TOMBSTONE_TTL_SECS;
        self.liked_tracks_lww.retain(|_, e| keep(e));
        self.disliked_tracks_lww.retain(|_, e| keep(e));
        self.liked_albums_lww.retain(|_, e| keep(e));
        self.liked_artists_lww.retain(|_, e| keep(e));
        self.disliked_artists_lww.retain(|_, e| keep(e));
        self.liked_playlists_lww.retain(|_, e| keep(e));
        self.pins_lww.retain(|_, e| keep(e));
        self.playlist_tracks_lww
            .retain(|_, e| !e.tracks.is_empty() || now_sec.saturating_sub(e.ts) <= SYNC_TOMBSTONE_TTL_SECS);
    }

    fn encode_lww_section(
        prefix: &str,
        map: &HashMap<String, LwwEntry>,
        base: u64,
        now_sec: u64,
    ) -> Option<String> {
        let mut items: Vec<(&String, &LwwEntry)> = map
            .iter()
            .filter(|(_, e)| e.active || now_sec.saturating_sub(e.ts) <= SYNC_TOMBSTONE_TTL_SECS)
            .collect();
        if items.is_empty() {
            return None;
        }
        items.sort_by(|a, b| {
            b.1.ts
                .cmp(&a.1.ts)
                .then_with(|| b.1.active.cmp(&a.1.active))
                .then_with(|| a.0.cmp(b.0))
        });
        let encoded: Vec<String> = items
            .into_iter()
            .map(|(id, e)| {
                let sign = if e.active { '+' } else { '-' };
                format!("{sign}{}@{}", encode_sync_id(id, base), encode_sync_ts(e.ts))
            })
            .collect();
        Some(format!("{prefix}:{}", encoded.join(",")))
    }

    fn encode_payload(&self, uid: Option<&str>, now_sec: u64) -> String {
        let mut sections = Vec::new();
        if let Some(s) = Self::encode_lww_section("L", &self.liked_tracks_lww, SYNTHETIC_TRACK_BASE, now_sec) {
            sections.push(s);
        }
        if let Some(s) = Self::encode_lww_section("D", &self.disliked_tracks_lww, SYNTHETIC_TRACK_BASE, now_sec) {
            sections.push(s);
        }
        if let Some(s) = Self::encode_lww_section("A", &self.liked_albums_lww, SYNTHETIC_ALBUM_BASE, now_sec) {
            sections.push(s);
        }
        if let Some(s) = Self::encode_lww_section("R", &self.liked_artists_lww, SYNTHETIC_ARTIST_BASE, now_sec) {
            sections.push(s);
        }
        if let Some(s) = Self::encode_lww_section("B", &self.disliked_artists_lww, SYNTHETIC_ARTIST_BASE, now_sec) {
            sections.push(s);
        }
        if let Some(s) = Self::encode_lww_section("Q", &self.liked_playlists_lww, SYNTHETIC_PLAYLIST_BASE, now_sec) {
            sections.push(s);
        }

        let mut pl_items: Vec<(String, &PlaylistLwwEntry)> = Vec::new();
        for (key, entry) in &self.playlist_tracks_lww {
            if entry.tracks.is_empty() && now_sec.saturating_sub(entry.ts) > SYNC_TOMBSTONE_TTL_SECS {
                continue;
            }
            let wire_kind = match uid {
                Some(u) if !u.is_empty() => {
                    if let Some(rest) = key.strip_prefix(&format!("{u}:")) {
                        rest.to_string()
                    } else if !key.contains(':') {
                        key.clone()
                    } else {
                        continue;
                    }
                }
                _ => key
                    .split_once(':')
                    .map_or_else(|| key.clone(), |(_, k)| k.to_string()),
            };
            if !wire_kind.is_empty() {
                pl_items.push((wire_kind, entry));
            }
        }
        if !pl_items.is_empty() {
            pl_items.sort_by(|a, b| a.0.cmp(&b.0));
            let encoded: Vec<String> = pl_items
                .into_iter()
                .map(|(kind, entry)| {
                    let tracks = entry
                        .tracks
                        .iter()
                        .map(|id| encode_sync_id(id, SYNTHETIC_TRACK_BASE))
                        .collect::<Vec<_>>()
                        .join(".");
                    format!("{kind}@{}={tracks}", encode_sync_ts(entry.ts))
                })
                .collect();
            sections.push(format!("P:{}", encoded.join(",")));
        }

        let mut pin_items: Vec<(&String, &LwwEntry, String, String)> = self
            .pins_lww
            .iter()
            .filter(|(_, e)| e.active || now_sec.saturating_sub(e.ts) <= SYNC_TOMBSTONE_TTL_SECS)
            .filter_map(|(k, e)| {
                let (pin_kind, enc_id) = encode_pin_key(k)?;
                Some((k, e, pin_kind, enc_id))
            })
            .collect();
        if !pin_items.is_empty() {
            pin_items.sort_by(|a, b| {
                b.1.ts
                    .cmp(&a.1.ts)
                    .then_with(|| b.1.active.cmp(&a.1.active))
                    .then_with(|| a.0.cmp(b.0))
            });
            let encoded: Vec<String> = pin_items
                .into_iter()
                .map(|(_, e, pin_kind, enc_id)| {
                    let sign = if e.active { '+' } else { '-' };
                    format!("{sign}{pin_kind}:{enc_id}@{}", encode_sync_ts(e.ts))
                })
                .collect();
            sections.push(format!("N:{}", encoded.join(",")));
        }

        sections.join(";")
    }

    fn merge_lww_entry(map: &mut HashMap<String, LwwEntry>, id: String, incoming: LwwEntry) -> bool {
        match map.get_mut(&id) {
            Some(cur) => {
                if incoming.ts > cur.ts || (incoming.ts == cur.ts && incoming.active && !cur.active) {
                    let changed = *cur != incoming;
                    *cur = incoming;
                    changed
                } else {
                    false
                }
            }
            None => {
                map.insert(id, incoming);
                true
            }
        }
    }

    fn decode_lww_section(
        body: &str,
        base: u64,
        target: &mut HashMap<String, LwwEntry>,
    ) -> Result<(), String> {
        if body.is_empty() {
            return Ok(());
        }
        for tok in body.split(',') {
            let (active, rest) = if let Some(r) = tok.strip_prefix('+') {
                (true, r)
            } else if let Some(r) = tok.strip_prefix('-') {
                (false, r)
            } else {
                return Err(format!("некорректная запись sync: {tok}"));
            };
            let (enc_id, ts_tok) = rest
                .split_once('@')
                .ok_or_else(|| format!("нет времени в sync: {tok}"))?;
            let id = decode_sync_id(enc_id, base)
                .ok_or_else(|| format!("некорректный id в sync: {enc_id}"))?;
            let ts = decode_sync_ts(ts_tok)
                .ok_or_else(|| format!("некорректная метка времени в sync: {ts_tok}"))?;
            Self::merge_lww_entry(target, id, LwwEntry { active, ts });
        }
        Ok(())
    }

    fn decode_payload(payload: &str, uid: Option<&str>) -> Result<Self, String> {
        let mut lib = Self::default();
        if payload.trim().is_empty() {
            return Ok(lib);
        }
        for section in payload.split(';') {
            if section.is_empty() {
                continue;
            }
            let (tag, body) = section
                .split_once(':')
                .ok_or_else(|| format!("некорректная секция sync: {section}"))?;
            match tag {
                "L" => Self::decode_lww_section(body, SYNTHETIC_TRACK_BASE, &mut lib.liked_tracks_lww)?,
                "D" => Self::decode_lww_section(body, SYNTHETIC_TRACK_BASE, &mut lib.disliked_tracks_lww)?,
                "A" => Self::decode_lww_section(body, SYNTHETIC_ALBUM_BASE, &mut lib.liked_albums_lww)?,
                "R" => Self::decode_lww_section(body, SYNTHETIC_ARTIST_BASE, &mut lib.liked_artists_lww)?,
                "B" => Self::decode_lww_section(body, SYNTHETIC_ARTIST_BASE, &mut lib.disliked_artists_lww)?,
                "Q" => Self::decode_lww_section(body, SYNTHETIC_PLAYLIST_BASE, &mut lib.liked_playlists_lww)?,
                "P" => {
                    if !body.is_empty() {
                        for entry in body.split(',') {
                            let (left, right) = entry
                                .split_once('=')
                                .ok_or_else(|| format!("некорректная запись P: {entry}"))?;
                            let (kind, ts_tok) = left
                                .split_once('@')
                                .ok_or_else(|| format!("нет времени P: {entry}"))?;
                            if kind.is_empty() {
                                return Err("пустой kind плейлиста в sync".to_string());
                            }
                            let ts = decode_sync_ts(ts_tok)
                                .ok_or_else(|| format!("некорректная метка времени P: {ts_tok}"))?;
                            let tracks = if right.is_empty() {
                                Vec::new()
                            } else {
                                right
                                    .split('.')
                                    .map(|tok| {
                                        decode_sync_id(tok, SYNTHETIC_TRACK_BASE).ok_or_else(|| {
                                            format!("некорректный трек P в sync: {tok}")
                                        })
                                    })
                                    .collect::<Result<Vec<_>, _>>()?
                            };
                            let key = match uid {
                                Some(u) if !u.is_empty() => format!("{u}:{kind}"),
                                _ => kind.to_string(),
                            };
                            let incoming = PlaylistLwwEntry { tracks, ts };
                            match lib.playlist_tracks_lww.get_mut(&key) {
                                Some(cur) if incoming.ts > cur.ts => *cur = incoming,
                                None => {
                                    lib.playlist_tracks_lww.insert(key, incoming);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "N" => {
                    if !body.is_empty() {
                        for tok in body.split(',') {
                            let (active, rest) = if let Some(r) = tok.strip_prefix('+') {
                                (true, r)
                            } else if let Some(r) = tok.strip_prefix('-') {
                                (false, r)
                            } else {
                                return Err(format!("некорректная запись N: {tok}"));
                            };
                            let (left, ts_tok) = rest
                                .split_once('@')
                                .ok_or_else(|| format!("нет времени N: {tok}"))?;
                            let (pin_kind, enc_id) = left
                                .split_once(':')
                                .ok_or_else(|| format!("нет вида пина N: {tok}"))?;
                            let ts = decode_sync_ts(ts_tok)
                                .ok_or_else(|| format!("некорректная метка времени N: {ts_tok}"))?;
                            if let Some(key) = decode_pin_key(pin_kind, enc_id) {
                                Self::merge_lww_entry(
                                    &mut lib.pins_lww,
                                    key,
                                    LwwEntry { active, ts },
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        lib.resolve_mutual_exclusions();
        lib.rebuild_active_lists();
        Ok(lib)
    }

    fn resolve_mutual_exclusions(&mut self) {
        let resolve = |likes: &mut HashMap<String, LwwEntry>, dislikes: &mut HashMap<String, LwwEntry>| {
            let keys: Vec<String> = likes
                .iter()
                .filter(|(k, le)| le.active && dislikes.get(*k).is_some_and(|de| de.active))
                .map(|(k, _)| k.clone())
                .collect();
            for k in keys {
                let lt = likes[&k].ts;
                let dt = dislikes[&k].ts;
                if dt > lt {
                    likes.insert(k, LwwEntry { active: false, ts: dt });
                } else {
                    dislikes.insert(k, LwwEntry { active: false, ts: lt });
                }
            }
        };
        resolve(&mut self.liked_tracks_lww, &mut self.disliked_tracks_lww);
        resolve(&mut self.liked_artists_lww, &mut self.disliked_artists_lww);
    }

    fn merge_from(&mut self, remote: &LocalLibrary) -> (bool /* lww_changed */, bool /* active_changed */) {
        let before_active = (
            self.liked_tracks.clone(),
            self.disliked_tracks.clone(),
            self.liked_albums.clone(),
            self.liked_artists.clone(),
            self.disliked_artists.clone(),
            self.liked_playlists.clone(),
            self.playlist_tracks.clone(),
            self.pins.clone(),
        );
        let mut lww_changed = false;

        let mut merge_map = |dst: &mut HashMap<String, LwwEntry>, src: &HashMap<String, LwwEntry>| {
            for (id, entry) in src {
                if Self::merge_lww_entry(dst, id.clone(), entry.clone()) {
                    lww_changed = true;
                }
            }
        };

        merge_map(&mut self.liked_tracks_lww, &remote.liked_tracks_lww);
        merge_map(&mut self.disliked_tracks_lww, &remote.disliked_tracks_lww);
        merge_map(&mut self.liked_albums_lww, &remote.liked_albums_lww);
        merge_map(&mut self.liked_artists_lww, &remote.liked_artists_lww);
        merge_map(&mut self.disliked_artists_lww, &remote.disliked_artists_lww);
        merge_map(&mut self.liked_playlists_lww, &remote.liked_playlists_lww);
        merge_map(&mut self.pins_lww, &remote.pins_lww);

        for (key, incoming) in &remote.playlist_tracks_lww {
            match self.playlist_tracks_lww.get_mut(key) {
                Some(cur) => {
                    let take = incoming.ts > cur.ts
                        || (incoming.ts == cur.ts
                            && incoming.tracks != cur.tracks
                            && (incoming.tracks.len() > cur.tracks.len()
                                || (incoming.tracks.len() == cur.tracks.len()
                                    && incoming.tracks < cur.tracks)));
                    if take {
                        *cur = incoming.clone();
                        lww_changed = true;
                    }
                }
                None => {
                    self.playlist_tracks_lww.insert(key.clone(), incoming.clone());
                    lww_changed = true;
                }
            }
        }

        self.resolve_mutual_exclusions();
        self.rebuild_active_lists();

        let active_changed = before_active
            != (
                self.liked_tracks.clone(),
                self.disliked_tracks.clone(),
                self.liked_albums.clone(),
                self.liked_artists.clone(),
                self.disliked_artists.clone(),
                self.liked_playlists.clone(),
                self.playlist_tracks.clone(),
                self.pins.clone(),
            );
        (lww_changed, active_changed)
    }

    fn has_newer_state_than(&self, remote: &LocalLibrary, uid: Option<&str>, now_sec: u64) -> bool {
        let map_newer = |local: &HashMap<String, LwwEntry>, rem: &HashMap<String, LwwEntry>| -> bool {
            local.iter().any(|(id, le)| {
                if !le.active && now_sec.saturating_sub(le.ts) > SYNC_TOMBSTONE_TTL_SECS {
                    return false;
                }
                match rem.get(id) {
                    Some(re) => le.ts > re.ts || (le.ts == re.ts && le.active && !re.active),
                    None => le.active,
                }
            })
        };

        if map_newer(&self.liked_tracks_lww, &remote.liked_tracks_lww)
            || map_newer(&self.disliked_tracks_lww, &remote.disliked_tracks_lww)
            || map_newer(&self.liked_albums_lww, &remote.liked_albums_lww)
            || map_newer(&self.liked_artists_lww, &remote.liked_artists_lww)
            || map_newer(&self.disliked_artists_lww, &remote.disliked_artists_lww)
            || map_newer(&self.liked_playlists_lww, &remote.liked_playlists_lww)
            || map_newer(&self.pins_lww, &remote.pins_lww)
        {
            return true;
        }

        for (key, le) in &self.playlist_tracks_lww {
            if le.tracks.is_empty() && now_sec.saturating_sub(le.ts) > SYNC_TOMBSTONE_TTL_SECS {
                continue;
            }
            if let Some(u) = uid.filter(|u| !u.is_empty()) {
                if !key.starts_with(&format!("{u}:")) && key.contains(':') {
                    continue;
                }
            }
            let rem_entry = remote.playlist_tracks_lww.get(key).or_else(|| {
                uid.and_then(|u| {
                    key.strip_prefix(&format!("{u}:"))
                        .and_then(|k| remote.playlist_tracks_lww.get(k))
                })
            });
            match rem_entry {
                Some(re) => {
                    if le.ts > re.ts || (le.ts == re.ts && le.tracks != re.tracks) {
                        return true;
                    }
                }
                None if !le.tracks.is_empty() => return true,
                _ => {}
            }
        }
        false
    }
}

fn local_lib_path() -> std::path::PathBuf {
    crate::api::data_dir().join("catalog_library.json")
}

fn write_local_library(lib: &LocalLibrary) -> Result<(), String> {
    let dir = crate::api::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(
        local_lib_path(),
        serde_json::to_string_pretty(&lib.to_value()).unwrap_or_default(),
    )
    .map_err(|e| format!("Не удалось сохранить библиотеку каталога: {e}"))
}

fn load_or_migrate_local_library() -> LocalLibrary {
    let now_sec = now();
    let (lib, migrated) = std::fs::read(local_lib_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|v| LocalLibrary::from_value_at(&v, now_sec))
        .unwrap_or_default();
    if migrated {
        LAST_LOCAL_TS.fetch_max(now_sec, Ordering::Relaxed);
        let _ = write_local_library(&lib);
    }
    lib
}

fn read_local_library() {
    let lib = load_or_migrate_local_library();
    *LOCAL_LIB.write().unwrap() = Some(lib);
}

pub fn remote_sync_generation() -> u64 {
    REMOTE_SYNC_GEN.load(Ordering::Relaxed)
}

fn with_local_lib_read<R>(f: impl FnOnce(&LocalLibrary) -> R) -> R {
    let mut guard = LOCAL_LIB.write().unwrap();
    let lib = guard.get_or_insert_with(load_or_migrate_local_library);
    f(lib)
}

fn mutate_local_lib(f: impl FnOnce(&mut LocalLibrary) -> bool) -> Result<(), String> {
    let mut guard = LOCAL_LIB.write().unwrap();
    let lib = guard.get_or_insert_with(load_or_migrate_local_library);
    let changed = f(lib);
    if !changed {
        return Ok(());
    }
    let res = write_local_library(lib);
    drop(guard);
    if res.is_ok() {
        crate::api::schedule_sync_push();
    }
    res
}

pub fn apply_remote_sync_shards(uid: &str, shards: &[SyncShardInfo]) -> SyncPullOutcome {
    let parsed = parse_sync_shards(shards);
    let mut guard = LOCAL_LIB.write().unwrap();
    let lib = guard.get_or_insert_with(load_or_migrate_local_library);
    let now_sec = now();

    match parsed {
        ParsedShards::Empty => SyncPullOutcome {
            active_changed: false,
            needs_push: lib.has_active_state(Some(uid)),
        },
        ParsedShards::PartialOrCorrupted { .. } => SyncPullOutcome {
            active_changed: false,
            needs_push: false,
        },
        ParsedShards::Complete {
            ref primary_payload,
            ref extra_payloads,
            ref canonical_kinds,
            ref canonical_titles,
            ref duplicates_to_delete,
            ref extra_kinds_to_delete,
            ref missing_cover_kinds,
            ..
        } => {
            let mut lww_changed = false;
            let mut active_changed = false;

            let primary_lib = match LocalLibrary::decode_payload(primary_payload, Some(uid)) {
                Ok(decoded) => {
                    let (lc, ac) = lib.merge_from(&decoded);
                    lww_changed |= lc;
                    active_changed |= ac;
                    Some(decoded)
                }
                Err(e) => {
                    crate::api::log(&format!("Tune Sync разбор основного фрагмента: {e}"));
                    return SyncPullOutcome {
                        active_changed: false,
                        needs_push: false,
                    };
                }
            };

            for extra in extra_payloads {
                if let Ok(decoded) = LocalLibrary::decode_payload(extra, Some(uid)) {
                    let (lc, ac) = lib.merge_from(&decoded);
                    lww_changed |= lc;
                    active_changed |= ac;
                }
            }

            if lww_changed {
                if active_changed {
                    REMOTE_SYNC_GEN.fetch_add(1, Ordering::Relaxed);
                }
                let _ = write_local_library(lib);
            }

            let target_payload = lib.encode_payload(Some(uid), now_sec);
            let descriptions = split_sync_descriptions(&target_payload);
            let missing_shards = (0..descriptions.len()).any(|i| !canonical_kinds.contains_key(&i));
            let wrong_titles = (0..descriptions.len())
                .any(|i| canonical_titles.get(&i) != Some(&sync_playlist_title(i)));
            let newer_local = primary_lib
                .as_ref()
                .is_some_and(|rem| lib.has_newer_state_than(rem, Some(uid), now_sec));

            let needs_push = newer_local
                || !extra_payloads.is_empty()
                || !duplicates_to_delete.is_empty()
                || !extra_kinds_to_delete.is_empty()
                || !missing_cover_kinds.is_empty()
                || missing_shards
                || wrong_titles;

            SyncPullOutcome {
                active_changed,
                needs_push,
            }
        }
    }
}

pub fn build_sync_push_plan(uid: &str, shards: &[SyncShardInfo]) -> Option<SyncPushPlan> {
    let parsed = parse_sync_shards(shards);
    let mut guard = LOCAL_LIB.write().unwrap();
    let lib = guard.get_or_insert_with(load_or_migrate_local_library);
    let now_sec = now();

    let (canonical_kinds, canonical_titles, canonical_descriptions, mut delete_kinds, missing_covers, is_empty) =
        match parsed {
            ParsedShards::Empty => (
                HashMap::new(),
                HashMap::new(),
                HashMap::new(),
                Vec::new(),
                Vec::new(),
                true,
            ),
            ParsedShards::Complete {
                primary_payload,
                extra_payloads,
                canonical_kinds,
                canonical_titles,
                canonical_descriptions,
                mut duplicates_to_delete,
                extra_kinds_to_delete,
                missing_cover_kinds,
            } => {
                let mut lww_changed = false;
                let mut active_changed = false;
                if let Ok(decoded) = LocalLibrary::decode_payload(&primary_payload, Some(uid)) {
                    let (lc, ac) = lib.merge_from(&decoded);
                    lww_changed |= lc;
                    active_changed |= ac;
                }
                for extra in extra_payloads {
                    if let Ok(decoded) = LocalLibrary::decode_payload(&extra, Some(uid)) {
                        let (lc, ac) = lib.merge_from(&decoded);
                        lww_changed |= lc;
                        active_changed |= ac;
                    }
                }
                if lww_changed {
                    if active_changed {
                        REMOTE_SYNC_GEN.fetch_add(1, Ordering::Relaxed);
                    }
                    let _ = write_local_library(lib);
                }
                duplicates_to_delete.extend(extra_kinds_to_delete);
                (
                    canonical_kinds,
                    canonical_titles,
                    canonical_descriptions,
                    duplicates_to_delete,
                    missing_cover_kinds,
                    false,
                )
            }
            ParsedShards::PartialOrCorrupted {
                canonical_kinds,
                canonical_titles,
                canonical_descriptions,
                duplicates_to_delete,
                missing_cover_kinds,
                extra_payloads,
            } => {
                let mut lww_changed = false;
                let mut active_changed = false;
                for extra in extra_payloads {
                    if let Ok(decoded) = LocalLibrary::decode_payload(&extra, Some(uid)) {
                        let (lc, ac) = lib.merge_from(&decoded);
                        lww_changed |= lc;
                        active_changed |= ac;
                    }
                }
                if lww_changed {
                    if active_changed {
                        REMOTE_SYNC_GEN.fetch_add(1, Ordering::Relaxed);
                    }
                    let _ = write_local_library(lib);
                }
                (
                    canonical_kinds,
                    canonical_titles,
                    canonical_descriptions,
                    duplicates_to_delete,
                    missing_cover_kinds,
                    false,
                )
            }
        };

    lib.prune_expired_tombstones(now_sec);
    let _ = write_local_library(lib);

    if is_empty && !lib.has_active_state(Some(uid)) {
        return None;
    }

    let payload = lib.encode_payload(Some(uid), now_sec);
    let descriptions = split_sync_descriptions(&payload);
    let missing_set: HashSet<String> = missing_covers.into_iter().collect();

    for (idx, kind) in &canonical_kinds {
        if *idx >= descriptions.len() && !delete_kinds.contains(kind) {
            delete_kinds.push(kind.clone());
        }
    }

    let targets = descriptions
        .into_iter()
        .enumerate()
        .map(|(idx, description)| {
            let existing_kind = canonical_kinds.get(&idx).cloned();
            let needs_cover = existing_kind
                .as_ref()
                .is_none_or(|k| missing_set.contains(k));
            SyncShardTarget {
                index: idx,
                existing_kind,
                existing_title: canonical_titles.get(&idx).cloned(),
                existing_description: canonical_descriptions.get(&idx).cloned(),
                needs_cover,
                title: sync_playlist_title(idx),
                description,
            }
        })
        .collect();

    Some(SyncPushPlan {
        targets,
        delete_kinds,
    })
}

pub fn local_liked_tracks() -> Vec<String> {
    if !catalog_enabled() {
        return Vec::new();
    }
    with_local_lib_read(|lib| lib.liked_tracks.clone())
}

pub fn set_local_track_liked(id: &str, liked: bool) -> Result<(), String> {
    mutate_local_lib(|lib| {
        lib.set_track_liked(id, liked);
        true
    })
}

pub fn local_disliked_tracks() -> Vec<String> {
    if !catalog_enabled() {
        return Vec::new();
    }
    with_local_lib_read(|lib| lib.disliked_tracks.clone())
}

pub fn set_local_track_disliked(id: &str, disliked: bool) -> Result<(), String> {
    mutate_local_lib(|lib| {
        lib.set_track_disliked(id, disliked);
        true
    })
}

pub fn local_liked_albums() -> Vec<String> {
    if !catalog_enabled() {
        return Vec::new();
    }
    with_local_lib_read(|lib| lib.liked_albums.clone())
}

pub fn set_local_album_liked(id: &str, liked: bool) -> Result<(), String> {
    mutate_local_lib(|lib| {
        lib.set_album_liked(id, liked);
        true
    })
}

pub fn local_liked_artists() -> Vec<String> {
    if !catalog_enabled() {
        return Vec::new();
    }
    with_local_lib_read(|lib| lib.liked_artists.clone())
}

pub fn set_local_artist_liked(id: &str, liked: bool) -> Result<(), String> {
    mutate_local_lib(|lib| {
        lib.set_artist_liked(id, liked);
        true
    })
}

pub fn local_disliked_artists() -> Vec<String> {
    if !catalog_enabled() {
        return Vec::new();
    }
    with_local_lib_read(|lib| lib.disliked_artists.clone())
}

pub fn set_local_artist_disliked(id: &str, disliked: bool) -> Result<(), String> {
    mutate_local_lib(|lib| {
        lib.set_artist_disliked(id, disliked);
        true
    })
}

pub fn local_liked_playlists() -> Vec<String> {
    if !catalog_enabled() {
        return Vec::new();
    }
    with_local_lib_read(|lib| lib.liked_playlists.clone())
}

pub fn set_local_playlist_liked(kind: &str, liked: bool) -> Result<(), String> {
    mutate_local_lib(|lib| {
        lib.set_playlist_liked(kind, liked);
        true
    })
}

pub fn local_playlist_tracks(uid: &str, kind: &str) -> Vec<String> {
    if !catalog_enabled() {
        return Vec::new();
    }
    let key = format!("{uid}:{kind}");
    with_local_lib_read(|lib| lib.playlist_tracks.get(&key).cloned().unwrap_or_default())
}

pub fn add_local_playlist_track(uid: &str, kind: &str, track_id: &str) -> Result<(), String> {
    let key = format!("{uid}:{kind}");
    mutate_local_lib(|lib| {
        lib.add_playlist_track(&key, track_id);
        true
    })
}

pub fn remove_local_playlist_track(uid: &str, kind: &str, track_id: &str) -> Result<(), String> {
    let key = format!("{uid}:{kind}");
    mutate_local_lib(|lib| lib.remove_playlist_track(&key, track_id))
}

pub fn move_local_playlist_track(
    uid: &str,
    kind: &str,
    track_id: &str,
    from: usize,
    to: usize,
) -> Result<(), String> {
    let key = format!("{uid}:{kind}");
    mutate_local_lib(|lib| lib.move_playlist_track(&key, track_id, from, to))
}

pub fn clear_local_playlist(uid: &str, kind: &str) {
    let key = format!("{uid}:{kind}");
    let _ = mutate_local_lib(|lib| lib.clear_playlist_tracks(&key));
}

pub fn local_pins() -> Vec<String> {
    if !catalog_enabled() {
        return Vec::new();
    }
    with_local_lib_read(|lib| lib.pins.clone())
}

pub fn set_local_pin(key: &str, pinned: bool) -> Result<(), String> {
    mutate_local_lib(|lib| {
        lib.set_pin(key, pinned);
        true
    })
}

/// Читает списки с диска и запускает фоновое обновление FckCensor и каталога Tune.
pub fn start() {
    read_own();
    read_local_library();
    let (at, mut mirror) = read_cache_into(cache_path(), &FCK_RAW, MIRRORS.len()).unwrap_or((0, 0));
    let (_cat_at, mut cat_mirror) =
        read_cache_into(catalog_cache_path(), &CATALOG_RAW, CATALOG_MIRRORS.len()).unwrap_or((0, 0));
    rebuild_lists();

    std::thread::spawn(move || {
        let mut wait = Duration::from_secs(REFRESH.as_secs().saturating_sub(now().saturating_sub(at)));
        loop {
            std::thread::sleep(wait);
            wait = match fetch_from(&MIRRORS, mirror, &FCK_RAW, cache_path(), false) {
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

    std::thread::spawn(move || {
        let mut wait = Duration::ZERO;
        loop {
            std::thread::sleep(wait);
            wait = match fetch_from(
                &CATALOG_MIRRORS,
                cat_mirror,
                &CATALOG_RAW,
                catalog_cache_path(),
                true,
            ) {
                Ok(used) => {
                    cat_mirror = used;
                    CATALOG_REFRESH
                }
                Err(e) => {
                    crate::api::log(&format!("Каталог Tune не скачался: {e}"));
                    CATALOG_RETRY
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
            { "tracks": { "1": { "title": "Без цензуры", "artists": [] } }, "albums": { "7": { "title": "Альбом", "coverUri": "c/%%", "volumes": [[1, { "id": 2 }]] } }, "artistsInsertions": { "5": { "tracks": [{ "index": -1, "releaseId": "1" }, 2], "albums": ["7"] } } },
            { "tracks": { "1": { "title": "Второй источник" } }, "tracksStorages": [
                { "urlTemplate": "https://example.org/%%.mp3", "trackIds": [1, { "id": 3, "durationMs": 1000 }] },
                { "urlTemplate": "https://example.org/f/%%.mp3", "tracks": { "2": 55, "4": "https://other.org/4.mp3" } }
            ] }
        ] });
        let parsed = parse_sources(&[&list], 0);
        assert_eq!(parsed.tracks["1"]["title"], "Без цензуры");
        assert_eq!(parsed.files["1"], "https://example.org/1.mp3");
        assert_eq!(parsed.files["2"], "https://example.org/f/55.mp3");
        assert_eq!(parsed.files["4"], "https://other.org/4.mp3");
        assert_eq!(parsed.tracks["3"]["durationMs"], 1000);
        assert_eq!(parsed.tracks["2"]["coverUri"], "c/%%");
        assert_eq!(
            parsed.inserts["5"],
            (vec!["1".to_string(), "2".to_string()], vec!["7".to_string()])
        );
        assert_eq!(
            volumes_of(&json!([[1, { "id": "2" }], []])),
            Some(vec![vec!["1".to_string(), "2".to_string()]])
        );
        let mut track = json!({ "id": 1, "title": "С цензурой", "artists": [{ "id": 5, "name": "Кто-то" }], "available": false })
            .as_object()
            .unwrap()
            .clone();
        apply(&mut track, &parsed.tracks["1"]);
        assert_eq!(
            (track["title"].as_str(), track["artists"][0]["name"].as_str()),
            (Some("Без цензуры"), Some("Кто-то"))
        );
    }

    #[test]
    fn merges_tune_catalog_and_searches_synthetic_entities() {
        let fck = json!({ "sources": [{
            "tracks": { "1": { "title": "Из Fck" } },
            "tracksStorages": [{ "urlTemplate": "https://fck.org/%%.mp3", "trackIds": [1] }]
        }]});
        let cat = json!({ "sources": [{
            "tracks": {
                "1": { "title": "Из Tune" },
                "9000000001": { "title": "Камен", "artists": [{ "id": 90000001, "name": "Ёжик" }], "durationMs": 186000 }
            },
            "albums": {
                "900000001": { "title": "Первый альбом", "coverUri": "c/%%", "artists": [{ "id": 90000001, "name": "Ёжик" }], "volumes": [["555", "9000000001"]] }
            },
            "artists": {
                "90000001": { "name": "Ёжик", "description": "Инди" }
            },
            "playlists": {
                "9000001": { "title": "Подборка дня", "tracks": ["9000000001"] }
            },
            "artistsInsertions": {
                "90000001": { "tracks": ["9000000001"], "albums": ["900000001"] }
            },
            "tracksStorages": [{
                "urlTemplate": "https://github.com/tune-music/tune-catalog-data/releases/download/audio-1/%%.mp3",
                "tracks": {
                    "1": { "url": "1", "durationMs": 1000 },
                    "9000000001": { "url": "9000000001", "durationMs": 186000 }
                }
            }]
        }]});

        let merged = parse_merged(&cat, &fck, 0);
        assert_eq!(merged.tracks["1"]["title"], "Из Tune");
        assert_eq!(
            merged.files["1"],
            "https://github.com/tune-music/tune-catalog-data/releases/download/audio-1/1.mp3"
        );
        assert_eq!(
            merged.track_albums.get("9000000001"),
            Some(&("900000001".to_string(), "Первый альбом".to_string()))
        );

        set_enabled(true);
        set_catalog_enabled(true);
        let ver_before = version();
        *CATALOG_RAW.write().unwrap() = Some((cat, 0));
        *FCK_RAW.write().unwrap() = Some((fck, 0));
        rebuild_lists();
        assert!(version() > ver_before);

        assert_eq!(search_tracks("камен").len(), 1);
        assert_eq!(search_tracks("ежик").len(), 1);
        assert_eq!(search_albums("первый").len(), 1);
        assert_eq!(search_artists("ежик").len(), 1);
        assert_eq!(search_playlists("подборка").len(), 1);
        assert!(track_matches_query("9000000001", "камен"));
        assert!(!track_matches_query("9000000001", "несуществующий"));

        // Значок «T» только у треков с файлом в каталоге Tune, замок — только у FckCensor;
        // родные треки Яндекса в объединённом альбоме («555») не получают «T»
        assert!(is_catalog_track("9000000001"));
        assert!(!has("9000000001"));
        assert!(is_catalog_track("1"));
        assert!(!has("1"));
        assert!(!is_catalog_track("555"));
        assert!(!has("555"));

        set_catalog_enabled(false);
        assert!(search_tracks("камен").is_empty());
        assert!(!is_catalog_track("1"));
        assert!(has("1"));

        set_catalog_enabled(true);
        set_enabled(false);
        assert_eq!(search_tracks("камен").len(), 1);
        assert!(is_catalog_track("1"));
        assert!(!has("1"));
        set_enabled(true);

        let (artist_val, t_ids, a_ids) = catalog_artist_json("90000001").unwrap();
        assert_eq!(artist_val["name"], "Ёжик");
        assert_eq!(t_ids, vec!["9000000001".to_string()]);
        assert_eq!(a_ids, vec!["900000001".to_string()]);
    }

    #[test]
    fn local_library_ops_and_roundtrip() {
        let mut lib = LocalLibrary::default();
        lib.set_track_liked("9000000001", true);
        lib.set_track_liked("9000000002", true);
        assert_eq!(lib.liked_tracks, vec!["9000000002", "9000000001"]);
        lib.set_track_disliked("9000000002", true);
        assert_eq!(lib.liked_tracks, vec!["9000000001"]);
        assert_eq!(lib.disliked_tracks, vec!["9000000002"]);

        lib.set_artist_liked("90000001", true);
        lib.set_artist_disliked("90000001", true);
        assert!(lib.liked_artists.is_empty());
        assert_eq!(lib.disliked_artists, vec!["90000001"]);

        lib.add_playlist_track("u:1", "9000000001");
        lib.add_playlist_track("u:1", "9000000002");
        assert_eq!(
            lib.playlist_tracks.get("u:1").unwrap(),
            &vec!["9000000002".to_string(), "9000000001".to_string()]
        );
        lib.move_playlist_track("u:1", "9000000002", 0, 1);
        assert_eq!(
            lib.playlist_tracks.get("u:1").unwrap(),
            &vec!["9000000001".to_string(), "9000000002".to_string()]
        );
        lib.remove_playlist_track("u:1", "9000000001");
        assert_eq!(
            lib.playlist_tracks.get("u:1").unwrap(),
            &vec!["9000000002".to_string()]
        );

        lib.set_pin("album:900000001", true);
        let restored = LocalLibrary::from_value(&lib.to_value());
        assert_eq!(restored, lib);
    }

    #[test]
    fn sync_shared_test_vector_and_codec() {
        let ts = 1_795_795_000u64;
        assert_eq!(encode_sync_ts(ts), "1l17w8");
        assert_eq!(decode_sync_ts("1l17w8"), Some(ts));
        assert_eq!(encode_sync_ts(1_795_790_288), "1l149c");

        let mut lib = LocalLibrary::default();
        lib.set_track_liked_at("9000000046", true, ts);
        lib.set_album_liked_at("900000001", true, ts);
        lib.set_artist_liked_at("90000002", true, ts);
        lib.add_playlist_track_at("12345:1005", "9000000004", ts);
        lib.add_playlist_track_at("12345:1005", "9000000046", ts);

        let payload = lib.encode_payload(Some("12345"), ts);
        assert_eq!(
            payload,
            "L:+1a@1l17w8;A:+1@1l17w8;R:+2@1l17w8;P:1005@1l17w8=1a.4"
        );

        let decoded = LocalLibrary::decode_payload(&payload, Some("12345")).unwrap();
        assert_eq!(decoded.liked_tracks, vec!["9000000046"]);
        assert_eq!(decoded.liked_albums, vec!["900000001"]);
        assert_eq!(decoded.liked_artists, vec!["90000002"]);
        assert_eq!(
            decoded.playlist_tracks.get("12345:1005").unwrap(),
            &vec!["9000000046".to_string(), "9000000004".to_string()]
        );

        // Полный набор секций (включая D, B, Q, N и fallback `~` для не-синтетических ID) + очистка >30д
        let mut full = LocalLibrary::default();
        full.set_track_liked_at("555", true, ts);
        full.set_track_liked_at("9000000001", false, ts - SYNC_TOMBSTONE_TTL_SECS - 1);
        full.set_track_disliked_at("9000000003", true, ts);
        full.set_artist_disliked_at("90000005", true, ts);
        full.set_playlist_liked_at("9000007", true, ts);
        full.set_pin_at("album:900000001", true, ts);
        full.set_pin_at("artist:90000002", true, ts - 1);
        full.set_pin_at("playlist:tune-catalog:9000003", true, ts - 2);
        full.set_pin_at("track:9000000010", false, ts - 3);
        let full_wire = full.encode_payload(Some("u"), ts);
        assert!(full_wire.contains("L:+~555@1l17w8"));
        assert!(!full_wire.contains("9000000001"));
        assert!(!full_wire.contains("+1@"));
        let decoded_full = LocalLibrary::decode_payload(&full_wire, Some("u")).unwrap();
        assert_eq!(decoded_full.liked_tracks, vec!["555"]);
        assert_eq!(decoded_full.disliked_tracks, vec!["9000000003"]);
        assert_eq!(decoded_full.disliked_artists, vec!["90000005"]);
        assert_eq!(decoded_full.liked_playlists, vec!["9000007"]);
        assert_eq!(
            decoded_full.pins,
            vec![
                "album:900000001",
                "artist:90000002",
                "playlist:tune-catalog:9000003"
            ]
        );
    }

    #[test]
    fn sync_multishard_split_reassembly_and_partial_safety() {
        let ts = 1_795_795_000u64;
        let mut lib = LocalLibrary::default();
        for i in 1..=120u64 {
            lib.set_track_liked_at(&(9_000_000_000 + i).to_string(), true, ts - i);
        }
        let payload = lib.encode_payload(Some("u"), ts);
        let descriptions = split_sync_descriptions(&payload);
        assert!(descriptions.len() >= 3);
        for (idx, desc) in descriptions.iter().enumerate() {
            assert!(
                desc.chars().count() <= SYNC_DESCRIPTION_MAX_CHARS,
                "shard {idx} len {} > 490",
                desc.chars().count()
            );
            assert!(desc.contains(&format!("[tune-sync:v1:{}/{}:", idx + 1, descriptions.len())));
        }

        let shards: Vec<SyncShardInfo> = descriptions
            .iter()
            .enumerate()
            .map(|(idx, desc)| SyncShardInfo {
                kind: (100 + idx).to_string(),
                title: sync_playlist_title(idx),
                description: desc.clone(),
                has_custom_cover: true,
            })
            .collect();

        match parse_sync_shards(&shards) {
            ParsedShards::Complete { primary_payload, .. } => {
                assert_eq!(primary_payload, payload);
            }
            other => panic!("expected Complete, got {other:?}"),
        }

        // Пропущен второй фрагмент -> PartialOrCorrupted, не должно перезаписать локальное состояние
        let partial = vec![shards[0].clone(), shards[2].clone()];
        assert!(matches!(
            parse_sync_shards(&partial),
            ParsedShards::PartialOrCorrupted { .. }
        ));

        // Дубликат шард-индекса + лишний устаревший шард -> сохраняет наименьший kind, сливает 1/1
        let dup_shards = vec![
            SyncShardInfo {
                kind: "205".into(),
                title: "Tune Sync".into(),
                description: format!("{}\n\n[tune-sync:v1:1/1:L:+2@1l17w8]", sync_header(1)),
                has_custom_cover: false,
            },
            SyncShardInfo {
                kind: "101".into(),
                title: "Tune Sync".into(),
                description: format!("{}\n\n[tune-sync:v1:1/1:L:+1@1l17w8]", sync_header(1)),
                has_custom_cover: true,
            },
            SyncShardInfo {
                kind: "300".into(),
                title: "Tune Sync 2".into(),
                description: format!("{}\n\n[tune-sync:v1:2/2:rest]", sync_header(2)),
                has_custom_cover: true,
            },
        ];
        match parse_sync_shards(&dup_shards) {
            ParsedShards::Complete {
                primary_payload,
                extra_payloads,
                canonical_kinds,
                duplicates_to_delete,
                extra_kinds_to_delete,
                ..
            } => {
                assert_eq!(canonical_kinds.get(&0).map(String::as_str), Some("101"));
                assert_eq!(primary_payload, "L:+1@1l17w8");
                assert_eq!(extra_payloads, vec!["L:+2@1l17w8".to_string()]);
                assert_eq!(duplicates_to_delete, vec!["205".to_string()]);
                assert_eq!(extra_kinds_to_delete, vec!["300".to_string()]);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn sync_lww_conflict_merge_and_tie_breakers() {
        let mut a = LocalLibrary::default();
        a.set_track_liked_at("9000000010", true, 1_700_000_010);
        a.set_track_liked_at("9000000012", true, 1_700_000_011); // Z @ 11
        a.set_track_liked_at("9000000013", false, 1_700_000_020); // tie test - @ 20
        a.add_playlist_track_at("u:1005", "9000000001", 1_700_000_010);

        let mut b = LocalLibrary::default();
        b.set_track_liked_at("9000000011", true, 1_700_000_012); // Y @ 12
        b.set_track_liked_at("9000000012", false, 1_700_000_015); // removes Z @ 15 > 11
        b.set_track_liked_at("9000000013", true, 1_700_000_020); // tie test + @ 20 wins
        b.add_playlist_track_at("u:1005", "9000000001", 1_700_000_010);
        b.remove_playlist_track_at("u:1005", "9000000001", 1_700_000_025); // cleared @ 25

        let (lww_changed, active_changed) = a.merge_from(&b);
        assert!(lww_changed && active_changed);
        assert_eq!(
            a.liked_tracks,
            vec!["9000000013", "9000000011", "9000000010"]
        );
        assert!(!a.liked_tracks.contains(&"9000000012".to_string()));
        assert!(!a.playlist_tracks.contains_key("u:1005"));
        assert!(a.encode_payload(Some("u"), 1_700_000_030).contains("P:1005@p="));
    }

    #[test]
    fn sync_playlist_detection_and_legacy_migration() {
        assert!(is_sync_playlist("Tune Sync", ""));
        assert!(is_sync_playlist("Tune Sync 2", ""));
        assert!(is_sync_playlist("Tune Sync 20", ""));
        assert!(is_sync_playlist("Другое имя", "Текст [tune-sync:v1:1/1:L:+1@0]"));
        assert!(!is_sync_playlist("Tune Sync 1", ""));
        assert!(!is_sync_playlist("Tune Sync 0", ""));
        assert!(!is_sync_playlist("Tune Sync 10", ""));
        assert!(!is_sync_playlist("Мой плейлист", "Обычное описание"));

        let legacy = json!({
            "likedTracks": ["9000000046", "9000000040", "9000000027"],
            "likedAlbums": ["900000001"],
            "playlistTracks": { "u:1005": ["9000000046", "9000000004"] }
        });
        let (migrated_lib, migrated) = LocalLibrary::from_value_at(&legacy, 1_795_795_000);
        assert!(migrated);
        assert_eq!(
            migrated_lib.liked_tracks,
            vec!["9000000046", "9000000040", "9000000027"]
        );
        assert_eq!(
            migrated_lib.liked_tracks_lww["9000000046"].ts,
            1_795_795_000
        );
        assert_eq!(
            migrated_lib.liked_tracks_lww["9000000040"].ts,
            1_795_794_999
        );
        assert_eq!(
            migrated_lib.liked_tracks_lww["9000000027"].ts,
            1_795_794_998
        );
        let roundtrip = LocalLibrary::from_value_at(&migrated_lib.to_value(), 1_795_795_999);
        assert!(!roundtrip.1);
        assert_eq!(roundtrip.0, migrated_lib);
    }
}
