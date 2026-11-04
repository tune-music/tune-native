//! Яндекс Музыка: токен, поиск, библиотека, волна и ссылка на файл трека.
//!
//! Те же запросы делает оригинальный клиент. Данные приложения общеизвестны, их используют
//! открытые клиенты (MarshalX/yandex-music-api).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

const API: &str = "https://api.music.yandex.net";
const CLIENT: &str = "YandexMusicDesktopAppWindows/5.120.0";
const SIGN_KEY: &[u8] = b"kzqU4XhfCaY6B6JTHODeq5";
const CODECS: [&str; 7] = ["flac", "flac-mp4", "mp3", "aac", "aac-mp4", "he-aac", "he-aac-mp4"];
const TRANSPORT: &str = "encraw";
const WAVE_SEED: &str = "user:onyourwave";
const FROM: &str = "tune-native";

/// Ошибка: токен отозвали или он истёк.
pub const NEED_LOGIN: &str = "NEED_LOGIN";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artists: String,
    pub duration: Duration,
    pub cover: Option<String>,
    pub album: Option<String>,
    pub album_title: String,
    /// Имя и номер каждого исполнителя.
    pub artist_list: Vec<(String, String)>,
    /// С ненормативной лексикой.
    pub explicit: bool,
    /// У Яндекса есть текст песни.
    pub lyrics: bool,
    /// Пачка волны, из которой пришёл трек.
    pub batch: Option<String>,
    /// Яндекс не даёт его слушать: нет прав или трек убран.
    pub unavailable: bool,
    /// Точки плавного перехода в секундах: нарастание и затухание. Есть только у музыки.
    pub fade: Option<Fade>,
    /// Точки перехода для волны-сета: в ней треки сведены плотнее, чем обычно.
    pub mix_fade: Option<Fade>,
    /// Выпуск подкаста или глава книги: Яндекс помнит, где остановились.
    pub remember: bool,
    /// Где остановились в прошлый раз, секунды.
    pub progress: Option<f32>,
    /// Когда вышел выпуск подкаста: «25 сентября 2026».
    pub published: Option<String>,
    /// Выпуск или глава дослушаны или отмечены прослушанными: место тогда сброшено в начало.
    pub finished: bool,
    /// Без подписки играет только отрывок: в оригинале у таких треков значок Плюса.
    pub plus_only: bool,
    /// Громкость по EBU R128 из ответа о треке: интегральная в LUFS и истинный пик в dBTP.
    pub loudness: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fade {
    pub in_start: f32,
    pub in_stop: f32,
    pub out_start: f32,
    pub out_stop: f32,
}

impl Fade {
    pub fn parse(v: &Value) -> Option<Fade> {
        let point = |key: &str| v[key].as_f64().map(|x| x as f32).filter(|x| x.is_finite() && *x >= 0.0);
        let fade = Fade { in_start: point("inStart")?, in_stop: point("inStop")?, out_start: point("outStart")?, out_stop: point("outStop")? };
        (fade.in_stop >= fade.in_start && fade.out_stop > fade.out_start).then_some(fade)
    }
}

impl Track {
    pub fn share_link(&self) -> String {
        match &self.album {
            Some(album) => format!("https://music.yandex.ru/album/{album}/track/{}", self.id),
            None => format!("https://music.yandex.ru/track/{}", self.id),
        }
    }

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
    pub login: String,
    pub email: String,
    /// Адрес аватарки. У аккаунта без своей картинки его нет.
    pub avatar: Option<String>,
    /// До какого дня действует подписка, в виде ДД.ММ.ГГГГ.
    pub plus_until: Option<String>,
    /// Подписка получена от владельца семейной группы.
    pub family: bool,
}

impl Playlist {
    pub fn album(id: &str, title: &str) -> Self {
        Self { source: Source::Album, owner: String::new(), kind: id.to_string(), title: title.to_string(), count: 1, cover: None }
    }

    pub fn artist(id: &str, title: &str) -> Self {
        Self { source: Source::Artist, owner: String::new(), kind: id.to_string(), title: title.to_string(), count: 1, cover: None }
    }
}

/// Откуда берутся треки списка.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Playlist,
    Album,
    Artist,
}

#[derive(Clone, Debug, PartialEq)]
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
    /// Что он открывает и включает.
    pub target: PinTarget,
    pub title: String,
    pub subtitle: String,
    pub cover: Option<String>,
    pub round: bool,
    /// Цвет под картинкой волны: сама картинка прозрачная.
    pub color: Option<String>,
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
    /// Кбит/с; ноль, если Яндекс не сказал.
    pub bitrate: u64,
    /// Качество, которое Яндекс дал на самом деле: lossless он отдаёт не всем и не всегда.
    pub quality: String,
}

/// Что играет: подпись и качество файла.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub label: String,
    pub quality: String,
}

impl Sound {
    /// Яндекс дал хуже, чем просили.
    pub fn below(&self, asked: &str) -> bool {
        let rank = |quality: &str| ["lq", "nq", "hq", "lossless"].iter().position(|known| *known == quality);
        matches!((rank(&self.quality), rank(asked)), (Some(got), Some(asked)) if got < asked)
    }
}

impl FileInfo {
    pub fn sound(&self) -> Sound {
        Sound { label: self.label(), quality: self.quality.clone() }
    }

    /// Что слышит пользователь, например «AAC, 256 кбит/с».
    pub fn label(&self) -> String {
        match self.bitrate {
            0 => codec_name(&self.codec),
            bitrate => format!("{}, {bitrate} кбит/с", codec_name(&self.codec)),
        }
    }
}

pub fn codec_name(codec: &str) -> String {
    match codec {
        "flac" | "flac-mp4" => "FLAC".to_string(),
        "mp3" => "MP3".to_string(),
        "aac" | "aac-mp4" => "AAC".to_string(),
        "he-aac" | "he-aac-mp4" => "HE-AAC".to_string(),
        other => other.to_uppercase(),
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).http_status_as_error(false).build().into()
}

pub fn data_dir() -> PathBuf {
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
    forget_cache();
    let _ = std::fs::remove_file(token_path());
    let _ = std::fs::remove_dir_all(login_data_dir());
}

/// Сколько живёт ответ сервера: повторный заход на страницу не должен идти в сеть.
const CACHE_TIME: Duration = Duration::from_secs(120);
const CACHE_SIZE: usize = 64;
/// Паузы перед повтором, когда Яндекс просит подождать.
const BUSY_WAITS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
/// Сколько блоков главной грузить одновременно.
const HOME_PARALLEL: usize = 4;
const BUSY: &str = "Яндекс просит подождать: слишком много запросов";

#[derive(Default)]
struct Cache {
    entries: HashMap<String, (Instant, Value)>,
}

impl Cache {
    fn get(&self, key: &str, now: Instant) -> Option<Value> {
        self.entries.get(key).filter(|(at, _)| now.duration_since(*at) < CACHE_TIME).map(|(_, value)| value.clone())
    }

    fn put(&mut self, key: String, value: Value, now: Instant) {
        self.entries.retain(|_, (at, _)| now.duration_since(*at) < CACHE_TIME);
        if self.entries.len() >= CACHE_SIZE {
            if let Some(oldest) = self.entries.iter().min_by_key(|(_, (at, _))| *at).map(|(key, _)| key.clone()) {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (now, value));
    }
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

/// Любое изменение на сервере делает сохранённые ответы устаревшими.
fn forget_cache() {
    CACHE.lock().unwrap().entries.clear();
}

/// История и ссылки на файлы всегда нужны свежими.
fn cacheable(path: &str) -> bool {
    !path.starts_with("music-history") && !path.contains("file-info") && !path.starts_with("rotor") && !path.ends_with("/lyrics")
}

fn get(path: &str, query: &[(&str, &str)], token: &str) -> Result<Value, String> {
    let key = format!("{path}?{query:?}");
    if cacheable(path) {
        if let Some(value) = CACHE.lock().unwrap().get(&key, Instant::now()) {
            return Ok(value);
        }
    }
    let mut waits = BUSY_WAITS.iter();
    let value = loop {
        match fetch(path, query, token) {
            Err(e) if e == BUSY => match waits.next() {
                Some(wait) => std::thread::sleep(*wait),
                None => return Err(e),
            },
            other => break other?,
        }
    };
    if cacheable(path) {
        CACHE.lock().unwrap().put(key, value.clone(), Instant::now());
    }
    Ok(value)
}

fn fetch(path: &str, query: &[(&str, &str)], token: &str) -> Result<Value, String> {
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
    if status == 429 {
        return Err(BUSY.into());
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
    // Отзывы волны и отчёты о прослушивании ничего из сохранённого не меняют.
    if !path.starts_with("rotor") && !path.starts_with("plays") && !path.starts_with("availability") && !path.starts_with("streams/progress/sync") {
        forget_cache();
    }
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
    if status == 429 {
        return Err(BUSY.into());
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

fn parse_artist_list(v: &Value) -> Vec<(String, String)> {
    v.as_array().into_iter().flatten().filter_map(|a| Some((a["name"].as_str()?.to_string(), id_text(&a["id"])?))).collect()
}

/// У загруженных пользователем треков исполнители — одна строка «А, Б, В», а номер
/// один на всех или его нет. Таких разбиваем по именам; пустой номер значит «искать по имени».
fn parse_track_artists(v: &Value) -> Vec<(String, String)> {
    if v["trackSource"].as_str() != Some("UGC") {
        return parse_artist_list(&v["artists"]);
    }
    let line = v["artists"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect::<Vec<_>>().join(", ");
    let names: Vec<&str> = line.split(", ").filter(|name| !name.is_empty()).collect();
    let single = id_text(&v["artists"][0]["id"]).filter(|_| names.len() == 1);
    names.into_iter().map(|name| (name.to_string(), single.clone().unwrap_or_default())).collect()
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
        album_title: v["albums"][0]["title"].as_str().unwrap_or_default().to_string(),
        artist_list: parse_track_artists(v),
        explicit: v["contentWarning"].as_str() == Some("explicit"),
        lyrics: v["lyricsInfo"]["hasAvailableSyncLyrics"].as_bool() == Some(true) || v["lyricsInfo"]["hasAvailableTextLyrics"].as_bool() == Some(true),
        batch: None,
        unavailable: v["available"].as_bool() == Some(false),
        plus_only: v["availableFullWithoutPermission"].as_bool() == Some(false),
        loudness: v["r128"]["i"].as_f64().zip(v["r128"]["tp"].as_f64()).map(|(i, tp)| (i as f32, tp as f32)),
        // Как в оригинале: переход только между музыкальными треками, не подкастами и книгами.
        fade: Fade::parse(&v["fade"]).filter(|_| v["type"].as_str() == Some("music")),
        mix_fade: Fade::parse(&v["mixFade"]).filter(|_| v["type"].as_str() == Some("music")),
        remember: v["rememberPosition"].as_bool() == Some(true),
        progress: v["streamProgress"]["endPositionSec"].as_f64().map(|x| x as f32),
        published: v["pubDate"].as_str().and_then(release_date),
        finished: v["streamProgress"]["everFinished"].as_bool() == Some(true),
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

/// Прошёл ли аккаунт мастер первого запуска: иначе оригинал предлагает выбрать исполнителей.
pub fn wizard_passed(token: &str) -> Result<bool, String> {
    let v = get("account/settings", &[], token)?;
    Ok(result(&v)["wizardIsPassed"].as_bool().unwrap_or(true))
}

/// Жанры мастера: номер для запроса и название, первым «Микс» — всё вместе.
pub fn wizard_genres(token: &str) -> Result<Vec<(String, String)>, String> {
    let v = get("feed/wizard2-new/get-genres", &[], token)?;
    Ok(result(&v)["genres"].as_array().into_iter().flatten().filter_map(|g| Some((g["id"].as_str()?.to_string(), g["title"].as_str()?.to_string()))).collect())
}

/// Ещё исполнители жанра для мастера, кроме уже показанных.
pub fn wizard_artists(genre: &str, showed: &[String], liked: &[String], count: usize, token: &str) -> Result<Vec<Card>, String> {
    let ids = |list: &[String]| list.iter().filter_map(|id| id.parse::<u64>().ok()).collect::<Vec<_>>();
    let body = json!({ "genre": genre, "showedArtists": ids(showed), "likedArtists": ids(liked), "unlikedArtists": [], "countOfNewArtists": count });
    let v = post("feed/wizard2-new/get-artists-by-genre", "application/json", body.to_string(), token)?;
    Ok(result(&v)["artists"].as_array().into_iter().flatten().filter_map(artist_card).collect())
}

/// Завершает мастер: выбранные исполнители уходят в рекомендации, мастер больше не предлагается.
pub fn wizard_finish(genre: &str, liked: &[String], token: &str) -> Result<(), String> {
    let ids: Vec<u64> = liked.iter().filter_map(|id| id.parse().ok()).collect();
    let body = json!({ "genre": genre, "likedArtists": ids, "unlikedArtists": [] });
    post("feed/wizard2-new/finish", "application/json", body.to_string(), token).map(|_| ())
}

/// Включено ли «Меньше ИИ» в аккаунте.
pub fn ai_reduction(token: &str) -> Result<bool, String> {
    let v = get("account/settings", &[], token)?;
    Ok(result(&v)["aiContentReductionEnabled"].as_bool().unwrap_or(false))
}

pub fn set_ai_reduction(enabled: bool, token: &str) -> Result<(), String> {
    post(&format!("account/settings?aiContentReductionEnabled={enabled}"), "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

pub fn profile(token: &str) -> Result<Profile, String> {
    let v = get("account/status", &[], token)?;
    let v = result(&v);
    let account = &v["account"];
    let uid = account["uid"].as_u64().map(|u| u.to_string()).ok_or("Яндекс не сообщил номер аккаунта")?;
    let name = account["displayName"].as_str().or(account["firstName"].as_str()).or(account["login"].as_str()).unwrap_or_default();
    // Без этих сведений приложение работает: их показывает только окно аккаунта.
    let about = get("account/about", &[], token).unwrap_or_default();
    let about = result(&about);
    Ok(Profile {
        uid,
        name: name.to_string(),
        plus: v["plus"]["hasPlus"].as_bool().unwrap_or(false),
        login: about["login"].as_str().or(account["login"].as_str()).unwrap_or_default().to_string(),
        email: about["email"].as_str().or(v["defaultEmail"].as_str()).unwrap_or_default().to_string(),
        avatar: about["avatarId"].as_str().and_then(avatar_uri),
        plus_until: v["subscription"]["autoRenewable"][0]["expires"].as_str().and_then(short_date),
        family: account["nonOwnerFamilyMember"].as_bool().unwrap_or(false),
    })
}

/// 0/0-0 Яндекс отдаёт тем, кто не ставил себе картинку.
fn avatar_uri(id: &str) -> Option<String> {
    (!id.is_empty() && id != "0/0-0").then(|| format!("avatars.mds.yandex.net/get-yapic/{id}/%%"))
}

fn short_date(stamp: &str) -> Option<String> {
    let (year, month, day) = (stamp.get(0..4)?, stamp.get(5..7)?, stamp.get(8..10)?);
    Some(format!("{day}.{month}.{year}"))
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

/// Сколько найденных треков показываем при поиске внутри списка.
const FILTER_LIMIT: usize = 300;
const FILTER_PAGE: usize = 100;

fn parse_filter_ids(v: &Value) -> Vec<String> {
    result(v)["playlistTrackIds"].as_array().into_iter().flatten().filter_map(|id| id.as_str()).take(FILTER_LIMIT).map(str::to_string).collect()
}

/// Поиск внутри плейлиста на сервере. Лайки для него тоже плейлист.
pub fn search_playlist(uid: &str, kind: &str, part: &str, token: &str) -> Result<Vec<Track>, String> {
    let v = get(&format!("search-playlist/{uid}/{kind}"), &[("part", part)], token)?;
    let mut tracks = Vec::new();
    for page in parse_filter_ids(&v).chunks(FILTER_PAGE) {
        tracks.extend(tracks_by_ids(page, token)?);
    }
    Ok(tracks)
}

fn parse_credits(v: &Value) -> Vec<(String, String)> {
    let items = result(v)["credits"].as_array().into_iter().flatten();
    items.filter_map(|item| Some((item["title"].as_str()?.to_string(), item["value"].as_str().filter(|value| !value.is_empty())?.to_string()))).collect()
}

/// Сведения о треке: исполнитель, авторы, лейбл.
/// Окно «Сведения о треке»: строки сведений и похожие треки.
#[derive(Clone, Debug, Default)]
pub struct TrackDetails {
    pub facts: Vec<(String, String)>,
    pub similar: Vec<Track>,
}

/// Русское название жанра по коду из ответа трека, например rock → «Рок».
fn genre_title(genres: &Value, code: &str) -> Option<String> {
    let list = genres.as_array()?;
    list.iter().chain(list.iter().flat_map(|genre| genre["subGenres"].as_array().into_iter().flatten())).find(|genre| genre["id"].as_str() == Some(code)).and_then(|genre| {
        genre["titles"]["ru"]["title"].as_str().or(genre["title"].as_str()).map(str::to_string)
    })
}

fn parse_details(full: &Value, genres: &Value, credits: Vec<(String, String)>) -> TrackDetails {
    let track = &full["track"];
    let album = &track["albums"][0];
    let mut facts = Vec::new();
    if let Some(genre) = album["genre"].as_str().and_then(|code| genre_title(genres, code)) {
        facts.push(("Жанр".to_string(), genre));
    }
    if let Some(year) = album["year"].as_u64() {
        facts.push(("Год".to_string(), year.to_string()));
    }
    if let Some(version) = track["version"].as_str().map(str::trim).filter(|version| !version.is_empty()) {
        facts.push(("Издание".to_string(), version.to_string()));
    }
    facts.extend(credits);
    TrackDetails { facts, similar: full["similarTracks"].as_array().into_iter().flatten().filter_map(parse_track).collect() }
}

/// Авторы — главное в окне, остальное дополняет их и может не прийти.
pub fn track_details(id: &str, token: &str) -> Result<TrackDetails, String> {
    let credits = parse_credits(&get(&format!("tracks/{id}/credits"), &[], token)?);
    let full = get(&format!("tracks/{id}/full-info"), &[], token).map(|v| result(&v).clone()).unwrap_or(Value::Null);
    let genres = get("genres", &[], token).map(|v| result(&v).clone()).unwrap_or(Value::Null);
    Ok(parse_details(&full, &genres, credits))
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
            Some(Pin { target: PinTarget::List(target), title, subtitle: "Плейлист".to_string(), cover, round: false, color: None })
        }
        "wave_item" => {
            let seeds = data["seeds"].as_array()?.iter().filter_map(|s| s.as_str().map(str::to_string)).collect();
            let cover = [&data["backgroundImageUrl"], &data["agent"]["cover"]["uri"]].iter().find_map(|uri| uri.as_str().filter(|uri| !uri.is_empty())).map(str::to_string);
            let title = data["title"].as_str()?.to_string();
            // Подпись говорит, по чему волна: «Моя волна по альбому» и так далее.
            let subtitle = data["header"].as_str().filter(|header| *header != title).unwrap_or("Волна").to_string();
            Some(Pin { target: PinTarget::Wave(seeds), title, subtitle, cover, round: true, color: data["colors"]["average"].as_str().map(str::to_string) })
        }
        "album_item" => {
            let title = data["title"].as_str()?;
            let cover = data["coverUri"].as_str();
            Some(Pin { target: PinTarget::List(list(Source::Album, title, cover)), title: title.to_string(), subtitle: "Альбом".to_string(), cover: cover.map(str::to_string), round: false, color: None })
        }
        "artist_item" => {
            let title = data["name"].as_str()?;
            let cover = data["cover"]["uri"].as_str().or(data["coverUri"].as_str());
            Some(Pin { target: PinTarget::List(list(Source::Artist, title, cover)), title: title.to_string(), subtitle: "Исполнитель".to_string(), cover: cover.map(str::to_string), round: true, color: None })
        }
        _ => None,
    }
}

/// Закреплённое. Ответ сохраняется: без сети показывается то, что было в прошлый раз.
pub fn pins(token: &str) -> Result<Vec<Pin>, String> {
    let saved = data_dir().join("pins.json");
    let v = match get("pins", &[], token) {
        Ok(v) => {
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(&saved, v.to_string());
            v
        }
        Err(e) if network_error(&e) => std::fs::read(&saved).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).ok_or(e)?,
        Err(e) => return Err(e),
    };
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

/// Шапка альбома.
#[derive(Clone, Debug)]
pub struct Album {
    pub id: String,
    pub title: String,
    /// Код жанра, например `rock`: по нему полка «Новые альбомы в жанре».
    pub genre: Option<String>,
    pub year: Option<u64>,
    pub artists: Vec<(String, String)>,
    pub cover: Option<String>,
    /// Сколько треков на каждом диске; у обычного альбома один диск.
    pub volumes: Vec<usize>,
    /// Пометка издания, например «Deluxe Edition».
    pub version: String,
    /// Лейблы: номер страницы лейбла, если Яндекс его дал, и название.
    pub labels: Vec<(Option<String>, String)>,
    /// Дата выхода словами: «1 января 2011».
    pub released: Option<String>,
    /// Другие издания того же альбома.
    pub versions: Vec<Card>,
    pub kind: AlbumKind,
    /// Описание подкаста или книги простым текстом.
    pub description: String,
    /// С ненормативной лексикой: на странице «18+».
    pub explicit: bool,
    /// Подкаст или книга прослушаны целиком.
    pub finished: bool,
    /// Кнопка издателя, например «Читать в Яндекс Книгах»: подпись и адрес.
    pub action: Option<(String, String)>,
}

/// Что за альбом: музыка, подкаст или аудиокнига.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AlbumKind {
    #[default]
    Music,
    Podcast,
    Audiobook,
}

impl AlbumKind {
    fn parse(v: &Value) -> AlbumKind {
        match v["type"].as_str() {
            Some("podcast") => AlbumKind::Podcast,
            Some("audiobook") => AlbumKind::Audiobook,
            _ if v["metaType"].as_str() == Some("podcast") => AlbumKind::Podcast,
            _ => AlbumKind::Music,
        }
    }
}

/// Описание от Яндекса приходит с разметкой: переносы строк остаются, теги и сущности — нет.
pub fn html_text(html: &str) -> String {
    let html = html.replace("<br/>", "\n").replace("<br />", "\n").replace("<br>", "\n").replace("</p>", "\n");
    let mut plain = String::new();
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => plain.push(c),
            _ => {}
        }
    }
    let mut out = String::new();
    let mut rest = plain.as_str();
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.char_indices().take(10).find(|&(_, c)| c == ';').map(|(at, _)| at) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let named = match name {
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "quot" => Some('"'),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "apos" => Some('\''),
            _ => name.strip_prefix('#').and_then(|code| code.parse::<u32>().ok()).and_then(char::from_u32),
        };
        match named {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    // Не больше одной пустой строки подряд.
    let mut text = String::new();
    let mut blank = 0;
    for line in out.lines().map(str::trim) {
        blank = if line.is_empty() { blank + 1 } else { 0 };
        if blank < 2 {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.trim().to_string()
}

/// Альбом или исполнитель в сетке обложек.
#[derive(Clone, Debug)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub cover: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub cover: Option<String>,
    pub listeners: u64,
    pub popular: Vec<Track>,
    pub albums: Vec<Card>,
    pub also: Vec<Card>,
    pub similar: Vec<Card>,
    /// «Об исполнителе»: текст и официальные страницы.
    pub about: String,
    pub links: Vec<ArtistLink>,
    /// «Знакомо вам»: его треки в коллекции и те, что звучали в Моей волне.
    pub familiar_collection: Vec<Track>,
    pub familiar_wave: Vec<Track>,
    /// «Новый релиз»: последний альбом или сингл.
    pub release: Option<Card>,
    /// Плейлисты исполнителя: «Лучшее», «В стиле».
    pub playlists: Vec<HomeItem>,
    /// Клипы: название, обложка и адрес плеера Яндекса.
    pub videos: Vec<Video>,
    /// Фотографии исполнителя.
    pub photos: Vec<String>,
    /// BandLink: «Плейлисты, где встречается» на других площадках.
    pub bandlink: Option<ArtistLink>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub cover: Option<String>,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArtistLink {
    pub title: String,
    pub subtitle: String,
    pub url: String,
}

/// Страница релизов исполнителя: свои или сборники, где он есть.
#[derive(Clone, Debug)]
pub struct Releases {
    pub albums: Vec<Card>,
    pub total: u64,
}

pub const RELEASES_PAGE: u64 = 100;

fn cover_uri(v: &Value) -> Option<String> {
    v["coverUri"].as_str().or(v["cover"]["uri"].as_str()).filter(|u| !u.is_empty()).map(str::to_string)
}

fn parse_album(v: &Value) -> Option<Album> {
    Some(Album {
        id: id_text(&v["id"])?,
        title: v["title"].as_str().unwrap_or_default().to_string(),
        year: v["year"].as_u64(),
        artists: parse_artist_list(&v["artists"]),
        cover: cover_uri(v),
        volumes: Vec::new(),
        version: v["version"].as_str().unwrap_or_default().trim().to_string(),
        genre: v["genre"].as_str().filter(|genre| !genre.is_empty()).map(str::to_string),
        labels: v["labels"].as_array().into_iter().flatten().filter_map(|label| Some((id_text(&label["id"]), label["name"].as_str().or(label.as_str())?.to_string()))).collect(),
        released: v["releaseDate"].as_str().and_then(release_date),
        versions: parse_cards(&v["duplicates"], version_card),
        kind: AlbumKind::parse(v),
        description: html_text(v["description"].as_str().unwrap_or_default()),
        explicit: v["contentWarning"].as_str() == Some("explicit"),
        finished: v["listeningFinished"].as_bool() == Some(true),
        action: v["actionButton"]["text"].as_str().zip(v["actionButton"]["url"].as_str()).filter(|(_, url)| url.starts_with("https://")).map(|(label, url)| (label.to_string(), url.to_string())),
    })
}

/// «2011-01-01T00:00:00+03:00» → «1 января 2011».
fn release_date(iso: &str) -> Option<String> {
    const MONTHS: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
    let mut parts = iso.get(..10)?.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?.parse::<usize>().ok()?, parts.next()?.parse::<u32>().ok()?);
    Some(format!("{day} {} {year}", MONTHS.get(month.checked_sub(1)?)?))
}

/// Издание альбома: подпись — его пометка, а если её нет, то год.
fn version_card(v: &Value) -> Option<Card> {
    let mut card = album_card(v)?;
    if let Some(version) = v["version"].as_str().map(str::trim).filter(|version| !version.is_empty()) {
        card.subtitle = version.to_string();
    }
    Some(card)
}

fn parse_cards(v: &Value, card: impl Fn(&Value) -> Option<Card>) -> Vec<Card> {
    v.as_array().into_iter().flatten().filter_map(card).collect()
}

fn album_card(v: &Value) -> Option<Card> {
    Some(Card {
        id: id_text(&v["id"])?,
        title: v["title"].as_str()?.to_string(),
        subtitle: album_subtitle(v),
        cover: cover_uri(v),
    })
}

/// Год и вид релиза, как на карточках официального клиента: «2020 · Сингл».
fn album_subtitle(v: &Value) -> String {
    // На лендинге книг и подкастов вид альбома лежит в albumType.
    let kind = match v["type"].as_str().or(v["albumType"].as_str()) {
        Some("single") => Some("Сингл"),
        Some("compilation") => Some("Сборник"),
        Some("podcast") => Some("Подкаст"),
        Some("audiobook") => Some("Аудиокнига"),
        _ => None,
    };
    match (v["year"].as_u64(), kind) {
        (Some(year), Some(kind)) => format!("{year} · {kind}"),
        (Some(year), None) => year.to_string(),
        (None, Some(kind)) => kind.to_string(),
        (None, None) => "Альбом".to_string(),
    }
}

fn artist_card(v: &Value) -> Option<Card> {
    Some(Card { id: id_text(&v["id"])?, title: v["name"].as_str()?.to_string(), subtitle: "Исполнитель".to_string(), cover: cover_uri(v) })
}

fn parse_artist(v: &Value) -> Option<Artist> {
    let artist = &v["artist"];
    Some(Artist {
        id: id_text(&artist["id"])?,
        name: artist["name"].as_str().unwrap_or_default().to_string(),
        cover: cover_uri(artist),
        listeners: v["stats"]["lastMonthListeners"].as_u64().unwrap_or(0),
        popular: v["popularTracks"].as_array().into_iter().flatten().filter_map(parse_track).collect(),
        albums: parse_cards(&v["albums"], album_card),
        also: parse_cards(&v["alsoAlbums"], album_card),
        similar: parse_cards(&v["similarArtists"], artist_card),
        about: String::new(),
        links: Vec::new(),
        familiar_collection: Vec::new(),
        familiar_wave: Vec::new(),
        release: v["lastReleases"].as_array().and_then(|list| list.first()).and_then(album_card),
        playlists: v["playlists"].as_array().into_iter().flatten().filter_map(parse_home_playlist).collect(),
        videos: v["videos"].as_array().into_iter().flatten().filter_map(|video| {
            let url = video["embedUrl"].as_str().filter(|url| url.starts_with("https://"))?;
            Some(Video {
                id: video["providerVideoId"].as_str().unwrap_or(url).to_string(),
                title: video["title"].as_str()?.to_string(),
                cover: video["cover"].as_str().map(|uri| uri.trim_start_matches("https://").to_string()),
                url: url.to_string(),
            })
        }).collect(),
        photos: v["allCovers"].as_array().into_iter().flatten().filter_map(|cover| cover["uri"].as_str().map(str::to_string)).collect(),
        bandlink: Some(&v["bandlinkScannerLink"]).filter(|link| link.is_object()).and_then(|link| {
            let url = link["url"].as_str().filter(|url| url.starts_with("https://"))?;
            Some(ArtistLink { title: link["title"].as_str()?.to_string(), subtitle: "BandLink".to_string(), url: url.to_string() })
        }),
    })
}

fn parse_about(v: &Value) -> (String, Vec<ArtistLink>) {
    let links = v["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|link| {
            let url = link["url"].as_str().filter(|url| url.starts_with("https://") || url.starts_with("http://"))?;
            Some(ArtistLink { title: link["title"].as_str()?.to_string(), subtitle: link["subtitle"].as_str().unwrap_or_default().to_string(), url: url.to_string() })
        })
        .collect();
    (v["description"].as_str().unwrap_or_default().trim().to_string(), links)
}

/// Свои релизы (`direct-albums`) или сборники с исполнителем (`also-albums`).
pub fn releases(id: &str, also: bool, by_year: bool, page: u64, token: &str) -> Result<Releases, String> {
    let path = format!("artists/{id}/{}", if also { "also-albums" } else { "direct-albums" });
    let (page, size) = (page.to_string(), RELEASES_PAGE.to_string());
    let v = get(&path, &[("page", &page), ("pageSize", &size), ("sort-by", if by_year { "year" } else { "rating" }), ("sort-order", "desc")], token)?;
    let v = result(&v);
    Ok(Releases { albums: parse_cards(&v["albums"], album_card), total: v["pager"]["total"].as_u64().unwrap_or(0) })
}

pub fn album(id: &str, token: &str) -> Result<(Album, Vec<Track>), String> {
    // Как в официальном клиенте: у выпусков и глав приходят место, где остановились, и дата выхода.
    let v = get(&format!("albums/{id}/with-tracks"), &[("resumeStream", "true"), ("richTracks", "true"), ("withListeningFinished", "true")], token)?;
    let v = result(&v);
    let mut disks: Vec<Vec<Track>> = v["volumes"].as_array().into_iter().flatten().map(|disk| disk.as_array().into_iter().flatten().filter_map(parse_track).collect()).collect();
    // Подкасты приходят от новых выпусков к старым: здесь всегда от первого сезона к последнему.
    if v["sortOrder"].as_str() == Some("desc") {
        disks.reverse();
        disks.iter_mut().for_each(|disk| disk.reverse());
    }
    let mut album = parse_album(v).ok_or("Яндекс не прислал альбом")?;
    album.volumes = disks.iter().map(Vec::len).filter(|&size| size > 0).collect();
    Ok((album, disks.into_iter().flatten().collect()))
}

pub fn artist(id: &str, token: &str) -> Result<Artist, String> {
    let v = get(&format!("artists/{id}/brief-info"), &[], token)?;
    let mut artist = parse_artist(result(&v)).ok_or_else(|| "Яндекс не прислал исполнителя".to_string())?;
    // Описание не главное: без него страница всё равно откроется.
    if let Ok(v) = get(&format!("artists/{id}/about-artist"), &[], token) {
        (artist.about, artist.links) = parse_about(result(&v));
    }
    let limit = FAMILIAR_LIMIT.to_string();
    if let Ok(v) = get(&format!("artists/{id}/familiar-you"), &[("waveTracksLimit", &limit), ("collectionTracksLimit", &limit), ("collectionAlbumsLimit", "0")], token) {
        (artist.familiar_collection, artist.familiar_wave) = parse_familiar(result(&v));
    }
    Ok(artist)
}

/// Сколько знакомых треков просится в каждой части.
const FAMILIAR_LIMIT: usize = 50;

fn parse_familiar(v: &Value) -> (Vec<Track>, Vec<Track>) {
    let tracks = |key: &str| v[key]["tracks"].as_array().into_iter().flatten().filter_map(parse_track).collect();
    (tracks("collection"), tracks("wave"))
}

/// Что нашёл смешанный поиск.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub tracks: Vec<Track>,
    pub albums: Vec<Card>,
    pub artists: Vec<Card>,
    pub playlists: Vec<Playlist>,
    /// Волны по жанру, настроению или занятию.
    pub waves: Vec<HomeItem>,
    /// Яндекс исправил опечатку и искал вот это.
    pub corrected: Option<String>,
}

fn parse_found(v: &Value) -> Found {
    let mut found = Found { corrected: v["text"].as_str().filter(|_| v["misspellCorrected"].as_bool() == Some(true)).map(str::to_string), ..Found::default() };
    for item in v["results"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("track") => found.tracks.extend(parse_track(&item["track"])),
            Some("album") => found.albums.extend(album_card(&item["album"])),
            // Подкаст и книга — тот же альбом, только без исполнителя в подписи.
            Some("podcast") => found.albums.extend(album_card(&item["podcast"]).map(|mut card| {
                if card.subtitle.is_empty() {
                    card.subtitle = if item["podcast"]["type"].as_str() == Some("audiobook") { "Аудиокнига" } else { "Подкаст" }.to_string();
                }
                card
            })),
            Some("podcast_episode") => found.tracks.extend(parse_track(&item["podcast_episode"])),
            Some("wave") => found.waves.extend(parse_home_item(&item["wave"]).filter(|wave| matches!(wave, HomeItem::Wave { .. }))),
            Some("artist") => found.artists.extend(artist_card(&item["artist"])),
            Some("playlist") => {
                let owner = id_text(&item["playlist"]["uid"]).unwrap_or_default();
                found.playlists.extend(parse_playlist(&item["playlist"], &owner));
            }
            _ => {}
        }
    }
    found
}

/// `types` — что искать через запятую: track, album, artist, playlist, podcast, wave.
/// `filter` — вкладка, как у Яндекса: podcast, book, waves. Без неё ищется по типам.
pub fn search_mixed(text: &str, types: &str, filter: Option<&str>, page_size: u32, token: &str) -> Result<Found, String> {
    let size = page_size.to_string();
    let mut params = vec![("text", text), ("type", types), ("page", "0"), ("pageSize", &size)];
    params.extend(filter.map(|filter| ("filter", filter)));
    let v = get("search/instant/mixed", &params, token)?;
    Ok(parse_found(result(&v)))
}

/// Альбомы и исполнители, которые нравятся пользователю, и исполнители с дизлайком.
pub fn liked_pages(uid: &str, token: &str) -> Result<(Vec<Card>, Vec<Card>, Vec<Card>), String> {
    let albums = get(&format!("users/{uid}/likes/albums"), &[("rich", "true")], token)?;
    let artists = get(&format!("users/{uid}/likes/artists"), &[("with-timestamps", "false")], token)?;
    let disliked = get(&format!("users/{uid}/dislikes/artists"), &[], token)?;
    // Исполнитель приходит то сам по себе, то внутри artist.
    let artist = |item: &Value| artist_card(item).or_else(|| artist_card(&item["artist"]));
    Ok((parse_cards(result(&albums), |item| album_card(&item["album"])), parse_cards(result(&artists), artist_card), parse_cards(result(&disliked), artist)))
}

/// Дизлайк исполнителя: он больше не появится в рекомендациях. Лайк с него Яндекс снимает сам.
pub fn set_artist_disliked(uid: &str, id: &str, disliked: bool, token: &str) -> Result<(), String> {
    let path = if disliked { format!("users/{uid}/dislikes/artists/add?artist-id={id}") } else { format!("users/{uid}/dislikes/artists/{id}/remove") };
    post(&path, "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

/// `kind` — albums или artists, `param` — album-id или artist-id.
fn set_page_liked(uid: &str, kind: &str, param: &str, id: &str, liked: bool, token: &str) -> Result<(), String> {
    let path = if liked { format!("users/{uid}/likes/{kind}/add?{param}={id}") } else { format!("users/{uid}/likes/{kind}/{id}/remove") };
    post(&path, "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

pub fn set_album_liked(uid: &str, id: &str, liked: bool, token: &str) -> Result<(), String> {
    set_page_liked(uid, "albums", "album-id", id, liked, token)
}

pub fn set_artist_liked(uid: &str, id: &str, liked: bool, token: &str) -> Result<(), String> {
    set_page_liked(uid, "artists", "artist-id", id, liked, token)
}

/// Элемент блока главной.
#[derive(Clone, Debug)]
pub enum HomeItem {
    Album(Card),
    Artist(Card),
    Playlist(Playlist),
    /// У волны прозрачная картинка: под неё кладётся цвет.
    Wave { title: String, seeds: Vec<String>, cover: Option<String>, color: Option<String> },
    /// Карточки в начале главной: «Мне нравится» со счётчиком и «История» с последними треками.
    Likes { count: u64, cover: Option<String> },
    /// Микс: подборка плейлистов по тегу, например «Осенняя».
    Mix { tag: String, title: String, cover: Option<String> },
    History { subtitle: String, cover: Option<String> },
    /// Другая страница из блоков: «Жанры», жанр, настроение. У пунктов списка жанров обложки нет.
    Landing { page: String, title: String, cover: Option<String> },
    /// Подборка редакции (`/post/<id>`): альбомы, плейлисты и исполнители под одним названием.
    Post { id: String, title: String, cover: Option<String> },
}

#[derive(Clone, Debug, Default)]
pub struct HomeBlock {
    pub title: String,
    pub items: Vec<HomeItem>,
    pub tracks: Vec<Track>,
    /// Вкладки внутри блока («топ», «настроение»…) с их карточками; `items` — выбранная.
    pub tabs: Vec<(String, Vec<HomeItem>)>,
    /// Куда ведёт «Все»: полный список жанра (`/genre/<id>/albums`) или страница из блоков.
    pub more: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Home {
    /// Название страницы у Яндекса: «Жанры», «Русский рок».
    pub title: String,
    pub tabs: Vec<String>,
    pub blocks: Vec<HomeBlock>,
}

fn parse_release(v: &Value) -> Option<HomeItem> {
    let album = &v["album"];
    let names = v["artists"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect::<Vec<_>>().join(", ");
    let cover = cover_uri(album).or_else(|| cover_uri(v));
    Some(HomeItem::Album(Card { id: id_text(&album["id"])?, title: album["title"].as_str()?.to_string(), subtitle: names, cover }))
}

fn parse_home_playlist(v: &Value) -> Option<HomeItem> {
    let owner = id_text(&v["uid"]).or_else(|| id_text(&v["owner"]["uid"]))?;
    let mut list = parse_playlist_loose(v, &owner)?;
    list.owner = owner;
    Some(HomeItem::Playlist(list))
}

/// На главной номер плейлиста приходит и числом, и строкой.
fn parse_playlist_loose(v: &Value, owner: &str) -> Option<Playlist> {
    Some(Playlist {
        source: Source::Playlist,
        owner: owner.to_string(),
        kind: id_text(&v["kind"])?,
        title: v["title"].as_str()?.to_string(),
        count: v["trackCount"].as_u64().unwrap_or(0),
        cover: cover_uri(v).or(v["ogImage"].as_str().filter(|c| !c.is_empty()).map(str::to_string)),
    })
}

fn parse_home_item(v: &Value) -> Option<HomeItem> {
    // В блоках волн и «В стиле» карточка лежит сразу, без обёртки data.
    let data = if v["data"].is_object() { &v["data"] } else { v };
    if data["playlist"].is_object() {
        return parse_home_playlist(&data["playlist"]);
    }
    if data["artist"].is_object() {
        return artist_card(&data["artist"]).map(HomeItem::Artist);
    }
    if data["album"].is_object() {
        return album_card(&data["album"]).map(HomeItem::Album);
    }
    if let Some(tag) = data["action"]["weblink"].as_str().and_then(|link| link.strip_prefix("/tag/")).map(|tag| tag.trim_matches('/')).filter(|tag| !tag.is_empty()) {
        return Some(HomeItem::Mix { tag: tag.to_string(), title: data["title"].as_str()?.to_string(), cover: data["covers"][0]["uri"].as_str().map(str::to_string) });
    }
    // Категория книг и подкастов — своя страница из блоков: «Юмор», «Детективы».
    if let Some(page) = data["action"]["weblink"].as_str().and_then(|link| link.strip_prefix("/landing/")).map(|page| page.trim_matches('/')).filter(|page| page.starts_with("category_")) {
        return Some(HomeItem::Mix { tag: format!("{CATEGORY}{page}"), title: data["title"].as_str()?.to_string(), cover: data["covers"][0]["uri"].as_str().map(str::to_string) });
    }
    // Остальные страницы из блоков: «Жанры», «Настроения» в миксах и пункты списка жанров.
    let link = data["action"]["weblink"].as_str().or(data["viewAllAction"]["weblink"].as_str()).unwrap_or_default();
    let cover = || data["covers"][0]["uri"].as_str().map(str::to_string);
    if let Some(page) = link.strip_prefix("/landing/").map(|page| page.trim_matches('/')).filter(|page| !page.is_empty()) {
        return Some(HomeItem::Landing { page: page.to_string(), title: data["title"].as_str()?.to_string(), cover: cover() });
    }
    if let Some(id) = link.strip_prefix("/post/").map(|id| id.trim_matches('/')).filter(|id| !id.is_empty()) {
        return Some(HomeItem::Post { id: id.to_string(), title: data["title"].as_str()?.to_string(), cover: cover() });
    }
    // В похожем волна лежит глубже: название и зёрна в wave, обложка в agent.
    let wave = &data["wave"];
    let seeds: Vec<String> = data["seeds"].as_array().or(wave["seeds"].as_array())?.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
    if seeds.is_empty() {
        return None;
    }
    // Фон у волн общий для всех, поэтому сначала своя картинка волны.
    let cover = data["compactImageUrl"].as_str().or(data["agent"]["cover"]["uri"].as_str()).or(data["imageUrl"].as_str()).or(data["backgroundImageUrl"].as_str()).map(str::to_string).or_else(|| cover_uri(data));
    let color = data["colors"]["average"].as_str().or(data["agent"]["cover"]["color"].as_str()).map(str::to_string);
    Some(HomeItem::Wave { title: data["title"].as_str().or(wave["name"].as_str())?.to_string(), seeds, cover, color })
}

impl HomeItem {
    /// Карточка ведёт в раздел приложения, а не на страницу каталога.
    pub fn is_shortcut(&self) -> bool {
        matches!(self, HomeItem::Likes { .. } | HomeItem::History { .. })
    }
}

/// Так помечен микс, который на самом деле категория книг и подкастов.
pub const CATEGORY: &str = "category:";
/// В категории бывает больше тысячи книг: страница показывает самые первые.
const CATEGORY_LIMIT: usize = 100;

/// Карточки категории книг и подкастов: все её блоки подряд, без повторов.
pub fn category(page: &str, token: &str) -> Result<Vec<HomeItem>, String> {
    let landing = landing(page, 0, token)?;
    let mut seen = std::collections::HashSet::new();
    Ok(landing.blocks.into_iter().flat_map(|block| block.items).filter(|item| seen.insert(item_key(item))).take(CATEGORY_LIMIT).collect())
}

/// Чем карточка отличается от других: вид и номер.
fn item_key(item: &HomeItem) -> String {
    match item {
        HomeItem::Album(card) => format!("album:{}", card.id),
        HomeItem::Artist(card) => format!("artist:{}", card.id),
        HomeItem::Playlist(list) => format!("playlist:{}:{}", list.owner, list.kind),
        HomeItem::Wave { seeds, .. } => format!("wave:{}", seeds.join(",")),
        HomeItem::Mix { tag, .. } => format!("mix:{tag}"),
        HomeItem::Likes { .. } => "likes".to_string(),
        HomeItem::History { .. } => "history".to_string(),
        HomeItem::Landing { page, .. } => format!("landing:{page}"),
        HomeItem::Post { id, .. } => format!("post:{id}"),
    }
}

/// Альбомы из ответов полки и новых выпусков: они лежат то списком, то внутри блоков.
fn shelf_albums(v: &Value) -> Vec<HomeItem> {
    let mut found = Vec::new();
    let mut take = |entry: &Value| {
        let album = if entry["album"].is_object() { &entry["album"] } else if entry["data"]["album"].is_object() { &entry["data"]["album"] } else { entry };
        if let Some(card) = album_card(album) {
            found.push(HomeItem::Album(card));
        }
    };
    for key in ["albums", "items", "entities"] {
        v[key].as_array().into_iter().flatten().for_each(&mut take);
    }
    for block in v["blocks"].as_array().into_iter().flatten() {
        block["entities"].as_array().into_iter().flatten().for_each(&mut take);
    }
    let mut seen = std::collections::HashSet::new();
    found.retain(|item| seen.insert(item_key(item)));
    found
}

/// Полка книг и подкастов и новые выпуски: первыми на странице раздела. Пустые не показываются.
fn shelf_blocks(podcasts: bool, token: &str) -> Vec<HomeBlock> {
    let handles: Vec<_> = [("non-music/bookshelf/recently-played", "Продолжить слушать"), ("new-episodes", "Новые выпуски"), ("non-music/bookshelf/liked", "Вы отложили")]
        .into_iter()
        // Новые выпуски бывают только у подкастов.
        .filter(|(path, _)| podcasts || *path != "new-episodes")
        .map(|(path, title)| {
            let token = token.to_string();
            let path = if path == "new-episodes" { "non-music/new-episodes" } else { path };
            std::thread::spawn(move || get(path, &[], &token).ok().map(|v| HomeBlock { title: title.to_string(), items: shelf_albums(result(&v)), ..HomeBlock::default() }))
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok().flatten()).filter(|block| !block.items.is_empty()).collect()
}

/// Название тега и его плейлисты: сначала номера, потом сами плейлисты одним запросом.
pub fn tag_playlists(tag: &str, token: &str) -> Result<(Option<String>, Vec<Playlist>), String> {
    let v = get(&format!("tags/{tag}/playlist-ids"), &[], token)?;
    let name = result(&v)["tag"]["name"].as_str().map(str::to_string);
    let ids: Vec<String> = result(&v)["ids"].as_array().into_iter().flatten().filter_map(|id| Some(format!("{}:{}", id_text(&id["uid"])?, id_text(&id["kind"])?))).collect();
    if ids.is_empty() {
        return Ok((name, Vec::new()));
    }
    let v = get("playlists/list", &[("playlist-ids", &ids.join(","))], token)?;
    Ok((name, parse_tag_playlists(result(&v))))
}

fn parse_tag_playlists(v: &Value) -> Vec<Playlist> {
    v.as_array().into_iter().flatten().filter_map(|list| parse_playlist_loose(list, &id_text(&list["uid"]).or_else(|| id_text(&list["owner"]["uid"]))?)).collect()
}

/// Куда ведёт ссылка на Яндекс Музыку.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Album(String),
    Track(String),
    Artist(String),
    Playlist(String, String),
    PlaylistUuid(String),
    /// Страница приложения: путь из адреса оригинала, например `collection/albums`.
    Page(String),
    Search(String),
    /// Микс по тегу или категория книг и подкастов.
    Tag(String),
    /// Страница из блоков: «Жанры», жанр (`/genre/rock` — это `tag_rock`).
    Landing(String),
    /// Подборка редакции.
    Post(String),
}

/// Так помечена страница лейбла среди страниц из блоков: у Яндекса это не скелет, а свои запросы.
pub const LABEL: &str = "label:";

/// Разбирает ссылку на music.yandex.*, ссылку протокола `yandexmusic://` и внутреннюю ссылку
/// оригинала `music-application://desktop/…`. Всё остальное не ссылка, а обычный поиск.
pub fn parse_link(text: &str) -> Option<Link> {
    let text = text.trim();
    let address = match text.strip_prefix("yandexmusic://").or_else(|| text.strip_prefix("music-application://desktop")) {
        Some(path) => path,
        None => {
            let rest = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")).unwrap_or(text);
            let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
            if !host.starts_with("music.yandex.") {
                return None;
            }
            path
        }
    };
    let (path, query) = address.split_once('?').unwrap_or((address, ""));
    let path = path.split('#').next()?;
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let number = |id: &str| (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_string());
    let page = |parts: &[&str]| Some(Link::Page(parts.join("/")));
    match parts.as_slice() {
        [] | ["home"] => page(&["home"]),
        ["search", ..] => Some(query_text(query).filter(|q| !q.is_empty()).map_or(Link::Page("search".to_string()), Link::Search)),
        ["tag", tag, ..] => Some(Link::Tag(tag.to_string())),
        ["landing", landing, ..] => Some(Link::Landing(landing.to_string())),
        ["genre", genre, ..] => Some(Link::Landing(format!("tag_{genre}"))),
        ["post", post, ..] => Some(Link::Post(post.to_string())),
        ["label", label, ..] => number(label).map(|label| Link::Landing(format!("{LABEL}{label}"))),
        ["non-music", "category", category, ..] => Some(Link::Tag(format!("{CATEGORY}{category}"))),
        [first @ ("collection" | "non-music" | "kids" | "chart"), ..] => page(&parts[..parts.len().min(if *first == "collection" { 2 } else { 1 })]),
        ["album", album, ..] => number(album).map(Link::Album),
        ["track", track, ..] => number(track).map(Link::Track),
        ["artist", artist, ..] => number(artist).map(Link::Artist),
        ["users", owner, "playlists", kind, ..] => number(kind).map(|kind| Link::Playlist(owner.to_string(), kind)),
        // В ссылках на плейлисты перед номером бывает приставка.
        ["playlists", id, ..] => Some(Link::PlaylistUuid(id.strip_prefix("lk.").unwrap_or(id).to_string())),
        _ => None,
    }
}

/// Значение `text` из строки запроса: `+` — пробел, `%xx` — байт.
fn query_text(query: &str) -> Option<String> {
    let value = query.split('&').find_map(|pair| pair.strip_prefix("text="))?;
    let mut bytes = Vec::with_capacity(value.len());
    let mut rest = value.bytes();
    while let Some(byte) = rest.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let hex: String = rest.by_ref().take(2).map(char::from).collect();
                bytes.push(u8::from_str_radix(&hex, 16).ok()?);
            }
            byte => bytes.push(byte),
        }
    }
    String::from_utf8(bytes).ok()
}

/// Плейлист по ссылке: владелец в ней может быть логином, а приложению нужен номер.
pub fn link_playlist(link: &Link, token: &str) -> Result<Playlist, String> {
    let path = match link {
        Link::Playlist(owner, kind) => format!("users/{owner}/playlists/{kind}"),
        Link::PlaylistUuid(id) => format!("playlist/{id}"),
        _ => return Err("Это не ссылка на плейлист".to_string()),
    };
    let v = get(&path, &[], token)?;
    parse_playlist(result(&v), "").ok_or_else(|| "Не удалось открыть плейлист".to_string())
}

fn parse_similar(v: &Value) -> Vec<HomeItem> {
    result(v)["items"].as_array().into_iter().flatten().filter_map(parse_home_item).collect()
}

/// Похожее на альбом или плейлист: альбомы, плейлисты, исполнители и волны.
pub fn similar(list: &Playlist, token: &str) -> Result<Vec<HomeItem>, String> {
    let path = match list.source {
        Source::Album => format!("albums/{}/similar-entities", list.kind),
        Source::Artist => return Ok(Vec::new()),
        Source::Playlist => {
            let v = get(&format!("users/{}/playlists/{}", list.owner, list.kind), &[("rich-tracks", "true")], token)?;
            let Some(uuid) = result(&v)["playlistUuid"].as_str() else { return Ok(Vec::new()) };
            format!("playlist/{uuid}/similar-entities")
        }
    };
    Ok(parse_similar(&get(&path, &[], token)?))
}

/// У блоков главной разные ответы, но элементы в них одни и те же.
fn parse_home_block(title: &str, v: &Value) -> HomeBlock {
    let mut items = Vec::new();
    items.extend(v["newReleases"].as_array().into_iter().flatten().filter_map(parse_release));
    items.extend(v["items"].as_array().into_iter().flatten().filter_map(parse_home_item));
    // Волна жанра приходит одна, без списка.
    if v["seeds"].is_array() {
        items.extend(parse_home_item(v));
    }
    let tabs: Vec<(String, Vec<HomeItem>)> = ["waves", "inStyleTabs"]
        .iter()
        .flat_map(|group| v[*group].as_array().into_iter().flatten())
        .filter_map(|tab| {
            let cards: Vec<HomeItem> = tab["items"].as_array().into_iter().flatten().filter_map(parse_home_item).collect();
            Some((tab["title"].as_str().unwrap_or_default().to_string(), cards)).filter(|(_, cards)| !cards.is_empty())
        })
        .collect();
    if let Some((_, first)) = tabs.first() {
        items.extend(first.iter().cloned());
    }
    // Одна вкладка — это просто блок, а вкладки без названий не на чем показать.
    let tabs = if tabs.len() > 1 && tabs.iter().all(|(title, _)| !title.is_empty()) { tabs } else { Vec::new() };
    if v["playlist"].is_object() {
        items.extend(parse_home_playlist(&v["playlist"]));
    }
    // Промо ведёт на альбом: подкаст или книгу.
    items.extend(v["promotions"].as_array().into_iter().flatten().filter_map(|promo| {
        let id = promo["action"]["weblink"].as_str()?.strip_prefix("/album/")?.trim_matches('/').to_string();
        let title = promo["title"].as_str().filter(|t| !t.is_empty())?.to_string();
        let subtitle = promo["subtitle"].as_str().unwrap_or_default().to_string();
        Some(HomeItem::Album(Card { id, title, subtitle, cover: promo["imageUrl"].as_str().map(str::to_string) }))
    }));
    let favorites = &v["favorites"];
    if favorites.is_object() {
        items.push(HomeItem::Likes { count: favorites["count"].as_u64().unwrap_or(0), cover: cover_uri(favorites) });
    }
    let history = &v["history"];
    if history.is_object() {
        let subtitle = history["subtitleElements"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ");
        items.push(HomeItem::History { subtitle, cover: history["trackCovers"][0]["uri"].as_str().map(str::to_string) });
    }
    // Чарт лежит глубже: сам плейлист и его треки внутри chart.
    let chart = &v["chart"];
    if chart.is_object() {
        items.extend(parse_home_playlist(chart));
    }
    let list = v["tracks"].as_array().or(chart["tracks"].as_array());
    let tracks = list.into_iter().flatten().filter_map(|t| parse_track(if t["track"].is_object() { &t["track"] } else { t })).collect();
    HomeBlock { title: title.to_string(), items, tracks, tabs, more: None }
}

/// Страница из блоков: главная (`main`) или книги и подкасты (`non_music`). Скелет страницы
/// и все блоки выбранной вкладки; блоки грузятся одновременно, порядок сохраняется.
pub fn landing(skeleton: &str, tab: usize, token: &str) -> Result<Home, String> {
    if let Some(id) = skeleton.strip_prefix(LABEL) {
        return label(id, token);
    }
    let skeleton_id = skeleton;
    let skeleton = get(&format!("landing/skeleton/{skeleton}"), &[], token)?;
    let skeleton = result(&skeleton);
    // У книг и подкастов вкладки идут не первым блоком.
    let blocks = skeleton["blocks"].as_array().cloned().unwrap_or_default();
    let tabs = blocks.iter().map(|block| &block["data"]["tabs"]).find(|tabs| tabs.is_array()).unwrap_or(&Value::Null);
    let names = tabs.as_array().into_iter().flatten().filter_map(|t| t["title"].as_str().map(str::to_string)).collect();
    let sources: Vec<(String, String, Option<String>)> = tabs[tab]["blocks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            let data = &b["data"];
            let more = data["viewAllAction"]["weblink"].as_str().map(str::to_string);
            Some((data["title"].as_str().unwrap_or_default().to_string(), data["source"]["uri"].as_str()?.trim_start_matches('/').to_string(), more))
        })
        .collect();
    let mut blocks = Vec::new();
    for part in sources.chunks(HOME_PARALLEL) {
        let handles: Vec<_> = part
            .iter()
            .cloned()
            .map(|(title, uri, more)| {
                let token = token.to_string();
                std::thread::spawn(move || get(&uri, &[], &token).ok().map(|v| HomeBlock { more, ..parse_home_block(&title, result(&v)) }))
            })
            .collect();
        // Без названия показываем только то, что понятно и так: карточки разделов, волну жанра, список жанров.
        let plain = |item: &HomeItem| item.is_shortcut() || matches!(item, HomeItem::Wave { .. } | HomeItem::Landing { cover: None, .. });
        blocks.extend(handles.into_iter().filter_map(|h| h.join().ok().flatten()).filter(|b| (!b.title.is_empty() || b.items.iter().all(plain)) && (!b.items.is_empty() || !b.tracks.is_empty())));
    }
    if skeleton_id == "non_music" {
        let podcasts = tabs[tab]["id"].as_str() == Some("podcasts");
        let mut shelf = shelf_blocks(podcasts, token);
        shelf.append(&mut blocks);
        blocks = shelf;
    }
    Ok(Home { title: skeleton["title"].as_str().unwrap_or_default().to_string(), tabs: names, blocks })
}

/// Страница лейбла: название, альбомы и исполнители — так же, как страница из блоков.
fn label(id: &str, token: &str) -> Result<Home, String> {
    let info = get(&format!("labels/{id}"), &[], token)?;
    let page = [("page", "0"), ("pageSize", "100")];
    let albums = get(&format!("labels/{id}/albums"), &page, token)?;
    let artists = get(&format!("labels/{id}/artists"), &page, token)?;
    let block = |title: &str, items: Vec<HomeItem>| HomeBlock { title: title.to_string(), items, ..HomeBlock::default() };
    let albums = result(&albums)["albums"].as_array().into_iter().flatten().filter_map(album_card).map(HomeItem::Album).collect();
    let artists = result(&artists)["artists"].as_array().into_iter().flatten().filter_map(artist_card).map(HomeItem::Artist).collect();
    let blocks = [block("Альбомы", albums), block("Исполнители", artists)].into_iter().filter(|b| !b.items.is_empty()).collect();
    Ok(Home { title: result(&info)["name"].as_str().unwrap_or_default().to_string(), tabs: Vec::new(), blocks })
}

/// Новые альбомы жанра за неделю, как на странице альбома в оригинале (`top/recent-albums`).
pub fn genre_new_albums(genre: &str, token: &str) -> Result<Vec<Card>, String> {
    let v = get("top/recent-albums", &[("genre", genre), ("period", "week"), ("page", "0"), ("pageSize", "8")], token)?;
    Ok(result(&v)["albums"].as_array().into_iter().flatten().filter_map(album_card).collect())
}

/// Полный список жанра для «Все»: альбомы, плейлисты или исполнители, сначала популярные.
pub fn genre_list(genre: &str, kind: &str, token: &str) -> Result<Vec<HomeItem>, String> {
    let path = format!("metatags/{}/{kind}", encode(genre));
    let v = get(&path, &[("offset", "0"), ("limit", "100"), ("sortBy", "popular"), ("period", "week"), ("tracksPerArtist", "0")], token)?;
    let v = result(&v);
    let items = v[kind].as_array().into_iter().flatten();
    Ok(match kind {
        "albums" => items.filter_map(album_card).map(HomeItem::Album).collect(),
        "artists" => items.filter_map(|a| artist_card(if a["artist"].is_object() { &a["artist"] } else { a })).map(HomeItem::Artist).collect(),
        _ => items.filter_map(parse_home_playlist).collect(),
    })
}

/// Подборка редакции (`feed/promotions/<id>`): название и карточки.
pub fn editorial_post(id: &str, token: &str) -> Result<HomeBlock, String> {
    let v = get(&format!("feed/promotions/{}", encode(id)), &[], token)?;
    let v = result(&v);
    let mut items: Vec<HomeItem> = v["albums"].as_array().into_iter().flatten().filter_map(album_card).map(HomeItem::Album).collect();
    items.extend(v["playlists"].as_array().into_iter().flatten().filter_map(|p| parse_home_playlist(if p["playlist"].is_object() { &p["playlist"] } else { p })));
    items.extend(v["artists"].as_array().into_iter().flatten().filter_map(artist_card).map(HomeItem::Artist));
    Ok(HomeBlock { title: v["title"].as_str().unwrap_or_default().to_string(), items, ..HomeBlock::default() })
}

/// Где остановились в выпуске подкаста или главе книги. Так же место видят другие устройства.
pub fn save_progress(track: &str, position: f32, length: f32, finished: bool, token: &str) -> Result<(), String> {
    let stamp = now();
    let body = json!({ "lastSyncTimestamp": stamp, "trackStreams": [{ "trackId": track, "positionSec": position, "trackLengthSec": length, "finished": finished, "timestamp": stamp }] });
    post("streams/progress/sync", "application/json", body.to_string(), token).map(|_| ())
}

/// Отмечает выпуск или главу прослушанными либо снова непрослушанными.
pub fn mark_finished(track: &str, finished: bool, token: &str) -> Result<(), String> {
    let path = if finished { "mark-finished" } else { "mark-unfinished" };
    post(&format!("streams/progress/{path}?trackId={}", encode(track)), "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

/// То же для подкаста или книги целиком.
pub fn mark_album_finished(album: &str, finished: bool, token: &str) -> Result<(), String> {
    let path = if finished { "mark-album-finished" } else { "mark-album-unfinished" };
    post(&format!("streams/progress/{path}?albumId={}", encode(album)), "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

/// Значение для адреса или формы.
fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn create_playlist(uid: &str, title: &str, token: &str) -> Result<Playlist, String> {
    let v = post(&format!("users/{uid}/playlists/create?visibility=private&title={}", encode(title)), "application/x-www-form-urlencoded", String::new(), token)?;
    parse_playlist(result(&v), uid).ok_or_else(|| "Яндекс не создал плейлист".to_string())
}

/// Самая большая картинка, которую Яндекс берёт в обложку: больше — уменьшается здесь.
const COVER_SIDE_MAX: u32 = 2000;
/// Предел размера файла обложки, как в официальном клиенте.
const COVER_BYTES_MAX: usize = 20_000_000;

/// Картинка для обложки: JPEG и PNG как есть, слишком большая — уменьшенная копия в JPEG.
pub fn cover_file(path: &std::path::Path) -> Result<(Vec<u8>, &'static str), String> {
    let bytes = std::fs::read(path).map_err(|_| "Не удалось прочитать картинку".to_string())?;
    let format = image::guess_format(&bytes).map_err(|_| "Это не картинка JPEG или PNG".to_string())?;
    let mime = match format {
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Png => "image/png",
        _ => return Err("Обложкой может быть только картинка JPEG или PNG".into()),
    };
    let picture = image::load_from_memory(&bytes).map_err(|_| "Картинка повреждена".to_string())?;
    if picture.width().max(picture.height()) <= COVER_SIDE_MAX && bytes.len() <= COVER_BYTES_MAX {
        return Ok((bytes, mime));
    }
    let smaller = picture.resize(COVER_SIDE_MAX, COVER_SIDE_MAX, image::imageops::FilterType::Triangle).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&smaller).map_err(|e| e.to_string())?;
    Ok((out, "image/jpeg"))
}

/// Ставит картинку обложкой своего плейлиста. Вернёт адрес новой обложки.
pub fn upload_cover(uid: &str, kind: &str, bytes: Vec<u8>, mime: &str, token: &str) -> Result<Option<String>, String> {
    let boundary = format!("----tune{}", random_bits());
    let extension = if mime == "image/png" { "png" } else { "jpg" };
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"cover.{extension}\"\r\nContent-Type: {mime}\r\n\r\n").into_bytes();
    body.extend_from_slice(&bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    forget_cache();
    let resp = agent()
        .post(format!("{API}/users/{uid}/playlists/{kind}/cover/upload"))
        .header("Authorization", &format!("OAuth {token}"))
        .header("X-Yandex-Music-Client", CLIENT)
        .header("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .send(&body[..])
        .map_err(|e| format!("Не удалось связаться с Яндексом: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 {
        return Err(NEED_LOGIN.into());
    }
    let text = resp.into_body().read_to_string().map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("Яндекс ответил {status}"));
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| "Яндекс прислал непонятный ответ".to_string())?;
    Ok(result(&v)["cover"]["uri"].as_str().map(str::to_string))
}

pub fn rename_playlist(uid: &str, kind: &str, title: &str, token: &str) -> Result<(), String> {
    post(&format!("users/{uid}/playlists/{kind}/name?value={}", encode(title)), "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

pub fn delete_playlist(uid: &str, kind: &str, token: &str) -> Result<(), String> {
    post(&format!("users/{uid}/playlists/{kind}/delete"), "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

fn change_playlist(uid: &str, kind: &str, diff: &Value, revision: u64, token: &str) -> Result<(), String> {
    let body = format!("diff={}&revision={revision}", encode(&diff.to_string()));
    post(&format!("users/{uid}/playlists/{kind}/change-relative"), "application/x-www-form-urlencoded", body, token).map(|_| ())
}

fn track_ref(id: &str, album: Option<&str>) -> Value {
    match album {
        Some(album) => json!({ "id": id, "albumId": album }),
        None => json!({ "id": id }),
    }
}

/// Плейлист меняют от его последней версии, поэтому она читается перед каждым изменением.
fn playlist_state(uid: &str, kind: &str, token: &str) -> Result<Value, String> {
    let v = get(&format!("users/{uid}/playlists/{kind}"), &[("rich-tracks", "true")], token)?;
    Ok(result(&v).clone())
}

pub fn add_to_playlist(uid: &str, kind: &str, track: &Track, token: &str) -> Result<(), String> {
    let state = playlist_state(uid, kind, token)?;
    let diff = json!([{ "op": "insert", "at": 0, "tracks": [track_ref(&track.id, track.album.as_deref())] }]);
    change_playlist(uid, kind, &diff, state["revision"].as_u64().unwrap_or(0), token)
}

/// Переставляет трек в своём плейлисте. `from` и `to` — места в списке, который видно
/// в окне; на сервере трек ищется по номеру, а сдвиг переносится как есть. `to` — где трек
/// окажется после перестановки.
pub fn move_in_playlist(uid: &str, kind: &str, track: &Track, from: usize, to: usize, token: &str) -> Result<(), String> {
    let state = playlist_state(uid, kind, token)?;
    let items = state["tracks"].as_array().cloned().unwrap_or_default();
    let at = items.iter().position(|item| id_text(&item["track"]["id"]).or_else(|| id_text(&item["id"])).as_deref() == Some(track.id.as_str())).ok_or("Трека уже нет в плейлисте")?;
    let target = (at as isize + to as isize - from as isize).clamp(0, items.len() as isize - 1) as usize;
    if target == at {
        return Ok(());
    }
    let diff = json!([{ "op": "move", "from": at, "to": target, "tracks": [track_ref(&track.id, track.album.as_deref())] }]);
    change_playlist(uid, kind, &diff, state["revision"].as_u64().unwrap_or(0), token)
}

/// Переносит плейлист в коллекции, как оригинал (`change-position`). Место считается по списку
/// с сервера: в окне могут быть не все плейлисты, а сдвиг тот же.
pub fn move_playlist(uid: &str, kind: &str, from: usize, to: usize, token: &str) -> Result<(), String> {
    let v = get(&format!("users/{uid}/playlists/list"), &[], token)?;
    let kinds: Vec<String> = v["result"].as_array().into_iter().flatten().filter_map(|p| id_text(&p["kind"])).collect();
    let at = kinds.iter().position(|k| k == kind).ok_or("Плейлиста уже нет")?;
    let target = (at as isize + to as isize - from as isize).clamp(0, kinds.len() as isize - 1) as usize;
    if target == at {
        return Ok(());
    }
    post(&format!("users/{uid}/playlists/{kind}/change-position?from={at}&to={target}"), "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

pub fn remove_from_playlist(uid: &str, kind: &str, track: &Track, token: &str) -> Result<(), String> {
    let state = playlist_state(uid, kind, token)?;
    let position = state["tracks"].as_array().into_iter().flatten().position(|item| id_text(&item["track"]["id"]).or_else(|| id_text(&item["id"])).as_deref() == Some(track.id.as_str())).ok_or("Трека уже нет в плейлисте")?;
    let diff = json!([{ "op": "delete", "from": position, "to": position + 1, "tracks": [track_ref(&track.id, track.album.as_deref())] }]);
    change_playlist(uid, kind, &diff, state["revision"].as_u64().unwrap_or(0), token)
}

/// Последние прослушанные треки, новые сначала и без повторов.
/// Откуда слушали треки в истории.
#[derive(Clone, Debug, PartialEq)]
pub enum HistoryFrom {
    /// «Слушали вперемешку».
    Other,
    Search,
    Wave { title: String, seeds: Vec<String> },
    Playlist(Playlist),
    Album(String),
    Artist(String),
}

/// Подряд прослушанное из одного места за один день. `tracks` — строки общего списка истории.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryBlock {
    /// «2026-11-03».
    pub date: String,
    pub from: HistoryFrom,
    pub tracks: std::ops::Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    pub tracks: Vec<Track>,
    pub blocks: Vec<HistoryBlock>,
}

fn parse_history_from(v: &Value) -> HistoryFrom {
    let (id, model) = (&v["data"]["itemId"], &v["data"]["fullModel"]);
    match v["type"].as_str() {
        Some("search") => HistoryFrom::Search,
        Some("wave") => {
            let seeds: Vec<String> = id["seeds"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect();
            let title = model["wave"]["title"].as_str().unwrap_or("Моя волна").to_string();
            if seeds.is_empty() { HistoryFrom::Other } else { HistoryFrom::Wave { title, seeds } }
        }
        Some("playlist") => {
            let (Some(owner), Some(kind)) = (id_text(&id["uid"]), id_text(&id["kind"])) else { return HistoryFrom::Other };
            let list = &model["playlist"];
            HistoryFrom::Playlist(Playlist {
                source: Source::Playlist,
                owner,
                kind,
                title: list["title"].as_str().unwrap_or("Плейлист").to_string(),
                count: model["tracksCount"].as_u64().unwrap_or(0),
                cover: cover_uri(list),
            })
        }
        Some("album") => id_text(&id["id"]).map_or(HistoryFrom::Other, HistoryFrom::Album),
        Some("artist") => id_text(&id["id"]).map_or(HistoryFrom::Other, HistoryFrom::Artist),
        _ => HistoryFrom::Other,
    }
}

/// Дни истории, новые первыми, и в каждом — откуда слушали и какие треки. Не больше `limit`
/// треков всего: старое отрезается.
fn parse_history(v: &Value, limit: usize) -> Vec<(String, HistoryFrom, Vec<String>)> {
    let mut blocks = Vec::new();
    let mut left = limit;
    for tab in v["historyTabs"].as_array().into_iter().flatten() {
        let date = tab["date"].as_str().unwrap_or_default().to_string();
        for item in tab["items"].as_array().into_iter().flatten() {
            let ids: Vec<String> = item["tracks"].as_array().into_iter().flatten().filter_map(|track| id_text(&track["data"]["itemId"]["trackId"])).take(left).collect();
            if ids.is_empty() {
                continue;
            }
            left -= ids.len();
            blocks.push((date.clone(), parse_history_from(&item["context"]), ids));
            if left == 0 {
                return blocks;
            }
        }
    }
    blocks
}

pub fn history(limit: usize, token: &str) -> Result<History, String> {
    let v = get("music-history", &[("fullModelsCount", "50")], token)?;
    let parsed = parse_history(result(&v), limit);
    // Сведения о каждом треке просятся один раз, даже если его слушали в разные дни.
    let mut seen = HashSet::new();
    let unique: Vec<String> = parsed.iter().flat_map(|(_, _, ids)| ids.iter()).filter(|id| seen.insert((*id).clone())).cloned().collect();
    let found: HashMap<String, Track> = tracks_by_ids(&unique, token)?.into_iter().map(|track| (track.id.clone(), track)).collect();
    let mut history = History::default();
    for (date, from, ids) in parsed {
        let start = history.tracks.len();
        history.tracks.extend(ids.iter().filter_map(|id| found.get(id).cloned()));
        if history.tracks.len() > start {
            history.blocks.push(HistoryBlock { date, from, tracks: start..history.tracks.len() });
        }
    }
    Ok(history)
}

/// Что можно закрепить в боковой панели.
#[derive(Clone, Debug, PartialEq)]
pub enum PinRef {
    Album(String),
    Artist(String),
    Playlist(String, String),
    Wave(Vec<String>),
}

impl Pin {
    /// Мою волну Яндекс возвращает всегда, поэтому открепить её нельзя.
    pub fn removable(&self) -> bool {
        self.key() != MY_WAVE
    }

    /// Ссылка на этот закреп для запроса на открепление.
    pub fn reference(&self) -> PinRef {
        match &self.target {
            PinTarget::Wave(seeds) => PinRef::Wave(seeds.clone()),
            PinTarget::List(list) => match list.source {
                Source::Album => PinRef::Album(list.kind.clone()),
                Source::Artist => PinRef::Artist(list.kind.clone()),
                Source::Playlist => PinRef::Playlist(list.owner.clone(), list.kind.clone()),
            },
        }
    }

    /// Постоянное имя обложки: порядок закрепов меняется.
    pub fn key(&self) -> String {
        match &self.target {
            PinTarget::Wave(seeds) => seeds.join(","),
            PinTarget::List(list) => format!("{:?}:{}:{}", list.source, list.owner, list.kind),
        }
    }
}

impl PinRef {
    fn path(&self) -> &'static str {
        match self {
            PinRef::Album(_) => "pin/album",
            PinRef::Artist(_) => "pin/artist",
            PinRef::Playlist(..) => "pin/playlist",
            PinRef::Wave(_) => "pin/wave",
        }
    }

    fn body(&self) -> Value {
        match self {
            PinRef::Album(id) | PinRef::Artist(id) => json!({ "id": id }),
            PinRef::Playlist(uid, kind) => json!({ "uid": uid, "kind": kind }),
            PinRef::Wave(seeds) => json!({ "seeds": seeds }),
        }
    }

    pub fn matches(&self, pin: &Pin) -> bool {
        if let (PinRef::Wave(seeds), PinTarget::Wave(pinned)) = (self, &pin.target) {
            return seeds == pinned;
        }
        let PinTarget::List(list) = &pin.target else { return false };
        match self {
            PinRef::Album(id) => list.source == Source::Album && list.kind == *id,
            PinRef::Artist(id) => list.source == Source::Artist && list.kind == *id,
            PinRef::Playlist(uid, kind) => list.source == Source::Playlist && list.owner == *uid && list.kind == *kind,
            PinRef::Wave(_) => false,
        }
    }
}

/// Закрепляет или открепляет: тело запроса одно, а метод разный.
pub fn set_pinned(target: &PinRef, pinned: bool, token: &str) -> Result<(), String> {
    let url = format!("{API}/{}", target.path());
    let body = target.body().to_string();
    forget_cache();
    let request = if pinned { agent().put(url) } else { agent().delete(url).force_send_body() };
    let resp = request
        .header("Authorization", &format!("OAuth {token}"))
        .header("X-Yandex-Music-Client", CLIENT)
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| format!("Не удалось связаться с Яндексом: {e}"))?;
    match resp.status().as_u16() {
        401 => Err(NEED_LOGIN.into()),
        200..=299 => Ok(()),
        status => Err(format!("Яндекс ответил {status}")),
    }
}

/// Описание и приватность плейлиста: их нет в общем списке треков.
pub fn playlist_meta(uid: &str, kind: &str, token: &str) -> Result<(String, String), String> {
    let v = get(&format!("users/{uid}/playlists/{kind}"), &[("rich-tracks", "false")], token)?;
    let v = result(&v);
    Ok((v["description"].as_str().unwrap_or_default().to_string(), v["visibility"].as_str().unwrap_or("public").to_string()))
}

pub fn set_description(uid: &str, kind: &str, text: &str, token: &str) -> Result<(), String> {
    post(&format!("users/{uid}/playlists/{kind}/description?value={}", encode(text)), "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

pub fn set_visibility(uid: &str, kind: &str, value: &str, token: &str) -> Result<(), String> {
    post(&format!("users/{uid}/playlists/{kind}/visibility?value={value}"), "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

fn parse_liked_list(item: &Value) -> Option<Playlist> {
    let v = if item["playlist"].is_object() { &item["playlist"] } else { item };
    let owner = id_text(&v["uid"]).or_else(|| id_text(&v["owner"]["uid"]))?;
    parse_playlist_loose(v, &owner)
}

pub fn liked_playlists(uid: &str, token: &str) -> Result<Vec<Playlist>, String> {
    let v = get(&format!("users/{uid}/likes/playlists"), &[], token)?;
    Ok(result(&v).as_array().into_iter().flatten().filter_map(parse_liked_list).collect())
}

pub fn set_playlist_liked(uid: &str, owner: &str, kind: &str, liked: bool, token: &str) -> Result<(), String> {
    let path = if liked { format!("users/{uid}/likes/playlists/add?owner-uid={owner}&kind={kind}") } else { format!("users/{uid}/likes/playlists/{owner}-{kind}/remove") };
    post(&path, "application/x-www-form-urlencoded", String::new(), token).map(|_| ())
}

pub fn disliked_tracks(uid: &str, limit: usize, token: &str) -> Result<Vec<Track>, String> {
    let mut ids = disliked_ids(uid, token)?;
    ids.truncate(limit);
    tracks_by_ids(&ids, token)
}

pub const MY_WAVE: &str = WAVE_SEED;

pub fn wave_new(seeds: &[String], token: &str) -> Result<Wave, String> {
    let body = json!({ "seeds": seeds, "includeTracksInResponse": true, "allowExplicit": true }).to_string();
    let v = post("rotor/session/new", "application/json", body, token)?;
    let v = result(&v);
    let session = v["radioSessionId"].as_str().ok_or("Яндекс не запустил волну")?;
    Ok(Wave { session: session.to_string(), tracks: parse_sequence(v) })
}

/// Разбирает последнюю волну: зёрна без настроек (они выбираются отдельно) и название.
/// У обычной Моей волны названия нет: её подпись и так постоянная.
fn parse_last_wave(v: &Value) -> Option<(Vec<String>, Option<String>)> {
    let seeds: Vec<String> = v["seeds"].as_array()?.iter().filter_map(|seed| seed.as_str()).filter(|seed| !seed.starts_with("setting") && !seed.starts_with("activity:")).map(str::to_string).collect();
    if seeds.is_empty() || seeds == [MY_WAVE] {
        return Some((vec![MY_WAVE.to_string()], None));
    }
    Some((seeds, v["name"].as_str().filter(|name| !name.is_empty()).map(str::to_string)))
}

/// Какая волна играла в прошлый раз.
pub fn wave_last(token: &str) -> Result<(Vec<String>, Option<String>), String> {
    let v = get("rotor/wave/last", &[], token)?;
    parse_last_wave(result(&v)).ok_or_else(|| "Яндекс не вернул последнюю волну".to_string())
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

/// Одно прослушивание трека для отчёта `plays`: по ним Яндекс ведёт историю и подбирает
/// рекомендации. Поля те же, что шлёт официальный клиент.
#[derive(Clone, Debug)]
pub struct Listen {
    pub id: String,
    pub track: String,
    pub album: Option<String>,
    pub length: f32,
    pub from: String,
    pub context: &'static str,
    pub context_item: Option<String>,
    pub playlist: Option<String>,
    /// Сессия волны и пачка, из которой пришёл трек.
    pub radio: Option<(String, Option<String>)>,
    added: String,
    pub started: Option<String>,
    pub restored: bool,
    pub repeated: bool,
    pub seek: bool,
    pub pause: bool,
    /// Сколько секунд трек действительно звучал, без пауз и перемоток.
    pub played: f32,
}

impl Listen {
    pub fn new(track: &Track, from: String, context: &'static str, context_item: Option<String>) -> Self {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) + 1;
        Self {
            id: random_id(),
            track: track.id.clone(),
            album: track.album.clone(),
            length: track.duration.as_secs_f32(),
            from,
            context,
            context_item,
            playlist: None,
            radio: None,
            added: format!("{}-{secs}", random_bits() % 10_u64.pow(16)),
            started: None,
            restored: false,
            repeated: false,
            seek: false,
            pause: false,
            played: 0.0,
        }
    }

    /// Запись отчёта. Первая уходит при запуске, без прослушанного времени и причины.
    pub fn report(&self, position: f32, reason: Option<&str>, first: bool) -> Value {
        let round = |value: f32| (value * 1000.0).round() / 1000.0;
        // Почти до конца — значит до конца: так же округляет официальный клиент.
        let snap = |value: f32| if (self.length - value).abs() < 0.5 { self.length } else { value };
        let (played, end) = if first { (0.0, 0.0) } else { (snap(self.played), snap(position)) };
        let mut play = json!({
            "playId": self.id,
            "from": self.from,
            "totalPlayedSeconds": round(played),
            "endPositionSeconds": round(end),
            "trackLengthSeconds": round(self.length),
            "timestamp": now(),
            "context": self.context,
            "contextItem": self.context_item,
            "addTracksToPlayerTime": self.added,
            "fromCache": false,
            "isRestored": self.restored,
            "audioAuto": "none",
            "audioOutputName": "Динамик",
            "audioOutputType": "Speaker",
            "trackId": self.track,
            "seek": self.seek,
            "pause": self.pause,
            "isRepeated": self.repeated,
            "isLivePlayableIndex": self.radio.is_some(),
        });
        if let Some(album) = &self.album {
            play["albumId"] = json!(album);
        }
        if let Some(playlist) = &self.playlist {
            play["playlistId"] = json!(playlist);
        }
        if let Some((session, batch)) = &self.radio {
            play["radioSessionId"] = json!(session);
            if let Some(batch) = batch {
                play["batchId"] = json!(batch);
            }
        }
        if let Some(started) = &self.started {
            play["startTimestamp"] = json!(started);
        }
        if !first {
            play["maxPlayerStage"] = json!("play");
        }
        if let Some(reason) = reason {
            play["changeReason"] = json!(reason);
        }
        play
    }
}

/// Случайные 64 бита: ключи `RandomState` каждый раз новые.
fn random_bits() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.finish()
}

/// Случайный UUID версии 4.
pub fn random_id() -> String {
    let (a, b) = (random_bits(), random_bits());
    format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", a >> 32, (a >> 16) & 0xffff, (a & 0x0fff) | 0x4000, ((b >> 48) & 0x3fff) | 0x8000, b & 0xffff_ffff_ffff)
}

pub fn now_stamp() -> String {
    now()
}

/// Можно ли сейчас слушать треки: номер трека и ответ.
/// Сырой ответ на запрос чтения: для проверки, что отдаёт сервер.
pub fn probe(path: &str, query: &[(&str, &str)], token: &str) -> Result<Value, String> {
    fetch(path, query, token)
}

/// Что оригинал вставляет после трека (`after-track`): спрашивает он только рекламу.
pub fn after_track(from: &str, types: &str, prev: &str, next: &str, token: &str) -> Result<Value, String> {
    get("after-track", &[("from", from), ("types", types), ("prevTrackId", prev), ("nextTrackId", next)], token)
}

pub fn availability(ids: &[String], token: &str) -> Result<Vec<(String, bool)>, String> {
    let mut found = Vec::new();
    for part in ids.chunks(100) {
        let v = post(&format!("availability/tracks?trackIds={}", part.join("%2C")), "application/json", String::new(), token)?;
        found.extend(v["result"]["availabilities"].as_array().into_iter().flatten().filter_map(|item| Some((id_text(&item["trackId"])?, item["available"].as_bool()?))));
    }
    Ok(found)
}

/// Отправляет отчёты о прослушивании.
/// Ошибка из-за сети, а не из-за Яндекса: запрос просто не дошёл.
pub fn network_error(error: &str) -> bool {
    ["Не удалось связаться с Яндексом", "Не удалось получить файл трека", "Не удалось загрузить текст"].iter().any(|prefix| error.starts_with(prefix))
}

pub fn plays(reports: Vec<Value>, token: &str) -> Result<(), String> {
    let client_now = now().replace(':', "%3A");
    post(&format!("plays?clientNow={client_now}"), "application/json", json!({ "plays": reports }).to_string(), token).map(|_| ())
}

/// Текст песни. У синхронного у каждой строки есть время.
#[derive(Clone, Debug, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<(Option<f32>, String)>,
    pub authors: Vec<String>,
}

impl Lyrics {
    pub fn synced(&self) -> bool {
        self.lines.iter().any(|(time, _)| time.is_some())
    }

    /// Строка, которая звучит в момент `position`.
    pub fn line_at(&self, position: f32) -> Option<usize> {
        self.lines.iter().rposition(|(time, _)| time.is_some_and(|time| time <= position))
    }
}

/// Строка LRC: `[01:23.45] текст`. Без метки времени остаётся обычным текстом.
fn parse_lrc(text: &str) -> Vec<(Option<f32>, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let timed = line.strip_prefix('[').and_then(|rest| rest.split_once(']')).and_then(|(stamp, words)| {
                let (minutes, seconds) = stamp.split_once(':')?;
                Some((minutes.parse::<f32>().ok()? * 60.0 + seconds.parse::<f32>().ok()?, words.trim()))
            });
            match timed {
                Some((_, "")) => None,
                Some((time, words)) => Some((Some(time), words.to_string())),
                None if line.is_empty() => None,
                None => Some((None, line.to_string())),
            }
        })
        .collect()
}

fn lyrics_in(id: &str, format: &str, token: &str) -> Result<Option<Lyrics>, String> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs().to_string();
    // Текст подписывается целиком, в отличие от запроса файла.
    let signature = sign_full(&format!("{id}{ts}"));
    let v = match get(&format!("tracks/{id}/lyrics"), &[("format", format), ("timeStamp", &ts), ("sign", &signature)], token) {
        // Так Яндекс отвечает, когда текста в этом виде просто нет.
        Err(e) if e.starts_with("Яндекс ответил 404") => return Ok(None),
        other => other?,
    };
    let v = result(&v);
    let Some(url) = v["downloadUrl"].as_str() else { return Ok(None) };
    let text = agent().get(url).call().map_err(|e| format!("Не удалось загрузить текст: {e}"))?.into_body().read_to_string().map_err(|e| e.to_string())?;
    let authors = v["writers"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_string)).collect();
    Ok(Some(Lyrics { lines: parse_lrc(&text), authors }).filter(|lyrics| !lyrics.lines.is_empty()))
}

/// Сначала синхронный текст, потом обычный. У многих треков есть только один из них.
pub fn lyrics(id: &str, token: &str) -> Result<Option<Lyrics>, String> {
    match lyrics_in(id, "LRC", token)? {
        Some(found) => Ok(Some(found)),
        None => lyrics_in(id, "TEXT", token),
    }
}

fn sign_full(data: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SIGN_KEY).expect("ключ HMAC любой длины");
    mac.update(data.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn sign(data: &str) -> String {
    let mut s = sign_full(data);
    // Как в оригинале: подпись без последнего символа.
    s.pop();
    s
}

pub fn file_info(track_id: &str, quality: &str, token: &str) -> Result<FileInfo, String> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs().to_string();
    // HE-AAC просим только в MP4: его кадры декодирует декодер Windows (см. `heaac`).
    let codecs: Vec<&str> = CODECS.iter().copied().filter(|codec| *codec != "he-aac").collect();
    let signature = sign(&format!("{ts}{track_id}{quality}{}{TRANSPORT}", codecs.concat()));
    let codecs = codecs.join(",");
    let v = get(
        "get-file-info",
        &[("ts", &ts), ("trackId", track_id), ("quality", quality), ("codecs", &codecs), ("transports", TRANSPORT), ("sign", &signature)],
        token,
    )?;
    let info = if v["result"].is_object() { &v["result"]["downloadInfo"] } else { &v["downloadInfo"] };
    let url = info["url"].as_str().or(info["urls"][0].as_str()).ok_or("Яндекс не дал файл трека")?;
    Ok(FileInfo {
        url: url.to_string(),
        key: info["key"].as_str().filter(|k| !k.is_empty()).map(str::to_string),
        codec: info["codec"].as_str().unwrap_or("mp3").to_string(),
        bitrate: info["bitrate"].as_u64().unwrap_or(0),
        quality: info["quality"].as_str().unwrap_or(quality).to_string(),
    })
}

/// Обложка нужного размера, например 100x100. Нет сети — останется заглушка.
pub fn cover(uri: &str, size: &str) -> Option<Vec<u8>> {
    let url = format!("https://{}", uri.trim_start_matches("https://").replace("%%", size));
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
    fn listen_report_has_official_fields() {
        let track = Track { id: "123".into(), album: Some("45".into()), duration: Duration::from_secs(200), ..Track::default() };
        let mut listen = Listen::new(&track, "desktop-album_screen-default".into(), "album", Some("45".into()));
        assert_eq!(listen.id.len(), 36);
        assert_eq!(&listen.id[14..15], "4");
        let start = listen.report(0.0, None, true);
        assert_eq!(start["totalPlayedSeconds"], 0.0);
        assert!(start.get("maxPlayerStage").is_none() && start.get("changeReason").is_none());
        listen.played = 199.7;
        let end = listen.report(199.8, Some("finish"), false);
        assert_eq!(end["totalPlayedSeconds"], 200.0);
        assert_eq!(end["endPositionSeconds"], 200.0);
        assert_eq!(end["changeReason"], "finish");
        assert_eq!(end["albumId"], "45");
        assert_eq!(end["trackId"], "123");
    }

    #[test]
    fn reads_artist_about_and_release_kind() {
        let v = json!({"description":" Группа. ","links":[{"title":"Сайт","subtitle":"q.com","url":"http://q.com/"},{"title":"Плохая","url":"javascript:alert(1)"}]});
        let (about, links) = parse_about(&v);
        assert_eq!(about, "Группа.");
        assert_eq!(links, vec![ArtistLink { title: "Сайт".into(), subtitle: "q.com".into(), url: "http://q.com/".into() }]);
        assert_eq!(album_subtitle(&json!({"year":2020,"type":"single"})), "2020 · Сингл");
        assert_eq!(album_subtitle(&json!({"year":1975})), "1975");
        assert_eq!(album_subtitle(&json!({})), "Альбом");
    }

    #[test]
    fn reads_album_details() {
        assert_eq!(release_date("2011-01-01T00:00:00+03:00").as_deref(), Some("1 января 2011"));
        assert_eq!(release_date("1977-10-28").as_deref(), Some("28 октября 1977"));
        assert_eq!(release_date("2011-13-01"), None);
        assert_eq!(release_date("2011"), None);
        let v = json!({"id":1,"title":"A","version":" Deluxe ","labels":[{"id":1,"name":"Universal"},"EMI"],"duplicates":[{"id":2,"title":"A","year":1977,"version":"Remaster"},{"id":3,"title":"A","year":1980}]});
        let album = parse_album(&v).unwrap();
        assert_eq!(album.version, "Deluxe");
        assert_eq!(album.labels, vec![(Some("1".to_string()), "Universal".to_string()), (None, "EMI".to_string())]);
        assert_eq!(album.versions.iter().map(|c| c.subtitle.as_str()).collect::<Vec<_>>(), vec!["Remaster", "1980"]);
    }

    #[test]
    fn reads_track_details() {
        let genres = json!([{"id":"allrock","title":"Rock","titles":{"ru":{"title":"Рок"}},"subGenres":[{"id":"rock","title":"Рок-музыка"}]}]);
        let full = json!({"track":{"version":"Remastered 2011","albums":[{"genre":"rock","year":1975}]},"similarTracks":[{"id":5,"title":"T"}]});
        let details = parse_details(&full, &genres, vec![("Исполнитель".into(), "Queen".into())]);
        let facts: Vec<(&str, &str)> = details.facts.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        assert_eq!(facts, vec![("Жанр", "Рок-музыка"), ("Год", "1975"), ("Издание", "Remastered 2011"), ("Исполнитель", "Queen")]);
        assert_eq!(details.similar.len(), 1);
        assert_eq!(genre_title(&genres, "allrock").as_deref(), Some("Рок"));
        assert!(parse_details(&Value::Null, &Value::Null, Vec::new()).facts.is_empty());
    }

    #[test]
    fn reads_home_blocks_with_tabs() {
        let wave = |title: &str| json!({"title": title, "seeds": [format!("mood:{title}")], "backgroundImageUrl": "bg/%%", "compactImageUrl": "own/%%", "colors": {"average": "#d65c5b"}});
        let v = json!({"waves": [{"title": "топ", "items": [wave("a"), wave("b")]}, {"title": "настроение", "items": [wave("c")]}, {"title": "пусто", "items": []}]});
        let block = parse_home_block("Волны", &v);
        assert_eq!(block.tabs.iter().map(|(t, items)| (t.as_str(), items.len())).collect::<Vec<_>>(), vec![("топ", 2), ("настроение", 1)]);
        assert_eq!(block.items.len(), 2);
        assert!(matches!(&block.items[0], HomeItem::Wave { title, cover, color, .. } if title == "a" && cover.as_deref() == Some("own/%%") && color.as_deref() == Some("#d65c5b")));
        let style = json!({"inStyleTabs": [{"title": "Группа", "items": [{"album": {"id": 7, "title": "Альбом", "year": 2020}}]}]});
        let block = parse_home_block("В стиле", &style);
        assert!(block.tabs.is_empty());
        assert!(matches!(&block.items[0], HomeItem::Album(card) if card.id == "7"));
    }

    #[test]
    fn reads_mixes_and_tag_playlists() {
        let v = json!({"items":[{"type":"mix_card_item","data":{"id":"autumn_playlists","title":"Осенняя","action":{"weblink":"/tag/autumn"},"covers":[{"uri":"c/%%"}]}}]});
        let block = parse_home_block("Миксы", &v);
        assert!(matches!(&block.items[0], HomeItem::Mix { tag, title, cover } if tag == "autumn" && title == "Осенняя" && cover.as_deref() == Some("c/%%")));
        let lists = parse_tag_playlists(&json!([{"uid":103372440,"kind":2620,"title":"Лоу-фай","trackCount":247},{"kind":1}]));
        assert_eq!(lists.len(), 1);
        assert_eq!((lists[0].owner.as_str(), lists[0].kind.as_str(), lists[0].count), ("103372440", "2620", 247));
    }

    #[test]
    fn reads_chart_block() {
        let v = json!({"title":"Чарт","chart":{"uid":414787002,"kind":1076,"title":"Чарт","tracks":[{"chartPosition":{"position":1},"track":{"id":"5","title":"Т","durationMs":1}}]}});
        let block = parse_home_block("Чарт", &v);
        assert_eq!(block.tracks.len(), 1);
        assert!(matches!(&block.items[0], HomeItem::Playlist(list) if list.kind == "1076"));
    }

    #[test]
    fn reads_likes_and_history_block() {
        let v = json!({"favorites":{"count":214,"cover":{"uri":"likes/%%"}},"history":{"subtitleElements":["A","B"],"trackCovers":[{"uri":"h/%%"}]}});
        let block = parse_home_block("", &v);
        assert!(matches!(&block.items[0], HomeItem::Likes { count: 214, cover } if cover.as_deref() == Some("likes/%%")));
        assert!(matches!(&block.items[1], HomeItem::History { subtitle, cover } if subtitle == "A, B" && cover.as_deref() == Some("h/%%")));
        assert!(block.items.iter().all(HomeItem::is_shortcut));
    }

    #[test]
    fn labels_sound() {
        let info = |codec: &str, bitrate| FileInfo { url: String::new(), key: None, codec: codec.to_string(), bitrate, quality: "nq".to_string() };
        assert_eq!(info("aac-mp4", 256).label(), "AAC, 256 кбит/с");
        let sound = info("aac-mp4", 192).sound();
        assert!(sound.below("lossless"));
        assert!(!sound.below("nq"));
        assert!(!sound.below("lq"));
        assert_eq!(info("flac-mp4", 0).label(), "FLAC");
        assert_eq!(codec_name("he-aac-mp4"), "HE-AAC");
    }

    #[test]
    fn pin_reference_matches_its_target() {
        let list = |source, owner: &str, kind: &str| Pin { target: PinTarget::List(Playlist { source, owner: owner.to_string(), kind: kind.to_string(), title: String::new(), count: 0, cover: None }), title: String::new(), subtitle: String::new(), cover: None, round: false, color: None };
        assert!(PinRef::Album("1".into()).matches(&list(Source::Album, "", "1")));
        assert!(!PinRef::Album("1".into()).matches(&list(Source::Artist, "", "1")));
        assert!(PinRef::Playlist("5".into(), "7".into()).matches(&list(Source::Playlist, "5", "7")));
        assert!(!PinRef::Playlist("6".into(), "7".into()).matches(&list(Source::Playlist, "5", "7")));
        assert_eq!(PinRef::Playlist("5".into(), "7".into()).body(), json!({"uid":"5","kind":"7"}));
        let v = json!({"playlist":{"uid":5,"kind":7,"title":"П"}});
        assert_eq!(parse_liked_list(&v).unwrap().owner, "5");
        let v = json!({"owner":{"uid":6},"kind":"8","title":"Р"});
        assert_eq!(parse_liked_list(&v).unwrap().kind, "8");
    }

    #[test]
    fn reads_familiar_tracks() {
        let v = json!({"wave":{"tracks":[{"id":"1","title":"А","durationMs":1000},{"id":"2","title":"Б","durationMs":1000}]},"collection":{}});
        let (collection, wave) = parse_familiar(&v);
        assert_eq!((collection.len(), wave.len(), wave[1].title.as_str()), (0, 2, "Б"));
    }

    #[test]
    fn encodes_titles_and_reads_history() {
        assert_eq!(encode("а b&c"), "%D0%B0%20b%26c");
        let v = json!({"historyTabs":[
            {"date":"2026-11-03","items":[
                {"context":{"type":"wave","data":{"itemId":{"seeds":["user:onyourwave"]}}},"tracks":[{"data":{"itemId":{"trackId":"1"}}},{"data":{"itemId":{"trackId":"2"}}}]},
                {"context":{"type":"playlist","data":{"itemId":{"uid":7,"kind":3},"fullModel":{"playlist":{"title":"Мне нравится"},"tracksCount":5}}},"tracks":[{"data":{"itemId":{"trackId":"5"}}}]}]},
            {"date":"2026-11-02","items":[{"context":{"type":"other"},"tracks":[{"data":{"itemId":{"trackId":"1"}}},{"data":{}},{"data":{"itemId":{"trackId":"3"}}}]}]}]});
        let blocks = parse_history(&v, 10);
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[0].1, HistoryFrom::Wave { title, seeds } if title == "Моя волна" && seeds == &["user:onyourwave"]));
        assert!(matches!(&blocks[1].1, HistoryFrom::Playlist(list) if list.title == "Мне нравится" && list.owner == "7" && list.kind == "3"));
        assert_eq!((blocks[2].0.as_str(), &blocks[2].1, blocks[2].2.as_slice()), ("2026-11-02", &HistoryFrom::Other, ["1".to_string(), "3".to_string()].as_slice()));
        // Сверх предела старое отрезается.
        let cut = parse_history(&v, 3);
        assert_eq!(cut.iter().map(|(_, _, ids)| ids.len()).collect::<Vec<_>>(), [2, 1]);
    }

    #[test]
    fn splits_artists_of_uploaded_tracks() {
        let track = |json: &str| parse_track_artists(&serde_json::from_str(json).unwrap());
        let names = |list: Vec<(String, String)>| list.into_iter().map(|(name, id)| format!("{name}={id}")).collect::<Vec<_>>();
        assert_eq!(names(track(r#"{"trackSource":"UGC","artists":[{"id":"1","name":"А, Б"}]}"#)), ["А=", "Б="]);
        assert_eq!(names(track(r#"{"trackSource":"UGC","artists":[{"name":"А, Б, В"}]}"#)), ["А=", "Б=", "В="]);
        assert_eq!(names(track(r#"{"trackSource":"UGC","artists":[{"id":"1","name":"А"}]}"#)), ["А=1"]);
        assert_eq!(names(track(r#"{"trackSource":"UGC","artists":[{"name":"А"}]}"#)), ["А="]);
        // Обычный исполнитель с запятой в имени остаётся целым.
        assert_eq!(names(track(r#"{"artists":[{"id":5,"name":"Tyler, The Creator"}]}"#)), ["Tyler, The Creator=5"]);
    }

    #[test]
    fn cache_keeps_fresh_answers_only() {
        let now = Instant::now();
        let mut cache = Cache::default();
        cache.put("a".to_string(), json!(1), now);
        assert_eq!(cache.get("a", now + CACHE_TIME / 2), Some(json!(1)));
        assert_eq!(cache.get("a", now + CACHE_TIME), None);
        assert_eq!(cache.get("b", now), None);
        for i in 0..CACHE_SIZE + 5 {
            cache.put(format!("k{i}"), json!(i), now + Duration::from_millis(i as u64));
        }
        assert!(cache.entries.len() <= CACHE_SIZE);
        assert_eq!(cache.get("k0", now + CACHE_TIME / 2), None);
        assert_eq!(html_text("Это подкаст &laquo;Время&raquo; &#8212; о деньгах.<br/><br/><br/>Сезон <b>пятый</b>&nbsp;&amp; финал"), "Это подкаст «Время» — о деньгах.\n\nСезон пятый & финал");
        assert!(!cacheable("music-history") && !cacheable("get-file-info") && cacheable("albums/1/with-tracks"));
    }

    #[test]
    fn pin_key_of_my_wave_is_its_seed() {
        let pin = Pin { target: PinTarget::Wave(vec![MY_WAVE.to_string()]), title: String::new(), subtitle: String::new(), cover: None, round: true, color: None };
        assert_eq!(pin.key(), MY_WAVE);
        assert!(!pin.removable());
    }

    #[test]
    fn pinned_wave_keeps_its_color() {
        let v: Value = serde_json::from_str(r##"{"type":"wave_item","data":{"colors":{"average":"#2AA75B"},"seeds":["user:onyourwave"],"title":"Моя волна"}}"##).unwrap();
        assert_eq!(parse_pin(&v).unwrap().color.as_deref(), Some("#2AA75B"));
    }

    #[test]
    fn pinned_wave_round_trip() {
        let v: Value = serde_json::from_str(r#"{"type":"wave_item","data":{"agent":{"cover":{"uri":""}},"backgroundImageUrl":"","header":"Моя волна по альбому","seeds":["album:7"],"title":"Альбом"}}"#).unwrap();
        let pin = parse_pin(&v).unwrap();
        assert_eq!(pin.subtitle, "Моя волна по альбому");
        assert!(pin.removable());
        assert_eq!(pin.color, None);
        assert_eq!(pin.cover, None);
        assert!(pin.reference().matches(&pin));
        assert_eq!(pin.reference().body(), json!({"seeds":["album:7"]}));
        assert!(!PinRef::Wave(vec!["album:8".to_string()]).matches(&pin));
    }

    #[test]
    fn reads_account_details() {
        assert_eq!(avatar_uri("0/0-0"), None);
        assert_eq!(avatar_uri(""), None);
        assert_eq!(avatar_uri("1/ab-c").as_deref(), Some("avatars.mds.yandex.net/get-yapic/1/ab-c/%%"));
        assert_eq!(short_date("2026-11-09T09:01:22+03:00").as_deref(), Some("09.11.2026"));
        assert_eq!(short_date("2026"), None);
    }

    #[test]
    fn parses_links() {
        let link = |text: &str| parse_link(text);
        assert_eq!(link("https://music.yandex.ru/album/3389008"), Some(Link::Album("3389008".to_string())));
        assert_eq!(link(" https://music.yandex.ru/album/1/track/2?utm_source=x "), Some(Link::Album("1".to_string())));
        assert_eq!(link("music.yandex.com/track/5"), Some(Link::Track("5".to_string())));
        assert_eq!(link("https://music.yandex.kz/artist/3121/tracks"), Some(Link::Artist("3121".to_string())));
        assert_eq!(link("https://music.yandex.ru/users/music-blog/playlists/2131"), Some(Link::Playlist("music-blog".to_string(), "2131".to_string())));
        assert_eq!(link("https://music.yandex.ru/playlists/lk.ab-cd"), Some(Link::PlaylistUuid("ab-cd".to_string())));
        assert_eq!(link("https://music.yandex.ru/genre/rusrock/albums"), Some(Link::Landing("tag_rusrock".to_string())));
        assert_eq!(link("yandexmusic://landing/music_genres"), Some(Link::Landing("music_genres".to_string())));
        assert_eq!(link("https://music.yandex.ru/post/5d776caa24c49852264d102d"), Some(Link::Post("5d776caa24c49852264d102d".to_string())));
        let page = |path: &str| Some(Link::Page(path.to_string()));
        assert_eq!(link("https://music.yandex.ru/home"), page("home"));
        assert_eq!(link("https://music.yandex.ru"), page("home"));
        assert_eq!(link("yandexmusic://album/7"), Some(Link::Album("7".to_string())));
        assert_eq!(link("music-application://desktop/collection/albums?from=x"), page("collection/albums"));
        assert_eq!(link("music-application://desktop/collection/playlists/created"), page("collection/playlists"));
        assert_eq!(link("music.yandex.ru/non-music"), page("non-music"));
        assert_eq!(link("https://music.yandex.ru/search?text=%D1%80%D0%BE%D0%BA+%26+roll"), Some(Link::Search("рок & roll".to_string())));
        assert_eq!(link("https://music.yandex.ru/search"), page("search"));
        assert_eq!(link("https://music.yandex.ru/tag/rock"), Some(Link::Tag("rock".to_string())));
        assert_eq!(link("https://music.yandex.ru/non-music/category/kids"), Some(Link::Tag(format!("{CATEGORY}kids"))));
        for other in ["metallica", "https://example.com/album/1", "https://music.yandex.ru/album/abc", "https://music.yandex.ru/settings/x", ""] {
            assert_eq!(link(other), None, "{other}");
        }
    }

    #[test]
    fn parses_similar() {
        let json = r#"{"result":{"items":[
            {"type":"wave_agent_item","data":{"agent":{"cover":{"uri":"a/%%"}},"wave":{"name":"Рок","seeds":["playlist:x_1"]}}},
            {"type":"playlist_item","data":{"playlist":{"kind":2311,"title":"Метал","uid":103372440}}},
            {"type":"unknown","data":{}}
        ]}}"#;
        let items = parse_similar(&serde_json::from_str(json).unwrap());
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], HomeItem::Wave { title, seeds, cover, .. } if title == "Рок" && seeds == &["playlist:x_1"] && cover.as_deref() == Some("a/%%")));
        assert!(matches!(&items[1], HomeItem::Playlist(list) if list.owner == "103372440" && list.kind == "2311"));
    }

    #[test]
    fn parses_credits() {
        let credits = |json: &str| parse_credits(&serde_json::from_str(json).unwrap());
        assert_eq!(credits(r#"{"result":{"credits":[{"title":"Лейбл","value":"А"},{"title":"Пусто","value":""}]}}"#), [("Лейбл".to_string(), "А".to_string())]);
        assert!(credits(r#"{"result":{}}"#).is_empty());
    }

    #[test]
    fn parses_filter_ids() {
        let ids = |json: &str| parse_filter_ids(&serde_json::from_str(json).unwrap());
        // Похожие треки из suggestedTrackIds в результат не входят.
        assert_eq!(ids(r#"{"result":{"playlistTrackIds":["1","a-b"],"suggestedTrackIds":["2"]}}"#), ["1", "a-b"]);
        assert!(ids(r#"{"result":{"playlistTrackIds":[]}}"#).is_empty());
    }

    #[test]
    fn parses_last_wave() {
        let last = |json: &str| parse_last_wave(&serde_json::from_str(json).unwrap());
        let mine = Some((vec![MY_WAVE.to_string()], None));
        assert_eq!(last(r#"{"name":"Моя волна","seeds":["user:onyourwave"]}"#), mine);
        // Настройки не считаются источником волны.
        assert_eq!(last(r#"{"seeds":["settingLanguage:russian"]}"#), mine);
        assert_eq!(last(r#"{"name":"Metallica","seeds":["artist:3121","settingLanguage:russian"]}"#), Some((vec!["artist:3121".to_string()], Some("Metallica".to_string()))));
        assert_eq!(last(r#"{}"#), None);
    }

    #[test]
    fn parses_lyrics() {
        let lines = parse_lrc("[00:01.50] раз\n[00:03.00]\n[01:00.00] два\n\nбез времени");
        assert_eq!(lines, [(Some(1.5), "раз".to_string()), (Some(60.0), "два".to_string()), (None, "без времени".to_string())]);
        let lyrics = Lyrics { lines, authors: Vec::new() };
        assert!(lyrics.synced());
        assert_eq!(lyrics.line_at(0.5), None);
        assert_eq!(lyrics.line_at(2.0), Some(0));
        assert_eq!(lyrics.line_at(90.0), Some(1));
        assert!(!Lyrics { lines: parse_lrc("текст"), authors: Vec::new() }.synced());
    }

    #[test]
    fn parses_home_blocks() {
        let v: Value = serde_json::from_str(r#"{"newReleases":[{"album":{"id":"1","title":"А","coverUri":"a/%%"},"artists":[{"name":"Имя"}]}]}"#).unwrap();
        assert!(matches!(&parse_home_block("Новое", &v).items[0], HomeItem::Album(card) if card.subtitle == "Имя"));
        let v: Value = serde_json::from_str(r#"{"items":[
            {"type":"liked_playlist_item","data":{"playlist":{"uid":"5","kind":"7","title":"П"}}},
            {"type":"artist_item","data":{"artist":{"id":"8","name":"Х"}}},
            {"type":"wave_item","data":{"seeds":["genre:rock"],"title":"Рок"}},
            {"type":"mix_card_item","data":{"id":"x","title":"М"}}]}"#).unwrap();
        let block = parse_home_block("Смесь", &v);
        assert_eq!(block.items.len(), 3);
        assert!(matches!(&block.items[0], HomeItem::Playlist(list) if list.owner == "5" && list.kind == "7"));
        let v: Value = serde_json::from_str(r#"{"waves":[{"items":[{"data":{"seeds":["a"],"title":"В"}}]},{"items":[{"data":{"seeds":["b"],"title":"Г"}}]}],
            "tracks":[{"track":{"id":"1","title":"Т","durationMs":1}},{"id":"2","title":"У","durationMs":1}]}"#).unwrap();
        let block = parse_home_block("Волны", &v);
        assert_eq!((block.items.len(), block.tracks.len()), (1, 2));
    }

    #[test]
    fn parses_mixed_search() {
        let v: Value = serde_json::from_str(r#"{"misspellCorrected":true,"text":"нойз","results":[
            {"type":"artist","artist":{"id":"1","name":"Имя"}},
            {"type":"track","track":{"id":"2","title":"Т","durationMs":1000}},
            {"type":"album","album":{"id":3,"title":"А"}},
            {"type":"playlist","playlist":{"uid":7,"kind":8,"title":"П","trackCount":2}},
            {"type":"clip","clip":{}}]}"#).unwrap();
        let found = parse_found(&v);
        assert_eq!((found.tracks.len(), found.albums.len(), found.artists.len()), (1, 1, 1));
        assert_eq!((found.playlists[0].owner.as_str(), found.playlists[0].kind.as_str()), ("7", "8"));
        assert_eq!(found.corrected.as_deref(), Some("нойз"));
        assert!(parse_found(&serde_json::json!({"misspellCorrected":false,"text":"а"})).corrected.is_none());
        let v: Value = serde_json::from_str(r##"{"results":[
            {"type":"podcast","podcast":{"id":4,"title":"К","type":"audiobook"}},
            {"type":"podcast_episode","podcast_episode":{"id":"5","title":"Глава","durationMs":1000}},
            {"type":"wave","wave":{"seeds":["genre:allrock"],"title":"Рок","agent":{"cover":{"uri":"w/%%","color":"#3779BC"}}}}]}"##).unwrap();
        let found = parse_found(&v);
        assert_eq!((found.albums[0].subtitle.as_str(), found.tracks.len()), ("Аудиокнига", 1));
        assert!(matches!(&found.waves[0], HomeItem::Wave { title, seeds, .. } if title == "Рок" && seeds == &["genre:allrock"]));
    }

    #[test]
    fn parses_artist_page_and_album() {
        let v: Value = serde_json::from_str(r#"{"artist":{"id":1,"name":"Имя","cover":{"uri":"a/%%"}},"stats":{"lastMonthListeners":5},
            "popularTracks":[{"id":"9","title":"Т","durationMs":1000,"artists":[{"id":1,"name":"Имя"}]}],
            "albums":[{"id":2,"title":"А","year":2012,"coverUri":"b/%%"},{"id":3}],
            "alsoAlbums":[{"id":4,"title":"Б"}],"similarArtists":[{"id":5,"name":"Другой"}]}"#).unwrap();
        let artist = parse_artist(&v).unwrap();
        assert_eq!((artist.id.as_str(), artist.listeners, artist.popular.len(), artist.albums.len()), ("1", 5, 1, 1));
        assert_eq!(artist.albums[0].subtitle, "2012");
        assert_eq!(artist.also[0].subtitle, "Альбом");
        assert_eq!(artist.similar[0].title, "Другой");
        assert!(parse_artist(&Value::Null).is_none());

        let v: Value = serde_json::from_str(r#"{"id":7,"title":"Альбом","year":2020,"coverUri":"c/%%","artists":[{"id":1,"name":"А"},{"id":2,"name":"Б"}]}"#).unwrap();
        let album = parse_album(&v).unwrap();
        assert_eq!(album.artists, [("А".to_string(), "1".to_string()), ("Б".to_string(), "2".to_string())]);
        assert_eq!(album.year, Some(2020));
    }

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
