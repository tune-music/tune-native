//! «Без цензуры на всех устройствах», как в Tune Client. Версия без цензуры играет только в
//! Tune, а свои загруженные треки Яндекс отдаёт везде, в том числе в приложении на телефоне.
//! Поэтому версия без цензуры качается (с тегами и обложкой, как «Скачать в файл») и
//! загружается в плейлист «Загруженное» тем же путём, что «Загрузить трек». Если оригинал
//! был в «Мне нравится», лайк переезжает на копию. Дальше копия — обычный свой трек.
//!
//! Какой оригинал уже загружен, помнится на диске и сверяется с «Загруженным»: Яндекс сам
//! сопоставляет загруженный файл с треком каталога (`matchedTrack`). Копию, удалённую из
//! «Загруженного», можно загрузить снова.

use std::collections::HashMap;
use std::time::Duration;

use crate::api::{self, Track};

const FILE: &str = "uc-uploaded.json";
/// Плейлист, куда Яндекс кладёт загруженные треки.
const UPLOADS: &str = "Загруженное";
/// Сколько ждать, пока Яндекс обработает файл: обычно полминуты.
const READY_CHECKS: usize = 60;
const READY_EVERY: Duration = Duration::from_secs(3);

fn path() -> std::path::PathBuf {
    api::data_dir().join(FILE)
}

/// Оригинал → загруженная копия.
pub fn uploaded() -> HashMap<String, String> {
    std::fs::read(path()).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn store(map: &HashMap<String, String>) {
    let _ = std::fs::create_dir_all(api::data_dir());
    let _ = std::fs::write(path(), serde_json::to_string(map).unwrap_or_default());
}

/// Трек можно загрузить без цензуры: у него есть версия без цензуры или свой файл.
pub fn can(track: &Track) -> bool {
    !track.ugc && !track.id.is_empty() && track.id.bytes().all(|b| b.is_ascii_digit()) && crate::uncensor::file(&track.id).is_some()
}

fn uploads_kind(uid: &str, token: &str) -> Result<String, String> {
    api::playlists(uid, token)?.into_iter().find(|list| list.title == UPLOADS).map(|list| list.kind).ok_or_else(|| "Нет плейлиста «Загруженное»: загрузите любой свой трек в Яндекс Музыке, и он появится".to_string())
}

/// Сверяет, что лежит в «Загруженном», и пересобирает список загруженных оригиналов.
pub fn sync(uid: &str, token: &str) -> Result<HashMap<String, String>, String> {
    let kind = uploads_kind(uid, token)?;
    let present: Vec<String> = api::playlist_track_ids(uid, &kind, token)?.into_iter().filter(|id| !id.bytes().all(|b| b.is_ascii_digit())).collect();
    let mut map: HashMap<String, String> = api::matched_tracks(&present, token)?.into_iter().map(|(copy, original)| (original, copy)).collect();
    for (original, copy) in uploaded() {
        if present.contains(&copy) {
            map.entry(original).or_insert(copy);
        }
    }
    store(&map);
    Ok(map)
}

/// Загружает один трек. Вернёт номер копии и перенесён ли лайк. Блокирует: зовётся в фоне.
pub fn upload(track: &Track, liked: bool, uid: &str, quality: &str, token: &str) -> Result<(String, bool), String> {
    let kind = uploads_kind(uid, token)?;
    let dir = std::env::temp_dir().join(format!("tune-uncensored-{}", std::process::id()));
    let file = crate::export::save_to(track, quality, token, &dir).map_err(|e| format!("Не удалось скачать файл без цензуры: {e}"))?;
    let sent = api::upload_track(uid, &kind, &file, token);
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir(&dir);
    let copy = sent?;
    let mut ready = false;
    for _ in 0..READY_CHECKS {
        std::thread::sleep(READY_EVERY);
        if api::uploaded_ready(std::slice::from_ref(&copy), token).is_ok_and(|done| done.contains(&copy)) {
            ready = true;
            break;
        }
    }
    if !ready {
        return Err("Файл загружен, но Яндекс ещё обрабатывает его: трек появится в «Загруженном» позже".to_string());
    }
    let mut map = uploaded();
    map.insert(track.id.clone(), copy.clone());
    store(&map);
    // Название и исполнитель как у подмены: Яндекс берёт их из файла, но не всегда верно.
    if let Err(e) = api::change_uploaded(&copy, &track.artists, &track.title, token) {
        api::log(&format!("Без цензуры на всех устройствах: название не сохранилось: {e}"));
    }
    if liked {
        api::set_liked(uid, &copy, true, token)?;
        api::set_liked(uid, &track.id, false, token)?;
    }
    Ok((copy, liked))
}
