//! Клиент Яндекс Музыки, из которого берутся шрифты и значки. Если Tune на компьютере нет,
//! программа сама скачивает ту же закреплённую версию официального установщика, сверяет SHA-512
//! и достаёт из него только `resources/app.asar` — туда же, где его держит Tune, чтобы обе
//! программы пользовались одним файлом. Сам официальный клиент не устанавливается.

use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use sha2::{Digest, Sha512};

/// Та же версия и сумма, что у Tune: значение `sha512` из `stable/latest.yml`.
const VERSION: &str = "5.120.0";
const SHA512: &str = "9Z43BtBjUo1nnFwbgXmK1rMPa6aVZn1uF9S6qERifQFUZs7UuBd8ove+gZiYmgfp/tjDVV6YkPehHGB2XvDnGA==";
const BASE_URL: &str = "https://desktop.app.music.yandex.net/stable/";
const ASAR_ENTRY: &str = "resources/app.asar";
const ATTEMPTS: usize = 5;

/// Папка, где Tune держит распакованные клиенты по версиям.
pub fn root() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("dev.tune.music").join("client"))
}

/// Скачивает и распаковывает клиент. Оборванная загрузка продолжается с того же места.
pub fn install() -> Result<(), String> {
    let dir = root().ok_or("не найдена папка APPDATA")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("Yandex_Music_x64_{VERSION}.exe");
    let installer = dir.join(format!("{name}.part"));
    if sha512_file(&installer).ok().as_deref() != Some(SHA512) {
        download(&format!("{BASE_URL}{name}"), &installer)?;
        if sha512_file(&installer)? != SHA512 {
            let _ = fs::remove_file(&installer);
            return Err("скачанный установщик не совпадает с ожидаемым (SHA-512)".into());
        }
    }
    let target = dir.join(VERSION).join("app.asar");
    fs::create_dir_all(target.parent().ok_or("нет папки версии")?).map_err(|e| e.to_string())?;
    let part = target.with_extension("asar.part");
    extract_asar(&installer, &part)?;
    fs::rename(&part, &target).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&installer);
    Ok(())
}

fn download(url: &str, path: &Path) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..ATTEMPTS {
        let have = fs::metadata(path).map_or(0, |m| m.len());
        let mut request = ureq::get(url).header("User-Agent", "TuneNative");
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        let resumed = response.status() == 206;
        let length: u64 = response.headers().get("Content-Length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
        let (mut done, total) = if resumed { (have, have + length) } else { (0, length) };
        let mut file = if resumed { fs::OpenOptions::new().append(true).open(path) } else { File::create(path) }.map_err(|e| e.to_string())?;
        let mut body = response.into_body();
        let mut reader = body.as_reader();
        let mut buffer = vec![0u8; 256 * 1024];
        let copied: io::Result<()> = (|| loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                return Ok(());
            }
            file.write_all(&buffer[..n])?;
            done += n as u64;
        })();
        match copied {
            Ok(()) if total == 0 || done >= total => return Ok(()),
            Ok(()) => last = "соединение оборвалось".into(),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("не удалось скачать клиент Яндекс Музыки: {last}"))
}

fn sha512_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha512::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(hasher.finalize()))
}

/// Кусок большого файла, который читается как отдельный файл.
struct Window {
    inner: BufReader<File>,
    start: u64,
    len: u64,
    pos: u64,
}

impl Read for Window {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos) as usize;
        let cap = buf.len().min(left);
        let n = self.inner.read(&mut buf[..cap])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Window {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => self.len as i64 + d,
        };
        if pos < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "перемотка до начала файла"));
        }
        self.pos = pos as u64;
        self.inner.seek(SeekFrom::Start(self.start + self.pos))?;
        Ok(self.pos)
    }
}

/// Где в файле начинается архив 7z. Сигнатура может встретиться случайно, поэтому
/// проверяется, что следующий заголовок архива укладывается в файл.
fn archive_start(data: &[u8], offset: u64, size: u64) -> Option<u64> {
    const SIGNATURE: [u8; 6] = [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C];
    (0..data.len().saturating_sub(32)).find_map(|i| {
        if data[i..i + 6] != SIGNATURE {
            return None;
        }
        let next = u64::from_le_bytes(data[i + 12..i + 20].try_into().ok()?);
        let length = u64::from_le_bytes(data[i + 20..i + 28].try_into().ok()?);
        let at = offset + i as u64;
        (length > 0 && at + 32 + next + length <= size).then_some(at)
    })
}

/// Установщик electron-builder — заглушка NSIS, внутри которой без сжатия лежит 7z с приложением.
fn extract_asar(installer: &Path, out: &Path) -> Result<(), String> {
    let mut file = File::open(installer).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut buffer = vec![0u8; 4 * 1024 * 1024];
    let mut offset = 0u64;
    let start = loop {
        file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n < 32 {
            return Err("в установщике не найден архив приложения".into());
        }
        if let Some(start) = archive_start(&buffer[..n], offset, size) {
            break start;
        }
        offset += (n - 32) as u64;
    };
    let window = Window { inner: BufReader::new(file), start, len: size - start, pos: 0 };
    let mut reader = sevenz_rust2::ArchiveReader::new(window, sevenz_rust2::Password::empty()).map_err(|e| format!("архив приложения не читается: {e}"))?;
    let mut found = false;
    reader
        .for_each_entries(|entry, data| {
            if entry.name().replace('\\', "/") != ASAR_ENTRY {
                // В сплошном архиве данные идут потоком, и пропустить их можно только чтением.
                io::copy(data, &mut io::sink())?;
                return Ok(true);
            }
            io::copy(data, &mut File::create(out)?)?;
            found = true;
            Ok(false)
        })
        .map_err(|e| format!("ошибка распаковки: {e}"))?;
    if found { Ok(()) } else { Err(format!("в архиве нет {ASAR_ENTRY}")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_archive_only_where_header_fits() {
        let mut data = vec![0u8; 100];
        data[10..16].copy_from_slice(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]);
        data[22..30].copy_from_slice(&5u64.to_le_bytes());
        data[30..38].copy_from_slice(&10u64.to_le_bytes());
        assert_eq!(archive_start(&data, 1000, 1100), Some(1010));
        assert_eq!(archive_start(&data, 1000, 1040), None);
    }

    /// Настоящая загрузка, около 150 МБ: APPDATA=<пустая папка> cargo test downloads -- --ignored
    #[test]
    #[ignore]
    fn downloads_client_with_fonts_and_icons() {
        install().unwrap();
        let asar = crate::fonts::asar().unwrap();
        assert!(crate::fonts::read_from_asar(&asar, &["app", "fonts"], "YSText-Regular.woff").is_some());
        assert!(crate::fonts::read_from_asar(&asar, &["app", "icons"], "sprite.svg").is_some());
    }
}
