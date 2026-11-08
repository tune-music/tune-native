//! Синхронный текст песен не только от Яндекса, как у Tune Client: из LRCLIB (открытая база)
//! и NetEase Cloud Music (много синхронных текстов, в том числе русских).
//!
//! Поиск отдаёт и чужие треки с похожим названием, поэтому берётся только совпадение по
//! названию, исполнителю и длительности. Синхронный текст лучше простого: простой берём, только
//! если синхронного не нашлось нигде.

use std::time::Duration;

use serde_json::Value;

use crate::api::{self, Lyrics, Track};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";

/// Откуда брать текст, как во вкладке «Плеер» у Tune Client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sources {
    /// Искать в LRCLIB и NetEase, когда у Яндекса нет синхронного текста.
    pub enabled: bool,
    /// Искать и там, где у Яндекса есть свой синхронный текст.
    pub always: bool,
    pub lrclib: bool,
    pub netease: bool,
    /// NetEase раньше LRCLIB.
    pub netease_first: bool,
}

impl Default for Sources {
    fn default() -> Self {
        Self { enabled: true, always: false, lrclib: true, netease: true, netease_first: false }
    }
}

fn get(url: &str, headers: &[(&str, &str)]) -> Result<Option<Value>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).http_status_as_error(false).build().into();
    let mut request = agent.get(url).header("User-Agent", UA);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let mut response = request.call().map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let text = response.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
    Ok(serde_json::from_str(&text).ok())
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Для сравнения названий: без регистра, «ё», скобок с пометками и знаков.
pub fn norm(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.to_lowercase().replace('ё', "е").chars() {
        match c {
            '(' | '[' | '（' => depth += 1,
            ')' | ']' | '）' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

/// Строки с авторами, что NetEase и LRCLIB ставят в начало текста: «作词 : …», «Lyrics by …».
pub fn is_credit(line: &str) -> bool {
    let t = line.trim();
    let head: String = t.chars().take_while(|&c| c != ':' && c != '：').collect();
    if head.len() == t.len() || head.chars().count() > 24 {
        return false;
    }
    let h = head.trim().to_lowercase();
    h.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        || [
            "lyrics", "lyricist", "composer", "composed", "written", "producer", "produced", "arranger", "arranged", "mixing", "mixed",
            "mastering", "текст", "слова", "музыка", "автор", "аранжировка", "сведение", "мастеринг",
        ]
        .iter()
        .any(|w| h.starts_with(w))
}

/// Тот же ли это трек: название, хоть один исполнитель и длительность в пределах 6 секунд.
fn same_track(track: &Track, title: &str, artist: &str, seconds: u64) -> bool {
    if title.is_empty() || norm(title) != norm(&track.title) {
        return false;
    }
    let theirs = norm(artist);
    let artist_ok = theirs.is_empty()
        || track.artist_list.iter().map(|(name, _)| norm(name)).chain([norm(&track.artists)]).any(|mine| !mine.is_empty() && (theirs.contains(&mine) || mine.contains(&theirs)));
    let duration = track.duration.as_secs();
    artist_ok && (duration == 0 || seconds == 0 || seconds.abs_diff(duration) <= 6)
}

fn first_artist(track: &Track) -> String {
    track.artist_list.first().map_or_else(|| track.artists.clone(), |(name, _)| name.clone())
}

/// Текст из LRCLIB: точное совпадение, а нет его — поиск.
fn lrclib(track: &Track) -> Result<Option<Lyrics>, String> {
    let headers = [("Lrclib-Client", "Tune Native")];
    let artist = first_artist(track);
    let mut query = format!("track_name={}&artist_name={}", enc(&track.title), enc(&artist));
    if !track.album_title.is_empty() {
        query += &format!("&album_name={}", enc(&track.album_title));
    }
    if track.duration.as_secs() > 0 {
        query += &format!("&duration={}", track.duration.as_secs());
    }
    let exact = get(&format!("https://lrclib.net/api/get?{query}"), &headers)?;
    let pick = |v: &Value| -> Option<Lyrics> {
        let text = v["syncedLyrics"].as_str().filter(|t| !t.trim().is_empty()).or(v["plainLyrics"].as_str().filter(|t| !t.trim().is_empty()))?;
        Some(external(text, "LRCLIB"))
    };
    if let Some(found) = exact.as_ref().filter(|v| v["syncedLyrics"].as_str().is_some_and(|t| !t.trim().is_empty())).and_then(pick) {
        return Ok(Some(found));
    }
    let list = get(&format!("https://lrclib.net/api/search?track_name={}&artist_name={}", enc(&track.title), enc(&artist)), &headers)?;
    let same: Vec<&Value> = list
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| same_track(track, v["trackName"].as_str().unwrap_or_default(), v["artistName"].as_str().unwrap_or_default(), v["duration"].as_f64().unwrap_or(0.0).round() as u64))
        .collect();
    let synced = same.iter().find(|v| v["syncedLyrics"].as_str().is_some_and(|t| !t.trim().is_empty()));
    Ok(synced.copied().or(exact.as_ref()).or(same.first().copied()).and_then(pick))
}

/// Синхронный текст из NetEase, если там именно этот трек.
fn netease(track: &Track) -> Result<Option<Lyrics>, String> {
    let headers = [("Referer", "https://music.163.com/")];
    let query = format!("{} {}", first_artist(track), track.title);
    let Some(found) = get(&format!("https://music.163.com/api/search/get?s={}&type=1&limit=10", enc(query.trim())), &headers)? else { return Ok(None) };
    let songs = found["result"]["songs"].as_array().cloned().unwrap_or_default();
    for song in songs.iter().filter(|song| {
        let artist = song["artists"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect::<Vec<_>>().join(" ");
        // «Miyagi & Andy Panda» у NetEase — один исполнитель, у Яндекса — два.
        !artist.is_empty() && same_track(track, song["name"].as_str().unwrap_or_default(), &artist, song["duration"].as_u64().unwrap_or(0) / 1000)
    }).take(3)
    {
        let Some(id) = song["id"].as_u64() else { continue };
        let Some(lyric) = get(&format!("https://music.163.com/api/song/lyric?id={id}&lv=1"), &headers)? else { continue };
        let lrc = lyric["lrc"]["lyric"].as_str().unwrap_or_default();
        // Хотя бы несколько строк со временем — иначе это пустышка или «纯音乐».
        if lrc.lines().filter(|l| l.starts_with('[') && l.len() > 11).count() >= 4 {
            return Ok(Some(external(lrc, "NetEase")));
        }
    }
    Ok(None)
}

/// Текст из чужой базы: без строк с авторами; отчёта о просмотре Яндексу у него нет.
fn external(text: &str, source: &'static str) -> Lyrics {
    let lines = api::parse_lrc(text).into_iter().filter(|(_, words)| !is_credit(words)).collect();
    Lyrics { lines, authors: Vec::new(), view: None, source }
}

/// Текст трека: Яндекс, LRCLIB и NetEase — в порядке из настроек.
pub fn find(track: &Track, sources: Sources, token: &str) -> Result<Option<Lyrics>, String> {
    let mut yandex: Option<Option<Lyrics>> = None;
    if track.lyrics && (!sources.enabled || !sources.always) {
        let found = api::lyrics(&track.id, token)?;
        if !sources.enabled || found.as_ref().is_some_and(Lyrics::synced) {
            return Ok(found);
        }
        yandex = Some(found);
    }
    if !sources.enabled {
        return Ok(None);
    }
    let order: [(bool, fn(&Track) -> Result<Option<Lyrics>, String>); 2] = if sources.netease_first { [(sources.netease, netease), (sources.lrclib, lrclib)] } else { [(sources.lrclib, lrclib), (sources.netease, netease)] };
    let mut fallback = None;
    for (on, search) in order {
        if !on {
            continue;
        }
        // Чужая база не ответила — не беда, есть другие.
        match search(track) {
            Ok(Some(found)) if found.synced() => return Ok(Some(found)),
            Ok(found) => fallback = fallback.or(found),
            Err(e) => api::log(&format!("текст песни: {e}")),
        }
    }
    // Синхронного нет нигде — текст Яндекса, хоть и простой, лучше простого из LRCLIB.
    if track.lyrics {
        let own = match yandex {
            Some(found) => found,
            None => api::lyrics(&track.id, token)?,
        };
        if own.is_some() {
            return Ok(own);
        }
    }
    Ok(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_titles() {
        assert_eq!(norm("Мотылёк (Remastered)"), "мотылек");
        assert_eq!(norm("I Got Love!"), "igotlove");
    }

    #[test]
    fn detects_credits() {
        assert!(is_credit(" 作词 : Макс Корж"));
        assert!(is_credit("Lyrics by: Someone"));
        assert!(is_credit("Текст: Автор"));
        assert!(!is_credit("Ну, что ты братишка притих... Обида залита в груди"));
    }

    #[test]
    fn same_track_by_title_artist_and_length() {
        let track = Track { title: "Мотылёк".into(), artists: "Miyagi, Andy Panda".into(), artist_list: vec![("Miyagi".into(), "1".into()), ("Andy Panda".into(), "2".into())], duration: Duration::from_secs(200), ..Track::default() };
        assert!(same_track(&track, "Мотылек (Remastered)", "Miyagi & Andy Panda", 203));
        assert!(!same_track(&track, "Мотылёк", "Кто-то ещё", 200));
        assert!(!same_track(&track, "Мотылёк", "Miyagi", 230));
    }

    #[test]
    fn external_drops_credits() {
        let lyrics = external("[00:01.00]作词 : Кто-то\n[00:05.00]Первая строка\n[00:09.00]Вторая", "NetEase");
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.source, "NetEase");
    }
}
