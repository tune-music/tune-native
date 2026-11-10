//! Темы Tune: свой вид поверх обычного. Пока одна — Aquarium, как тема Tune Client: живой
//! аквариум с обложки sqwore на фоне, бирюзовая палитра, стеклянные панели и пузыри.
//!
//! Видео темы (20 МБ) в программу не вшито. Если Tune Client уже скачал тему, файл
//! копируется из его папки, иначе один раз скачивается из репозитория тем. Пока видео нет
//! или живой фон выключен, на фоне стоит первый кадр — он вшит.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{image, svg};

const VIDEO_URL: &str = "https://raw.githubusercontent.com/absolute2007/next-music-themes/main/themes/Aquarium/assets/bg.mp4";
/// Короче — файл недокачан.
const VIDEO_MIN: u64 = 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// Логотип темы для полосы заголовка: рыба в пузыре, как в Tune Client.
const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><defs><radialGradient id="o" cx="0.4" cy="0.25" r="0.8"><stop offset="0" stop-color="#b9fff4"/><stop offset="0.5" stop-color="#2bb8c4"/><stop offset="1" stop-color="#06374a"/></radialGradient></defs><circle cx="32" cy="32" r="28" fill="url(#o)" stroke="#d8fffa" stroke-opacity="0.7" stroke-width="2"/><path d="M12 34 C20 22 40 20 50 28 L58 22 L56 33 L59 42 L50 37 C40 46 20 44 12 34 Z" fill="#e9fffc" fill-opacity="0.92"/><circle cx="20" cy="31" r="1.8" fill="#06374a"/><circle cx="24" cy="14" r="2.6" fill="none" stroke="#e9fffc" stroke-width="1.4"/><circle cx="31" cy="9" r="1.6" fill="none" stroke="#e9fffc" stroke-width="1.2"/></svg>"##;

pub fn video_path() -> PathBuf {
    crate::api::data_dir().join("themes").join("aquarium").join("bg.mp4")
}

/// Видео уже на диске.
pub fn video_ready() -> bool {
    fs::metadata(video_path()).is_ok_and(|meta| meta.len() >= VIDEO_MIN)
}

/// Кладёт видео темы на диск: из папки Tune Client или из сети. Блокирует: зовётся в фоне.
pub fn ensure_video() -> Result<PathBuf, String> {
    let path = video_path();
    if video_ready() {
        return Ok(path);
    }
    let dir = path.parent().ok_or("Нет папки темы")?;
    fs::create_dir_all(dir).map_err(|e| format!("Не удалось создать папку темы: {e}"))?;
    let part = path.with_extension("part");
    let client = std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("dev.tune.music").join("addons").join("Aquarium").join("assets").join("bg.mp4"));
    let copied = client.filter(|source| fs::metadata(source).is_ok_and(|meta| meta.len() >= VIDEO_MIN)).is_some_and(|source| fs::copy(source, &part).is_ok());
    if !copied {
        download(&part)?;
    }
    if fs::metadata(&part).map_or(0, |meta| meta.len()) < VIDEO_MIN {
        let _ = fs::remove_file(&part);
        return Err("Видео темы скачалось не целиком".to_string());
    }
    fs::rename(&part, &path).map_err(|e| format!("Не удалось сохранить видео темы: {e}"))?;
    Ok(path)
}

fn download(part: &std::path::Path) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(DOWNLOAD_TIMEOUT)).build().into();
    let response = agent.get(VIDEO_URL).header("User-Agent", "TuneNative").call().map_err(|e| format!("Видео темы не скачалось: {e}"))?;
    let mut body = response.into_body();
    let mut reader = body.as_reader();
    let mut file = File::create(part).map_err(|e| e.to_string())?;
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut buffer).map_err(|e| format!("Видео темы не скачалось: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
    }
}

/// Первый кадр видео: стоит на фоне, пока видео грузится или если живой фон выключен.
pub fn poster() -> image::Handle {
    static POSTER: std::sync::OnceLock<image::Handle> = std::sync::OnceLock::new();
    POSTER.get_or_init(|| image::Handle::from_bytes(include_bytes!("../assets/aquarium-poster.jpg").as_slice())).clone()
}

pub fn logo() -> svg::Handle {
    static HANDLE: std::sync::OnceLock<svg::Handle> = std::sync::OnceLock::new();
    HANDLE.get_or_init(|| svg::Handle::from_memory(LOGO.as_bytes())).clone()
}
