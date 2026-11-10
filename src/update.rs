//! Обновления из релизов github.com/tune-music/tune-native и окно «Что нового».
//!
//! Tune сверяет последний релиз с собственной версией, скачивает установщик
//! `Tune-Native-<версия>-setup.exe`, сверяет размер и SHA-256 из релиза и запускает его тихо
//! (`/S /R`). Установщик ждёт, пока Tune закроется, ставит новую версию и запускает её.
//!
//! Что изменилось, лежит в `CHANGES.md` и вшито в программу: окно «Что нового» после
//! обновления открывается и без сети. Состояние — в `update.json`: последняя версия, которую
//! пользователь уже видел, пропущенная версия и автопроверка.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const LATEST: &str = "https://api.github.com/repos/tune-music/tune-native/releases/latest";
/// Файлы берутся только из релизов самого Tune Native.
const DOWNLOADS: &str = "https://github.com/tune-music/tune-native/releases/download/";
pub const RELEASES_PAGE: &str = "https://github.com/tune-music/tune-native/releases";
const SETUP_SUFFIX: &str = "-setup.exe";
const USER_AGENT: &str = "TuneNative";
const CHANGES: &str = include_str!("../CHANGES.md");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub size: u64,
    /// SHA-256 файла из релиза (GitHub считает его сам), если он есть.
    pub sha256: Option<String>,
}

/// "v0.2.10" → [0, 2, 10]; всё после первой не-цифры в части отбрасывается.
fn parse_version(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches(['v', 'V']).split('.').map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)).collect()
}

pub fn newer(latest: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse_version(latest), parse_version(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// Tune поставлен установщиком: рядом с ним лежит деинсталлятор.
pub fn installed() -> bool {
    std::env::current_exe().is_ok_and(|exe| exe.with_file_name("uninstall.exe").exists())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(20))).build().into()
}

/// Последний релиз, если он новее запущенного Tune.
pub fn check() -> Result<Option<Release>, String> {
    let mut response = agent()
        .get(LATEST)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => "Релизов пока нет".to_string(),
            e => format!("Не удалось проверить обновления: {e}"),
        })?;
    let text = response.body_mut().read_to_string().map_err(|e| format!("Не удалось проверить обновления: {e}"))?;
    let release: Value = serde_json::from_str(&text).map_err(|e| format!("Не удалось проверить обновления: {e}"))?;
    parse_release(&release, VERSION)
}

fn parse_release(release: &Value, current: &str) -> Result<Option<Release>, String> {
    let version = release["tag_name"].as_str().unwrap_or_default().trim_start_matches(['v', 'V']).to_string();
    if version.is_empty() || !newer(&version, current) {
        return Ok(None);
    }
    let asset = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|asset| asset["name"].as_str().is_some_and(|name| name.ends_with(SETUP_SUFFIX)))
        .ok_or("В новом релизе нет установщика")?;
    let url = asset["browser_download_url"].as_str().unwrap_or_default().to_string();
    if !url.starts_with(DOWNLOADS) {
        return Err("Ссылка на обновление ведёт не в релизы Tune Native".to_string());
    }
    Ok(Some(Release {
        version,
        notes: release["body"].as_str().unwrap_or_default().trim().to_string(),
        url,
        size: asset["size"].as_u64().unwrap_or(0),
        sha256: asset["digest"].as_str().and_then(|digest| digest.strip_prefix("sha256:")).map(str::to_lowercase),
    }))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Скачивает установщик; `progress(скачано, всего)`.
fn fetch(release: &Release, dest: &Path, progress: &impl Fn(u64, u64)) -> Result<(), String> {
    let mut response = agent().get(&release.url).header("User-Agent", USER_AGENT).call().map_err(|e| format!("Не удалось скачать обновление: {e}"))?;
    let total = release.size;
    let part = dest.with_extension("part");
    let result = (|| {
        let mut out = File::create(&part).map_err(|e| e.to_string())?;
        let mut body = response.body_mut().with_config().limit(1024 * 1024 * 1024).reader();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut loaded = 0u64;
        let mut hash = Sha256::new();
        progress(0, total);
        loop {
            let n = body.read(&mut buffer).map_err(|e| format!("Загрузка прервалась: {e}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            hash.update(&buffer[..n]);
            loaded += n as u64;
            progress(loaded, total);
        }
        out.flush().map_err(|e| e.to_string())?;
        if total > 0 && loaded != total {
            return Err("Обновление скачалось не полностью".to_string());
        }
        if release.sha256.as_ref().is_some_and(|want| *want != hex(&hash.finalize())) {
            return Err("Скачанный файл не совпал с релизом".to_string());
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    fs::rename(&part, dest).map_err(|e| e.to_string())
}

fn download_dir() -> PathBuf {
    std::env::temp_dir().join("tune-native-update")
}

/// Скачивает и запускает установщик. После `Ok` Tune должен сразу выйти: установщик ждёт,
/// пока он закроется, и запускает новую версию сам.
pub fn install(release: &Release, progress: impl Fn(u64, u64)) -> Result<(), String> {
    let dir = download_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let setup = dir.join(format!("Tune-Native-{}{SETUP_SUFFIX}", release.version));
    fetch(release, &setup, &progress)?;
    // /S — без окон; /R — запустить Tune после установки. Неустановленный Tune (собранный
    // вручную) ставится в обычную папку программ.
    std::process::Command::new(&setup).args(["/S", "/R"]).spawn().map_err(|e| format!("Не удалось запустить установщик: {e}"))?;
    Ok(())
}

/// Уборка после прошлого обновления: скачанные установщики.
pub fn cleanup() {
    let _ = fs::remove_dir_all(download_dir());
}

/// Что хранится между запусками.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub auto: bool,
    /// Последняя версия, о которой пользователь уже знает: для «Что нового».
    pub seen: Option<String>,
    /// Версия, которую попросили не предлагать.
    pub skipped: Option<String>,
}

fn state_path() -> PathBuf {
    crate::api::data_dir().join("update.json")
}

pub fn load() -> State {
    let value: Value = fs::read(state_path()).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or(Value::Null);
    State {
        auto: value["auto"].as_bool().unwrap_or(true),
        seen: value["seen"].as_str().map(str::to_string),
        skipped: value["skipped"].as_str().map(str::to_string),
    }
}

pub fn save(state: &State) {
    let value = json!({ "auto": state.auto, "seen": state.seen, "skipped": state.skipped });
    let _ = fs::create_dir_all(crate::api::data_dir());
    let _ = fs::write(state_path(), value.to_string());
}

/// Изменения версий из `CHANGES.md`, новые сверху: (версия, пункты).
fn changes() -> Vec<(String, Vec<String>)> {
    let mut versions: Vec<(String, Vec<String>)> = Vec::new();
    for line in CHANGES.lines() {
        let line = line.trim_end();
        if let Some(version) = line.strip_prefix("## ") {
            versions.push((version.trim().trim_start_matches(['v', 'V']).to_string(), Vec::new()));
        } else if let (Some(item), Some((_, items))) = (line.trim_start().strip_prefix("- "), versions.last_mut()) {
            items.push(item.trim().to_string());
        }
    }
    versions
}

/// Что нового с версии `since` до запущенной; пусто — показывать нечего.
pub fn news_since(since: &str) -> Vec<(String, Vec<String>)> {
    news_between(since, VERSION)
}

fn news_between(since: &str, current: &str) -> Vec<(String, Vec<String>)> {
    changes().into_iter().filter(|(version, items)| !items.is_empty() && newer(version, since) && !newer(version, current)).collect()
}

/// Изменения запущенной версии: для кнопки «Что нового» в настройках.
pub fn news_current() -> Vec<(String, Vec<String>)> {
    changes().into_iter().filter(|(version, _)| version == VERSION).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("v0.1.10", "0.1.9"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn reads_release() {
        let release = json!({
            "tag_name": "v0.2.0",
            "body": "Новое",
            "assets": [
                { "name": "notes.txt", "browser_download_url": "https://github.com/tune-music/tune-native/releases/download/v0.2.0/notes.txt", "size": 1 },
                { "name": "Tune-Native-0.2.0-setup.exe", "browser_download_url": "https://github.com/tune-music/tune-native/releases/download/v0.2.0/Tune-Native-0.2.0-setup.exe", "size": 42, "digest": "sha256:ABCD" }
            ]
        });
        let found = parse_release(&release, "0.1.0").unwrap().unwrap();
        assert_eq!(found.version, "0.2.0");
        assert_eq!(found.size, 42);
        assert_eq!(found.sha256.as_deref(), Some("abcd"));
        assert_eq!(parse_release(&release, "0.2.0").unwrap(), None);
    }

    #[test]
    fn refuses_foreign_links() {
        let release = json!({ "tag_name": "v9.0.0", "assets": [{ "name": "x-setup.exe", "browser_download_url": "https://example.com/x-setup.exe", "size": 1 }] });
        assert!(parse_release(&release, "0.1.0").is_err());
    }

    #[test]
    fn changes_are_listed() {
        assert!(changes().iter().any(|(version, items)| version == VERSION && !items.is_empty()), "в CHANGES.md нет пунктов для {VERSION}");
    }

    #[test]
    fn news_between_versions() {
        let all = changes();
        if let Some((oldest, _)) = all.last() {
            assert!(news_between(oldest, oldest).is_empty());
        }
        assert!(!news_between("0.0.0", VERSION).is_empty());
    }
}
