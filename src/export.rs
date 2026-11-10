//! «Скачать в файл»: трек сохраняется в «Загрузки» обычным файлом — MP3, FLAC или M4A — с
//! названием, исполнителями, альбомом и обложкой внутри, чтобы его понимали любые плееры,
//! проводник и телефон. Скачанное для офлайна (`offline`) лежит отдельно и нужно только Tune.
//!
//! Теги пишутся здесь же, без сторонних библиотек: ID3v2.3 у MP3, Vorbis-комментарии и
//! картинка у FLAC, атомы iTunes (`moov/udta/meta/ilst`) у M4A.

use std::path::{Path, PathBuf};

use crate::api::{self, Track};

/// Что записать в файл.
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Обложка JPEG или PNG.
    pub cover: Option<Vec<u8>>,
}

/// Формат по первым байтам: у версий без цензуры бывает m4a под видом mp3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Mp3,
    Flac,
    Mp4,
    /// Голый AAC (ADTS): тегов у него нет, файл сохраняется как есть.
    Aac,
}

impl Format {
    fn of(bytes: &[u8], codec: &str) -> Format {
        if bytes.get(4..8) == Some(b"ftyp") {
            Format::Mp4
        } else if bytes.starts_with(b"fLaC") {
            Format::Flac
        } else if bytes.starts_with(b"ID3") || (bytes.len() > 1 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0 && codec == "mp3") {
            Format::Mp3
        } else if codec.starts_with("aac") || codec.starts_with("he-aac") {
            if codec.ends_with("mp4") { Format::Mp4 } else { Format::Aac }
        } else {
            Format::Mp3
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Mp3 => "mp3",
            Format::Flac => "flac",
            Format::Mp4 => "m4a",
            Format::Aac => "aac",
        }
    }
}

/// Папка «Загрузки» пользователя, где бы она ни лежала.
pub fn downloads_dir() -> PathBuf {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG};
    // SAFETY: строку выделяет система, после копирования она освобождается.
    let known = unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, KNOWN_FOLDER_FLAG(0), None) }.ok().map(|path| {
        // SAFETY: путь — строка с нулём на конце от системы.
        let text = unsafe { path.to_string() }.ok();
        unsafe { CoTaskMemFree(Some(path.0 as _)) };
        text
    });
    known.flatten().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Downloads"))
}

/// Имя файла без запрещённых Windows знаков.
fn safe_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let clean: String = clean.trim().trim_end_matches(['.', ' ']).chars().take(150).collect();
    if clean.is_empty() { "Трек".to_string() } else { clean }
}

/// Свободное имя: «Трек.mp3», «Трек (2).mp3»…
fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// Качает трек, вписывает теги и кладёт в «Загрузки». Блокирует поток: зовётся в фоне.
pub fn save(track: &Track, quality: &str, token: &str) -> Result<PathBuf, String> {
    save_to(track, quality, token, &downloads_dir())
}

/// То же, что `save`, но в любую папку.
pub fn save_to(track: &Track, quality: &str, token: &str, dir: &Path) -> Result<PathBuf, String> {
    let info = api::file_info(&track.id, quality, token)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = dir.join(format!(".tune-{}.part", std::process::id()));
    let result = (|| {
        crate::player::save_file(&info, &part)?;
        let bytes = std::fs::read(&part).map_err(|e| e.to_string())?;
        let format = Format::of(&bytes, &info.codec);
        let cover = track.cover.as_deref().and_then(|uri| api::cover(uri, "1000x1000"));
        let tags = Tags { title: track.title.clone(), artist: track.artists.clone(), album: track.album_title.clone(), cover };
        // Теги не вписались — файл всё равно нужен, просто без них.
        let tagged = match format {
            Format::Mp3 => Some(mp3(&bytes, &tags)),
            Format::Flac => flac(&bytes, &tags),
            Format::Mp4 => mp4(&bytes, &tags),
            Format::Aac => None,
        };
        let stem = if track.artists.is_empty() { track.title.clone() } else { format!("{} — {}", track.artists, track.title) };
        let path = free_path(dir, &safe_name(&stem), format.extension());
        std::fs::write(&path, tagged.as_deref().unwrap_or(&bytes)).map_err(|e| format!("Не удалось сохранить файл: {e}"))?;
        Ok(path)
    })();
    let _ = std::fs::remove_file(&part);
    result
}

fn is_png(image: &[u8]) -> bool {
    image.starts_with(b"\x89PNG")
}

// --- MP3: ID3v2.3 -------------------------------------------------------------------------

/// Прежний тег ID3v2 убирается целиком: в нём бывают чужие названия и обложки.
fn mp3(bytes: &[u8], tags: &Tags) -> Vec<u8> {
    let mut audio = bytes;
    if audio.starts_with(b"ID3") && audio.len() >= 10 {
        let size = audio[6..10].iter().fold(0usize, |size, b| size << 7 | usize::from(b & 0x7F));
        let footer = if audio[5] & 0x10 != 0 { 10 } else { 0 };
        audio = audio.get(10 + size + footer..).unwrap_or(audio);
    }
    // Старый ID3v1 в конце («TAG» и 125 байт) тоже убирается: в нём бывает название с цензурой.
    if audio.len() >= 128 && audio[audio.len() - 128..].starts_with(b"TAG") {
        audio = &audio[..audio.len() - 128];
    }
    let mut frames = Vec::new();
    let mut frame = |id: &[u8; 4], body: Vec<u8>| {
        frames.extend_from_slice(id);
        frames.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frames.extend_from_slice(&[0, 0]);
        frames.extend_from_slice(&body);
    };
    // UTF-16 с меткой порядка байт: ID3v2.3 понимают все, включая проводник Windows.
    let text = |value: &str| {
        let mut body = vec![1, 0xFF, 0xFE];
        body.extend(value.encode_utf16().flat_map(u16::to_le_bytes));
        body
    };
    for (id, value) in [(b"TIT2", &tags.title), (b"TPE1", &tags.artist), (b"TALB", &tags.album)] {
        if !value.is_empty() {
            frame(id, text(value));
        }
    }
    if let Some(cover) = &tags.cover {
        let mut body = vec![0];
        body.extend_from_slice(if is_png(cover) { b"image/png\0" } else { b"image/jpeg\0" });
        body.extend_from_slice(&[3, 0]);
        body.extend_from_slice(cover);
        frame(b"APIC", body);
    }
    let size = frames.len();
    let mut out = Vec::with_capacity(10 + size + audio.len());
    out.extend_from_slice(b"ID3\x03\x00\x00");
    out.extend((0..4).rev().map(|i| ((size >> (7 * i)) & 0x7F) as u8));
    out.extend_from_slice(&frames);
    out.extend_from_slice(audio);
    out
}

// --- FLAC: Vorbis-комментарии и картинка --------------------------------------------------

fn flac(bytes: &[u8], tags: &Tags) -> Option<Vec<u8>> {
    let mut blocks: Vec<(u8, &[u8])> = Vec::new();
    let mut at = 4;
    loop {
        let header = bytes.get(at..at + 4)?;
        let (last, kind) = (header[0] & 0x80 != 0, header[0] & 0x7F);
        let len = (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        let body = bytes.get(at + 4..at + 4 + len)?;
        // Прежние комментарии и картинки заменяются своими; отступы не нужны.
        if !matches!(kind, 1 | 4 | 6) {
            blocks.push((kind, body));
        }
        at += 4 + len;
        if last {
            break;
        }
    }
    if blocks.first().is_none_or(|(kind, _)| *kind != 0) {
        return None;
    }
    let mut comment = Vec::new();
    let vendor = b"Tune";
    comment.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    comment.extend_from_slice(vendor);
    let fields: Vec<String> = [("TITLE", &tags.title), ("ARTIST", &tags.artist), ("ALBUM", &tags.album)].into_iter().filter(|(_, value)| !value.is_empty()).map(|(key, value)| format!("{key}={value}")).collect();
    comment.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for field in &fields {
        comment.extend_from_slice(&(field.len() as u32).to_le_bytes());
        comment.extend_from_slice(field.as_bytes());
    }
    let mut picture = Vec::new();
    if let Some(cover) = &tags.cover {
        let mime: &[u8] = if is_png(cover) { b"image/png" } else { b"image/jpeg" };
        picture.extend_from_slice(&3u32.to_be_bytes());
        picture.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        picture.extend_from_slice(mime);
        // Описание пустое; ширина, высота, глубина и палитра — 0: плееры читают их из картинки.
        picture.extend_from_slice(&[0u8; 20]);
        picture.extend_from_slice(&(cover.len() as u32).to_be_bytes());
        picture.extend_from_slice(cover);
    }
    // Порядок: STREAMINFO первым, затем комментарии и картинка, затем остальное.
    let mut ordered: Vec<(u8, &[u8])> = vec![blocks[0], (4, &comment)];
    if !picture.is_empty() && picture.len() < 1 << 24 {
        ordered.push((6, &picture));
    }
    ordered.extend(blocks[1..].iter().copied());
    let mut out = b"fLaC".to_vec();
    for (index, (kind, body)) in ordered.iter().enumerate() {
        let last = if index + 1 == ordered.len() { 0x80 } else { 0 };
        out.push(last | kind);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(body);
    }
    out.extend_from_slice(&bytes[at..]);
    Some(out)
}

// --- M4A: атомы iTunes --------------------------------------------------------------------

/// Атом: размер, имя, содержимое.
fn atom(name: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(body);
    out
}

/// Дочерние атомы: (имя, начало заголовка, начало тела, конец).
fn children(bytes: &[u8]) -> Option<Vec<([u8; 4], usize, usize, usize)>> {
    let mut list = Vec::new();
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().ok()?) as usize;
        let name: [u8; 4] = bytes[at + 4..at + 8].try_into().ok()?;
        let (body, end) = match size {
            0 => (at + 8, bytes.len()),
            1 => (at + 16, at + u64::from_be_bytes(bytes.get(at + 8..at + 16)?.try_into().ok()?) as usize),
            size => (at + 8, at + size),
        };
        if end > bytes.len() || end < body {
            return None;
        }
        list.push((name, at, body, end));
        at = end;
    }
    Some(list)
}

/// Сдвигает смещения кусков в `stco` и `co64`: данные `mdat` после `moov` уезжают на `delta`.
fn shift_offsets(moov: &mut [u8], delta: i64) -> Option<()> {
    for (name, _, body, end) in children(moov)? {
        match &name {
            b"trak" | b"mdia" | b"minf" | b"stbl" => shift_offsets(&mut moov[body..end], delta)?,
            b"stco" | b"co64" => {
                let wide = &name == b"co64";
                let count = u32::from_be_bytes(moov.get(body + 4..body + 8)?.try_into().ok()?) as usize;
                let step = if wide { 8 } else { 4 };
                for index in 0..count {
                    let at = body + 8 + index * step;
                    let slot = moov.get_mut(at..at + step)?;
                    if wide {
                        let value = u64::from_be_bytes(slot.try_into().ok()?) as i64 + delta;
                        slot.copy_from_slice(&(value as u64).to_be_bytes());
                    } else {
                        let value = i64::from(u32::from_be_bytes(slot.try_into().ok()?)) + delta;
                        slot.copy_from_slice(&u32::try_from(value).ok()?.to_be_bytes());
                    }
                }
            }
            _ => {}
        }
    }
    Some(())
}

fn mp4(bytes: &[u8], tags: &Tags) -> Option<Vec<u8>> {
    let top = children(bytes)?;
    let &(_, moov_start, moov_body, moov_end) = top.iter().find(|(name, ..)| name == b"moov")?;
    let data = |kind: u32, payload: &[u8]| {
        let mut body = kind.to_be_bytes().to_vec();
        body.extend_from_slice(&[0, 0, 0, 0]);
        body.extend_from_slice(payload);
        atom(b"data", &body)
    };
    let mut items = Vec::new();
    for (name, value) in [(b"\xA9nam", &tags.title), (b"\xA9ART", &tags.artist), (b"\xA9alb", &tags.album)] {
        if !value.is_empty() {
            items.extend(atom(name, &data(1, value.as_bytes())));
        }
    }
    if let Some(cover) = &tags.cover {
        items.extend(atom(b"covr", &data(if is_png(cover) { 14 } else { 13 }, cover)));
    }
    let mut handler = vec![0u8; 8];
    handler.extend_from_slice(b"mdirappl");
    handler.extend_from_slice(&[0u8; 9]);
    let mut meta = vec![0u8; 4];
    meta.extend(atom(b"hdlr", &handler));
    meta.extend(atom(b"ilst", &items));
    let udta = atom(b"udta", &atom(b"meta", &meta));
    // Прежний udta уходит, остальное содержимое moov остаётся как было.
    let inner = &bytes[moov_body..moov_end];
    let mut body = Vec::with_capacity(inner.len() + udta.len());
    for (name, start, _, end) in children(inner)? {
        if &name != b"udta" {
            body.extend_from_slice(&inner[start..end]);
        }
    }
    body.extend(udta);
    let mut moov = atom(b"moov", &body);
    let delta = moov.len() as i64 - (moov_end - moov_start) as i64;
    // Данные после moov сдвигаются: смещения в таблицах кусков правятся на ту же величину.
    if top.iter().any(|(name, start, ..)| name == b"mdat" && *start > moov_start) {
        shift_offsets(&mut moov[8..], delta)?;
    }
    let mut out = Vec::with_capacity(bytes.len() + delta.max(0) as usize);
    out.extend_from_slice(&bytes[..moov_start]);
    out.extend(moov);
    out.extend_from_slice(&bytes[moov_end..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags() -> Tags {
        Tags { title: "Трек".into(), artist: "Исполнитель".into(), album: "Альбом".into(), cover: Some(b"\xFF\xD8\xFFjpeg".to_vec()) }
    }

    #[test]
    fn mp3_tag_replaces_old_one() {
        let old = [b"ID3\x03\x00\x00\x00\x00\x00\x02xx".as_slice(), b"\xFF\xFBaudio"].concat();
        let out = mp3(&old, &tags());
        assert!(out.starts_with(b"ID3\x03"));
        assert!(out.ends_with(b"\xFF\xFBaudio"));
        let size = out[6..10].iter().fold(0usize, |size, b| size << 7 | usize::from(*b));
        assert_eq!(&out[10 + size..], b"\xFF\xFBaudio");
        assert_eq!(Format::of(&out, "mp3"), Format::Mp3);
    }

    #[test]
    fn mp3_drops_old_id3v1() {
        let mut old = b"\xFF\xFBaudio".to_vec();
        old.extend_from_slice(b"TAG");
        old.extend_from_slice(&[b'x'; 125]);
        assert!(mp3(&old, &tags()).ends_with(b"\xFF\xFBaudio"));
    }

    #[test]
    fn flac_gets_comment_and_picture() {
        let mut file = b"fLaC".to_vec();
        file.extend_from_slice(&[0x00, 0, 0, 34]);
        file.extend_from_slice(&[7u8; 34]);
        file.extend_from_slice(&[0x84, 0, 0, 4, 1, 2, 3, 4]);
        file.extend_from_slice(b"frames");
        let out = flac(&file, &tags()).unwrap();
        assert!(out.ends_with(b"frames"));
        assert_eq!(out[4] & 0x7F, 0);
        assert_eq!(out[8 + 34] & 0x7F, 4);
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("TITLE=Трек") && text.contains("ALBUM=Альбом"));
    }

    #[test]
    fn mp4_offsets_follow_bigger_moov() {
        // ftyp, moov с одним stco на смещение mdat, затем mdat.
        let ftyp = atom(b"ftyp", b"M4A \0\0\0\0");
        let build = |offset: u32| {
            let mut stco = vec![0u8; 4];
            stco.extend_from_slice(&1u32.to_be_bytes());
            stco.extend_from_slice(&offset.to_be_bytes());
            atom(b"moov", &atom(b"trak", &atom(b"mdia", &atom(b"minf", &atom(b"stbl", &atom(b"stco", &stco))))))
        };
        let moov_len = build(0).len();
        let offset = (ftyp.len() + moov_len + 8) as u32;
        let file = [ftyp.clone(), build(offset), atom(b"mdat", b"audio!")].concat();
        let out = mp4(&file, &tags()).unwrap();
        let top = children(&out).unwrap();
        let mdat = top.iter().find(|(name, ..)| name == b"mdat").unwrap();
        assert_eq!(&out[mdat.2..mdat.3], b"audio!");
        // Смещение в stco указывает ровно на начало данных mdat.
        let at = out.windows(4).position(|w| w == b"stco").unwrap() + 4 + 8;
        assert_eq!(u32::from_be_bytes(out[at..at + 4].try_into().unwrap()) as usize, mdat.2);
        assert!(out.windows(4).any(|w| w == b"ilst"));
    }
}
