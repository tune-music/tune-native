//! Яндекс Музыка: вход по коду, поиск и ссылка на файл трека.
//!
//! Те же запросы делает оригинальный клиент. Данные приложения общеизвестны, их используют
//! открытые клиенты (MarshalX/yandex-music-api).

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

const API: &str = "https://api.music.yandex.net";
const CLIENT: &str = "YandexMusicDesktopAppWindows/5.120.0";
const CLIENT_ID: &str = "23cabbbdc6cd418abb4b39c32c41195d";
const CLIENT_SECRET: &str = "53bc75238f0c4d08a118e51fe9203300";
const SIGN_KEY: &[u8] = b"kzqU4XhfCaY6B6JTHODeq5";
const QUALITY: &str = "nq";
const CODECS: [&str; 5] = ["mp3", "aac", "aac-mp4", "he-aac", "he-aac-mp4"];
const TRANSPORT: &str = "encraw";

/// Ошибка: токен отозвали или он истёк.
pub const NEED_LOGIN: &str = "NEED_LOGIN";

#[derive(Clone, Debug)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artists: String,
    pub duration: Duration,
    pub cover: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub url: String,
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

pub fn logout() {
    let _ = std::fs::remove_file(token_path());
}

/// Код для страницы подтверждения Яндекса.
pub fn login_start() -> Result<DeviceCode, String> {
    let body = agent()
        .post("https://oauth.yandex.ru/device/code")
        .send_form([("client_id", CLIENT_ID), ("device_name", "Tune Native")])
        .map_err(|e| format!("Не удалось связаться с Яндексом: {e}"))?
        .into_body()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let (Some(device_code), Some(user_code), Some(url)) = (v["device_code"].as_str(), v["user_code"].as_str(), v["verification_url"].as_str()) else {
        return Err("Яндекс не выдал код входа".into());
    };
    Ok(DeviceCode { device_code: device_code.into(), user_code: user_code.into(), url: url.into() })
}

/// Токен, когда вход подтверждён; None — ещё ждём.
pub fn login_poll(device_code: &str) -> Result<Option<String>, String> {
    let body = agent()
        .post("https://oauth.yandex.ru/token")
        .send_form([("grant_type", "device_code"), ("code", device_code), ("client_id", CLIENT_ID), ("client_secret", CLIENT_SECRET)])
        .map_err(|e| format!("Не удалось связаться с Яндексом: {e}"))?
        .into_body()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    if let Some(token) = v["access_token"].as_str() {
        std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
        std::fs::write(token_path(), token).map_err(|e| e.to_string())?;
        return Ok(Some(token.to_string()));
    }
    match v["error"].as_str() {
        Some("authorization_pending") => Ok(None),
        Some("expired_token") => Err("Код устарел, войдите ещё раз".into()),
        _ => Err("Яндекс не подтвердил вход".into()),
    }
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
    })
}

pub fn search(text: &str, token: &str) -> Result<Vec<Track>, String> {
    let v = get("search", &[("text", text), ("type", "track"), ("page", "0")], token)?;
    Ok(v["result"]["tracks"]["results"].as_array().into_iter().flatten().filter_map(parse_track).collect())
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

/// Обложка размером 50×50. Нет сети — трек просто останется без неё.
pub fn cover(uri: &str) -> Option<Vec<u8>> {
    let url = format!("https://{}", uri.replace("%%", "50x50"));
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
