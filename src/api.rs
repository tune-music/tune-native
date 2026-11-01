//! Яндекс Музыка: токен, поиск, библиотека, волна и ссылка на файл трека.
//!
//! Те же запросы делает оригинальный клиент. Данные приложения общеизвестны, их используют
//! открытые клиенты (MarshalX/yandex-music-api).

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[derive(Clone, Debug, PartialEq)]
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
    /// Пачка волны, из которой пришёл трек.
    pub batch: Option<String>,
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

/// Шапка альбома.
#[derive(Clone, Debug)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub year: Option<u64>,
    pub artists: Vec<(String, String)>,
    pub cover: Option<String>,
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
}

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
    })
}

fn parse_cards(v: &Value, card: impl Fn(&Value) -> Option<Card>) -> Vec<Card> {
    v.as_array().into_iter().flatten().filter_map(card).collect()
}

fn album_card(v: &Value) -> Option<Card> {
    Some(Card {
        id: id_text(&v["id"])?,
        title: v["title"].as_str()?.to_string(),
        subtitle: v["year"].as_u64().map_or_else(|| "Альбом".to_string(), |year| year.to_string()),
        cover: cover_uri(v),
    })
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
    })
}

pub fn album(id: &str, token: &str) -> Result<(Album, Vec<Track>), String> {
    let v = get(&format!("albums/{id}/with-tracks"), &[], token)?;
    let v = result(&v);
    let tracks = v["volumes"].as_array().into_iter().flatten().flat_map(|disk| disk.as_array().into_iter().flatten()).filter_map(parse_track).collect();
    Ok((parse_album(v).ok_or("Яндекс не прислал альбом")?, tracks))
}

pub fn artist(id: &str, token: &str) -> Result<Artist, String> {
    let v = get(&format!("artists/{id}/brief-info"), &[], token)?;
    parse_artist(result(&v)).ok_or_else(|| "Яндекс не прислал исполнителя".to_string())
}

/// Что нашёл смешанный поиск.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub tracks: Vec<Track>,
    pub albums: Vec<Card>,
    pub artists: Vec<Card>,
    pub playlists: Vec<Playlist>,
    /// Яндекс исправил опечатку и искал вот это.
    pub corrected: Option<String>,
}

fn parse_found(v: &Value) -> Found {
    let mut found = Found { corrected: v["text"].as_str().filter(|_| v["misspellCorrected"].as_bool() == Some(true)).map(str::to_string), ..Found::default() };
    for item in v["results"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("track") => found.tracks.extend(parse_track(&item["track"])),
            Some("album") => found.albums.extend(album_card(&item["album"])),
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

/// `types` — что искать через запятую: track, album, artist, playlist.
pub fn search_mixed(text: &str, types: &str, page_size: u32, token: &str) -> Result<Found, String> {
    let size = page_size.to_string();
    let v = get("search/instant/mixed", &[("text", text), ("type", types), ("page", "0"), ("pageSize", &size)], token)?;
    Ok(parse_found(result(&v)))
}

/// Альбомы и исполнители, которые нравятся пользователю.
pub fn liked_pages(uid: &str, token: &str) -> Result<(Vec<Card>, Vec<Card>), String> {
    let albums = get(&format!("users/{uid}/likes/albums"), &[("rich", "true")], token)?;
    let artists = get(&format!("users/{uid}/likes/artists"), &[("with-timestamps", "false")], token)?;
    Ok((parse_cards(result(&albums), |item| album_card(&item["album"])), parse_cards(result(&artists), artist_card)))
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
    Wave { title: String, seeds: Vec<String>, cover: Option<String> },
}

#[derive(Clone, Debug, Default)]
pub struct HomeBlock {
    pub title: String,
    pub items: Vec<HomeItem>,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Default)]
pub struct Home {
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
    let data = &v["data"];
    if data["playlist"].is_object() {
        return parse_home_playlist(&data["playlist"]);
    }
    if data["artist"].is_object() {
        return artist_card(&data["artist"]).map(HomeItem::Artist);
    }
    if data["album"].is_object() {
        return album_card(&data["album"]).map(HomeItem::Album);
    }
    let seeds: Vec<String> = data["seeds"].as_array()?.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
    if seeds.is_empty() {
        return None;
    }
    let cover = data["backgroundImageUrl"].as_str().or(data["imageUrl"].as_str()).map(str::to_string).or_else(|| cover_uri(data));
    Some(HomeItem::Wave { title: data["title"].as_str()?.to_string(), seeds, cover })
}

/// У блоков главной разные ответы, но элементы в них одни и те же.
fn parse_home_block(title: &str, v: &Value) -> HomeBlock {
    let mut items = Vec::new();
    items.extend(v["newReleases"].as_array().into_iter().flatten().filter_map(parse_release));
    items.extend(v["items"].as_array().into_iter().flatten().filter_map(parse_home_item));
    for group in ["waves", "inStyleTabs"] {
        // Вкладки блока пока не показываем: берём первую.
        items.extend(v[group][0]["items"].as_array().into_iter().flatten().filter_map(parse_home_item));
    }
    if v["playlist"].is_object() {
        items.extend(parse_home_playlist(&v["playlist"]));
    }
    let tracks = v["tracks"].as_array().into_iter().flatten().filter_map(|t| parse_track(if t["track"].is_object() { &t["track"] } else { t })).collect();
    HomeBlock { title: title.to_string(), items, tracks }
}

/// Главная: скелет страницы и все её блоки. Блоки грузятся одновременно, порядок сохраняется.
pub fn home(tab: usize, token: &str) -> Result<Home, String> {
    let skeleton = get("landing/skeleton/main", &[], token)?;
    let skeleton = result(&skeleton);
    let tabs = &skeleton["blocks"][0]["data"]["tabs"];
    let names = tabs.as_array().into_iter().flatten().filter_map(|t| t["title"].as_str().map(str::to_string)).collect();
    let sources: Vec<(String, String)> = tabs[tab]["blocks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| Some((b["data"]["title"].as_str().unwrap_or_default().to_string(), b["data"]["source"]["uri"].as_str()?.trim_start_matches('/').to_string())))
        .collect();
    let handles: Vec<_> = sources
        .into_iter()
        .map(|(title, uri)| {
            let token = token.to_string();
            std::thread::spawn(move || get(&uri, &[], &token).ok().map(|v| parse_home_block(&title, result(&v))))
        })
        .collect();
    let blocks = handles.into_iter().filter_map(|h| h.join().ok().flatten()).filter(|b| !b.title.is_empty() && (!b.items.is_empty() || !b.tracks.is_empty())).collect();
    Ok(Home { tabs: names, blocks })
}

/// Значение для адреса или формы.
fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn create_playlist(uid: &str, title: &str, token: &str) -> Result<Playlist, String> {
    let v = post(&format!("users/{uid}/playlists/create?visibility=private&title={}", encode(title)), "application/x-www-form-urlencoded", String::new(), token)?;
    parse_playlist(result(&v), uid).ok_or_else(|| "Яндекс не создал плейлист".to_string())
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

pub fn remove_from_playlist(uid: &str, kind: &str, track: &Track, token: &str) -> Result<(), String> {
    let state = playlist_state(uid, kind, token)?;
    let position = state["tracks"].as_array().into_iter().flatten().position(|item| id_text(&item["track"]["id"]).or_else(|| id_text(&item["id"])).as_deref() == Some(track.id.as_str())).ok_or("Трека уже нет в плейлисте")?;
    let diff = json!([{ "op": "delete", "from": position, "to": position + 1, "tracks": [track_ref(&track.id, track.album.as_deref())] }]);
    change_playlist(uid, kind, &diff, state["revision"].as_u64().unwrap_or(0), token)
}

/// Последние прослушанные треки, новые сначала и без повторов.
fn parse_history(v: &Value) -> Vec<String> {
    let mut seen = HashSet::new();
    v["historyTabs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|tab| tab["items"].as_array().into_iter().flatten())
        .flat_map(|item| item["tracks"].as_array().into_iter().flatten())
        .filter_map(|track| id_text(&track["data"]["itemId"]["trackId"]))
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

pub fn history(limit: usize, token: &str) -> Result<Vec<Track>, String> {
    let v = get("music-history", &[("fullModelsCount", "0")], token)?;
    let mut ids = parse_history(result(&v));
    ids.truncate(limit);
    tracks_by_ids(&ids, token)
}

/// Что можно закрепить в боковой панели.
#[derive(Clone, Debug, PartialEq)]
pub enum PinRef {
    Album(String),
    Artist(String),
    Playlist(String, String),
}

impl PinRef {
    fn path(&self) -> &'static str {
        match self {
            PinRef::Album(_) => "pin/album",
            PinRef::Artist(_) => "pin/artist",
            PinRef::Playlist(..) => "pin/playlist",
        }
    }

    fn body(&self) -> Value {
        match self {
            PinRef::Album(id) | PinRef::Artist(id) => json!({ "id": id }),
            PinRef::Playlist(uid, kind) => json!({ "uid": uid, "kind": kind }),
        }
    }

    pub fn matches(&self, pin: &Pin) -> bool {
        let PinTarget::List(list) = &pin.target else { return false };
        match self {
            PinRef::Album(id) => list.source == Source::Album && list.kind == *id,
            PinRef::Artist(id) => list.source == Source::Artist && list.kind == *id,
            PinRef::Playlist(uid, kind) => list.source == Source::Playlist && list.owner == *uid && list.kind == *kind,
        }
    }
}

/// Закрепляет или открепляет: тело запроса одно, а метод разный.
pub fn set_pinned(target: &PinRef, pinned: bool, token: &str) -> Result<(), String> {
    let url = format!("{API}/{}", target.path());
    let body = target.body().to_string();
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

pub fn file_info(track_id: &str, quality: &str, token: &str) -> Result<FileInfo, String> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs().to_string();
    let signature = sign(&format!("{ts}{track_id}{quality}{}{TRANSPORT}", CODECS.concat()));
    let codecs = CODECS.join(",");
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
    fn pin_reference_matches_its_target() {
        let list = |source, owner: &str, kind: &str| Pin { target: PinTarget::List(Playlist { source, owner: owner.to_string(), kind: kind.to_string(), title: String::new(), count: 0, cover: None }), title: String::new(), subtitle: "", cover: None, round: false };
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
    fn encodes_titles_and_reads_history() {
        assert_eq!(encode("а b&c"), "%D0%B0%20b%26c");
        let v = json!({"historyTabs":[
            {"items":[{"tracks":[{"data":{"itemId":{"trackId":"1"}}},{"data":{"itemId":{"trackId":"2"}}}]}]},
            {"items":[{"tracks":[{"data":{"itemId":{"trackId":"1"}}},{"data":{}},{"data":{"itemId":{"trackId":"3"}}}]}]}]});
        assert_eq!(parse_history(&v), ["1", "2", "3"]);
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
