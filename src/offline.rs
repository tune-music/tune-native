//! Скачанные треки: файлы в папке приложения и список с их сведениями. Скачанный трек играет
//! с диска, без сети; его обложка лежит рядом, чтобы список выглядел так же и без интернета.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::api::{self, FileInfo, Profile, Sound, Track};
use crate::session;

const INDEX: &str = "index.json";
/// Отчёты о прослушивании, которые не ушли без сети. Старые выкидываются сверх предела.
const PLAYS: &str = "plays.json";
const PLAYS_LIMIT: usize = 500;
/// Аккаунт и лайки с прошлого запуска: без сети приложение открывается с ними.
const ACCOUNT: &str = "account.json";
/// Отзывы волне, которые не ушли без сети: сессия, затравки и сам отзыв.
const FEEDBACKS: &str = "feedbacks.json";
/// Файл отчётов меняют сразу несколько потоков.
static PLAYS_LOCK: Mutex<()> = Mutex::new(());
/// Обложка скачанного трека: того же размера, что в строках списков.
const COVER_SIZE: &str = "200x200";

#[derive(Clone, Debug)]
pub struct Saved {
    pub track: Track,
    pub codec: String,
    pub bitrate: u64,
    pub quality: String,
    pub bytes: u64,
}

impl Saved {
    pub fn path(&self) -> PathBuf {
        dir().join(format!("{}.{}", safe(&self.track.id), safe(&self.codec)))
    }

    /// Подпись качества, как у трека из сети.
    pub fn sound(&self) -> Sound {
        FileInfo { url: String::new(), key: None, codec: self.codec.clone(), bitrate: self.bitrate, quality: self.quality.clone() }.sound()
    }
}

pub fn dir() -> PathBuf {
    api::data_dir().join("downloads")
}

/// Имя файла только из букв, цифр и дефиса: номер трека бывает вида `123:456`.
fn safe(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn cover_path(id: &str) -> PathBuf {
    dir().join(format!("{}.jpg", safe(id)))
}

fn value(saved: &Saved) -> Value {
    json!({ "track": session::track_value(&saved.track), "codec": saved.codec, "bitrate": saved.bitrate, "quality": saved.quality, "bytes": saved.bytes })
}

fn from(v: &Value) -> Option<Saved> {
    Some(Saved {
        track: session::track_from(&v["track"])?,
        codec: v["codec"].as_str()?.to_string(),
        bitrate: v["bitrate"].as_u64().unwrap_or(0),
        quality: v["quality"].as_str().unwrap_or_default().to_string(),
        bytes: v["bytes"].as_u64().unwrap_or(0),
    })
}

/// Скачанное, у чего файл на месте. Новые первыми.
pub fn load() -> Vec<Saved> {
    let Some(list) = fs::read(dir().join(INDEX)).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()) else { return Vec::new() };
    list.as_array().into_iter().flatten().filter_map(from).filter(|saved| saved.path().exists()).collect()
}

/// Сбой посреди записи не должен портить прошлый список.
pub fn save(list: &[Saved]) {
    let dir = dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let draft = dir.join(format!("{INDEX}.tmp"));
    let body = Value::Array(list.iter().map(value).collect()).to_string();
    if fs::write(&draft, body).is_ok() {
        let _ = fs::rename(&draft, dir.join(INDEX));
    }
}

/// Качает трек целиком в выбранном качестве вместе с обложкой. Блокирует поток.
pub fn download(track: &Track, quality: &str, token: &str) -> Result<Saved, String> {
    fs::create_dir_all(dir()).map_err(|_| "Не удалось создать папку для скачанного".to_string())?;
    let info = api::file_info(&track.id, quality, token)?;
    let saved = Saved { track: Track { batch: None, ..track.clone() }, codec: info.codec.clone(), bitrate: info.bitrate, quality: info.quality.clone(), bytes: 0 };
    let part = dir().join(format!("{}.part", safe(&track.id)));
    let bytes = match crate::player::save_file(&info, &part) {
        Ok(bytes) => bytes,
        Err(e) => {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
    };
    fs::rename(&part, saved.path()).map_err(|_| "Не удалось сохранить файл трека".to_string())?;
    if let Some(picture) = track.cover.as_deref().and_then(|uri| api::cover(uri, COVER_SIZE)) {
        let _ = fs::write(cover_path(&track.id), picture);
    }
    Ok(Saved { bytes, ..saved })
}

pub fn remove(saved: &Saved) {
    let _ = fs::remove_file(saved.path());
    let _ = fs::remove_file(cover_path(&saved.track.id));
}

/// Номер трека, а не ключ другой обложки.
pub fn is_track(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_digit() || c == ':')
}

/// Обложка с диска: у скачанного трека — рядом с файлом, у закреплённого и аватарки — среди
/// сохранённых обложек.
pub fn cover(id: &str) -> Option<Vec<u8>> {
    if is_track(id) {
        return fs::read(cover_path(id)).ok();
    }
    fs::read(api::data_dir().join("covers").join(format!("{}.jpg", safe(id)))).ok()
}

/// Сохраняет обложку, которая нужна и без сети: закреплённого и аватарки.
pub fn keep_cover(id: &str, bytes: &[u8]) {
    let dir = api::data_dir().join("covers");
    if fs::create_dir_all(&dir).is_ok() {
        let _ = fs::write(dir.join(format!("{}.jpg", safe(id))), bytes);
    }
}

fn read_json(name: &str) -> Option<Value> {
    serde_json::from_slice(&fs::read(api::data_dir().join(name)).ok()?).ok()
}

fn write_json(name: &str, value: &Value) {
    let dir = api::data_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let draft = dir.join(format!("{name}.tmp"));
    if fs::write(&draft, value.to_string()).is_ok() {
        let _ = fs::rename(&draft, dir.join(name));
    }
}

/// Откладывает отчёты, которые не ушли без сети.
pub fn keep_plays(reports: Vec<Value>) {
    let _guard = PLAYS_LOCK.lock().unwrap();
    let mut list = read_json(PLAYS).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    list.extend(reports);
    let extra = list.len().saturating_sub(PLAYS_LIMIT);
    list.drain(..extra);
    write_json(PLAYS, &Value::Array(list));
}

/// Откладывает отзыв волне, который не ушёл без сети.
pub fn keep_feedback(session: &str, seeds: &[String], feedback: Value) {
    let _guard = PLAYS_LOCK.lock().unwrap();
    let mut list = read_json(FEEDBACKS).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    list.push(serde_json::json!({ "session": session, "seeds": seeds, "feedback": feedback }));
    let extra = list.len().saturating_sub(PLAYS_LIMIT);
    list.drain(..extra);
    write_json(FEEDBACKS, &Value::Array(list));
}

/// Отзывы по сессиям в порядке, в каком они случились.
fn group_feedbacks(list: &[Value]) -> Vec<(String, Vec<String>, Vec<Value>)> {
    let mut sessions: Vec<(String, Vec<String>, Vec<Value>)> = Vec::new();
    for item in list {
        let Some(session) = item["session"].as_str() else { continue };
        let seeds = item["seeds"].as_array().into_iter().flatten().filter_map(|seed| seed.as_str().map(str::to_string)).collect();
        match sessions.iter_mut().find(|(known, _, _)| known == session) {
            Some((_, _, feedbacks)) => feedbacks.push(item["feedback"].clone()),
            None => sessions.push((session.to_string(), seeds, vec![item["feedback"].clone()])),
        }
    }
    sessions
}

fn flush_feedbacks(token: &str) {
    let list = {
        let _guard = PLAYS_LOCK.lock().unwrap();
        let list = read_json(FEEDBACKS).and_then(|v| v.as_array().cloned()).unwrap_or_default();
        if !list.is_empty() {
            let _ = fs::remove_file(api::data_dir().join(FEEDBACKS));
        }
        list
    };
    if list.is_empty() {
        return;
    }
    if let Err(e) = api::wave_feedbacks(&group_feedbacks(&list), token) {
        if api::network_error(&e) {
            let _guard = PLAYS_LOCK.lock().unwrap();
            let mut kept = read_json(FEEDBACKS).and_then(|v| v.as_array().cloned()).unwrap_or_default();
            kept.splice(0..0, list);
            write_json(FEEDBACKS, &Value::Array(kept));
        } else {
            eprintln!("отложенные отзывы волне не приняты: {e}");
        }
    }
}

/// Отправляет отложенные отчёты в фоне. Не дошли из-за сети — останутся до следующего раза;
/// Яндекс отказал — выкидываются, иначе застрянут навсегда.
pub fn flush_plays(token: String) {
    std::thread::spawn(move || {
        flush_feedbacks(&token);
        let list = {
            let _guard = PLAYS_LOCK.lock().unwrap();
            let list = read_json(PLAYS).and_then(|v| v.as_array().cloned()).unwrap_or_default();
            if !list.is_empty() {
                let _ = fs::remove_file(api::data_dir().join(PLAYS));
            }
            list
        };
        if list.is_empty() {
            return;
        }
        if let Err(e) = api::plays(list.clone(), &token) {
            if api::network_error(&e) {
                keep_plays(list);
            } else {
                eprintln!("отложенные отчёты не приняты: {e}");
            }
        }
    });
}

/// Сколько отчётов ждут отправки.
pub fn plays_waiting() -> usize {
    read_json(PLAYS).and_then(|v| v.as_array().map(Vec::len)).unwrap_or(0)
}

pub fn save_account(profile: &Profile, liked: &[String], disliked: &[String]) {
    let value = json!({
        "uid": profile.uid, "name": profile.name, "plus": profile.plus, "login": profile.login, "email": profile.email,
        "avatar": profile.avatar, "plusUntil": profile.plus_until, "family": profile.family,
        "liked": liked, "disliked": disliked,
    });
    write_json(ACCOUNT, &value);
}

pub fn load_account() -> Option<(Profile, Vec<String>, Vec<String>)> {
    let v = read_json(ACCOUNT)?;
    let text = |key: &str| v[key].as_str().unwrap_or_default().to_string();
    let ids = |key: &str| v[key].as_array().into_iter().flatten().filter_map(|id| id.as_str().map(str::to_string)).collect::<Vec<_>>();
    let profile = Profile {
        uid: v["uid"].as_str()?.to_string(),
        name: text("name"),
        plus: v["plus"].as_bool().unwrap_or(false),
        login: text("login"),
        email: text("email"),
        avatar: v["avatar"].as_str().map(str::to_string),
        plus_until: v["plusUntil"].as_str().map(str::to_string),
        family: v["family"].as_bool().unwrap_or(false),
    };
    Some((profile, ids("liked"), ids("disliked")))
}

/// Размер словами: «12,4 МБ», «1,2 ГБ».
pub fn size_label(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    let text = if mb >= 1024.0 { format!("{:.1} ГБ", mb / 1024.0) } else { format!("{mb:.1} МБ") };
    text.replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedbacks_group_by_session() {
        let item = |session: &str, kind: &str| serde_json::json!({"session": session, "seeds": ["user:onyourwave"], "feedback": {"event": {"type": kind}}});
        let groups = group_feedbacks(&[item("a", "trackStarted"), item("b", "radioStarted"), item("a", "skip")]);
        assert_eq!(groups.len(), 2);
        assert_eq!((groups[0].0.as_str(), groups[0].2.len(), groups[0].1.as_slice()), ("a", 2, ["user:onyourwave".to_string()].as_slice()));
        assert_eq!(groups[0].2[1]["event"]["type"], "skip");
    }

    #[test]
    fn names_and_sizes_read_well() {
        assert_eq!(safe("123:456"), "123-456");
        assert_eq!(size_label(13_002_342), "12,4 МБ");
        assert_eq!(size_label(1_288_490_189), "1,2 ГБ");
        assert!(is_track("123:456") && !is_track("pin:album:5") && !is_track(""));
    }
}
