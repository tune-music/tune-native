//! Что играло при выходе: очередь и место в треке. Очередь меняется редко, а место каждые
//! несколько секунд, поэтому они лежат в разных файлах.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use crate::api::{self, Track};

const QUEUE_FILE: &str = "queue.json";
const PLACE_FILE: &str = "place.json";
/// Недавние запросы поиска. Как и у официального клиента, они хранятся только на компьютере.
const SEARCH_FILE: &str = "searches.json";
const SEARCH_LIMIT: usize = 20;

pub struct Saved {
    pub tracks: Vec<Track>,
    pub title: String,
    /// Зёрна волны, если играла волна, а не список.
    pub wave: Option<Vec<String>>,
    pub current: usize,
    pub position: Duration,
}

fn track_value(track: &Track) -> Value {
    json!({
        "id": track.id,
        "title": track.title,
        "artists": track.artists,
        "ms": track.duration.as_millis() as u64,
        "cover": track.cover,
        "album": track.album,
        "albumTitle": track.album_title,
        "artistList": track.artist_list,
        "explicit": track.explicit,
        "lyrics": track.lyrics,
        "unavailable": track.unavailable,
        "fade": track.fade.map(|fade| json!({ "inStart": fade.in_start, "inStop": fade.in_stop, "outStart": fade.out_start, "outStop": fade.out_stop })),
    })
}

fn track_from(v: &Value) -> Option<Track> {
    Some(Track {
        id: v["id"].as_str()?.to_string(),
        title: v["title"].as_str()?.to_string(),
        artists: v["artists"].as_str().unwrap_or_default().to_string(),
        duration: Duration::from_millis(v["ms"].as_u64().unwrap_or(0)),
        cover: v["cover"].as_str().map(str::to_string),
        album: v["album"].as_str().map(str::to_string),
        album_title: v["albumTitle"].as_str().unwrap_or_default().to_string(),
        artist_list: serde_json::from_value(v["artistList"].clone()).unwrap_or_default(),
        explicit: v["explicit"].as_bool().unwrap_or(false),
        lyrics: v["lyrics"].as_bool().unwrap_or(false),
        batch: None,
        unavailable: v["unavailable"].as_bool().unwrap_or(false),
        fade: api::Fade::parse(&v["fade"]),
    })
}

/// Сбой посреди записи не должен портить прошлый файл.
fn write(dir: &Path, name: &str, value: &Value) {
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let draft = dir.join(format!("{name}.tmp"));
    if fs::write(&draft, value.to_string()).is_ok() {
        let _ = fs::rename(&draft, dir.join(name));
    }
}

fn read(dir: &Path, name: &str) -> Option<Value> {
    serde_json::from_slice(&fs::read(dir.join(name)).ok()?).ok()
}

fn load_from(dir: &Path) -> Option<Saved> {
    let queue = read(dir, QUEUE_FILE)?;
    let tracks: Vec<Track> = queue["tracks"].as_array()?.iter().filter_map(track_from).collect();
    let place = read(dir, PLACE_FILE).unwrap_or(Value::Null);
    let current = place["current"].as_u64().unwrap_or(0) as usize;
    // Трек мог исчезнуть из файла: тогда лучше начать очередь сначала, чем не открыть её.
    let (current, position) = if current < tracks.len() { (current, place["seconds"].as_f64().unwrap_or(0.0).max(0.0)) } else { (0, 0.0) };
    if tracks.is_empty() {
        return None;
    }
    let position = Duration::from_secs_f64(position).min(tracks[current].duration);
    let wave = serde_json::from_value(queue["wave"].clone()).ok();
    Some(Saved { title: queue["title"].as_str().unwrap_or_default().to_string(), wave, tracks, current, position })
}

pub fn load() -> Option<Saved> {
    load_from(&api::data_dir())
}

/// Запись идёт в фоне, чтобы диск не задерживал музыку. При выходе её надо дождаться.
fn run(wait: bool, job: impl FnOnce() + Send + 'static) {
    let job = std::thread::spawn(job);
    if wait {
        let _ = job.join();
    }
}

pub fn save_queue(tracks: Vec<Track>, title: String, wave: Option<Vec<String>>, wait: bool) {
    run(wait, move || {
        let tracks: Vec<Value> = tracks.iter().map(track_value).collect();
        write(&api::data_dir(), QUEUE_FILE, &json!({ "title": title, "wave": wave, "tracks": tracks }));
    });
}

pub fn save_place(current: usize, position: Duration, wait: bool) {
    run(wait, move || write(&api::data_dir(), PLACE_FILE, &json!({ "current": current, "seconds": position.as_secs_f64() })));
}

pub fn searches() -> Vec<String> {
    read(&api::data_dir(), SEARCH_FILE).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

pub fn save_searches(list: Vec<String>) {
    run(false, move || write(&api::data_dir(), SEARCH_FILE, &json!(list)));
}

/// Запрос встаёт первым; тот же запрос в другом регистре не повторяется.
pub fn remember_search(list: &[String], query: &str) -> Vec<String> {
    let query = query.trim();
    let same = query.to_lowercase();
    let rest = list.iter().filter(|old| old.to_lowercase() != same).cloned();
    std::iter::once(query.to_string()).filter(|q| !q.is_empty()).chain(rest).take(SEARCH_LIMIT).collect()
}

pub fn clear() {
    let dir = api::data_dir();
    let _ = fs::remove_file(dir.join(QUEUE_FILE));
    let _ = fs::remove_file(dir.join(PLACE_FILE));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, seconds: u64) -> Track {
        Track {
            id: id.to_string(),
            title: format!("Песня {id}"),
            artists: "Группа".to_string(),
            duration: Duration::from_secs(seconds),
            cover: Some("avatars/%%".to_string()),
            album: Some("7".to_string()),
            album_title: "Альбом".to_string(),
            artist_list: vec![("Группа".to_string(), "5".to_string())],
            explicit: true,
            lyrics: true,
            batch: None,
            unavailable: false,
            fade: Some(api::Fade { in_start: 0.0, in_stop: 1.5, out_start: seconds as f32 - 5.0, out_stop: seconds as f32 }),
        }
    }

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tune-session-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn restores_queue_and_place() {
        let dir = folder("full");
        let tracks = [track("1", 100), track("2", 200)];
        write(&dir, QUEUE_FILE, &json!({ "title": "Лист", "tracks": tracks.iter().map(track_value).collect::<Vec<_>>() }));
        write(&dir, PLACE_FILE, &json!({ "current": 1, "seconds": 42.5 }));
        let saved = load_from(&dir).unwrap();
        assert_eq!(saved.title, "Лист");
        assert_eq!(saved.tracks, tracks);
        assert_eq!(saved.current, 1);
        assert_eq!(saved.position, Duration::from_secs_f64(42.5));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remembers_that_a_wave_was_playing() {
        let dir = folder("wave");
        let tracks = vec![track_value(&track("1", 100))];
        write(&dir, QUEUE_FILE, &json!({ "title": "Волна", "wave": ["user:onyourwave"], "tracks": tracks }));
        assert_eq!(load_from(&dir).unwrap().wave, Some(vec!["user:onyourwave".to_string()]));
        // Ни списку, ни файлу прошлой версии зёрна не положены.
        write(&dir, QUEUE_FILE, &json!({ "title": "Лист", "wave": null, "tracks": tracks }));
        assert_eq!(load_from(&dir).unwrap().wave, None);
        write(&dir, QUEUE_FILE, &json!({ "title": "Лист", "tracks": tracks }));
        assert_eq!(load_from(&dir).unwrap().wave, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_recent_searches_unique() {
        let list = remember_search(&[], " Queen ");
        assert_eq!(list, vec!["Queen"]);
        let list = remember_search(&list, "Земфира");
        let list = remember_search(&list, "queen");
        assert_eq!(list, vec!["queen", "Земфира"]);
        assert_eq!(remember_search(&list, "  "), list);
        let many = (0..30).fold(Vec::new(), |list, n| remember_search(&list, &n.to_string()));
        assert_eq!((many.len(), many[0].as_str()), (SEARCH_LIMIT, "29"));
    }

    #[test]
    fn survives_bad_place_and_missing_files() {
        let dir = folder("bad");
        assert!(load_from(&dir).is_none());
        write(&dir, QUEUE_FILE, &json!({ "title": "", "tracks": [track_value(&track("1", 100))] }));
        write(&dir, PLACE_FILE, &json!({ "current": 9, "seconds": 42.0 }));
        let saved = load_from(&dir).unwrap();
        assert_eq!((saved.current, saved.position), (0, Duration::ZERO));
        // Место дальше конца трека приводится к концу.
        write(&dir, PLACE_FILE, &json!({ "current": 0, "seconds": 500.0 }));
        assert_eq!(load_from(&dir).unwrap().position, Duration::from_secs(100));
        write(&dir, QUEUE_FILE, &json!({ "title": "", "tracks": [] }));
        assert!(load_from(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
