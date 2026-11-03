//! Скачанные треки: файлы в папке приложения и список с их сведениями. Скачанный трек играет
//! с диска, без сети; его обложка лежит рядом, чтобы список выглядел так же и без интернета.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::api::{self, FileInfo, Sound, Track};
use crate::session;

const INDEX: &str = "index.json";
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

/// Обложка скачанного трека с диска. Для всего остального её здесь нет.
pub fn cover(id: &str) -> Option<Vec<u8>> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit() || c == ':') {
        return None;
    }
    fs::read(cover_path(id)).ok()
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
    fn names_and_sizes_read_well() {
        assert_eq!(safe("123:456"), "123-456");
        assert_eq!(size_label(13_002_342), "12,4 МБ");
        assert_eq!(size_label(1_288_490_189), "1,2 ГБ");
        assert!(cover("album:5").is_none());
    }
}
