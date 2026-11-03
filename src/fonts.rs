//! Шрифты Яндекса. В приложение они не входят: берутся из клиента, который уже лежит на этом
//! компьютере у Tune, и один раз преобразуются для программы. Нет клиента — системный шрифт.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use serde_json::Value;

use crate::api;

const FILES: [&str; 4] = ["YSText-Regular", "YSText-Medium", "YSText-Bold", "YSMusic-HeadlineBold"];

pub fn asar() -> Option<PathBuf> {
    fs::read_dir(crate::client::root()?).ok()?.flatten().map(|dir| dir.path().join("app.asar")).find(|path| path.exists())
}

/// Читает файл из папки архива asar, не распаковывая остальное.
pub fn read_from_asar(path: &Path, folder: &[&str], name: &str) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut head = [0u8; 16];
    file.read_exact(&mut head).ok()?;
    let pickle = u32::from_le_bytes(head[4..8].try_into().ok()?) as u64;
    let size = u32::from_le_bytes(head[12..16].try_into().ok()?) as usize;
    let mut header = vec![0u8; size];
    file.read_exact(&mut header).ok()?;
    let header: Value = serde_json::from_slice(&header).ok()?;
    let entry = folder.iter().fold(&header, |node, part| &node["files"][part])["files"].get(name)?;
    let offset: u64 = entry["offset"].as_str()?.parse().ok()?;
    file.seek(SeekFrom::Start(8 + pickle + offset)).ok()?;
    let mut data = vec![0u8; entry["size"].as_u64()? as usize];
    file.read_exact(&mut data).ok()?;
    Some(data)
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// WOFF — это обычный шрифт, таблицы которого сжаты по отдельности.
fn woff_to_sfnt(woff: &[u8]) -> Option<Vec<u8>> {
    if woff.get(..4)? != b"wOFF" {
        return None;
    }
    let tables = u16_at(woff, 12)? as usize;
    let mut bodies = Vec::with_capacity(tables);
    for i in 0..tables {
        let at = 44 + i * 20;
        let (offset, compressed, original) = (u32_at(woff, at + 4)? as usize, u32_at(woff, at + 8)? as usize, u32_at(woff, at + 12)? as usize);
        let data = woff.get(offset..offset + compressed)?;
        let body = if compressed == original {
            data.to_vec()
        } else {
            let mut out = Vec::with_capacity(original);
            ZlibDecoder::new(data).read_to_end(&mut out).ok()?;
            out
        };
        if body.len() != original {
            return None;
        }
        bodies.push((woff.get(at..at + 4)?, u32_at(woff, at + 16)?, body));
    }
    let power = tables.checked_ilog2()?;
    let range = (1u16 << power) * 16;
    let mut out = woff.get(4..8)?.to_vec();
    out.extend((tables as u16).to_be_bytes());
    out.extend(range.to_be_bytes());
    out.extend((power as u16).to_be_bytes());
    out.extend((tables as u16 * 16 - range).to_be_bytes());
    let mut offset = 12 + tables * 16;
    for (tag, checksum, body) in &bodies {
        out.extend(*tag);
        out.extend(checksum.to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((body.len() as u32).to_be_bytes());
        offset += body.len().next_multiple_of(4);
    }
    for (_, _, body) in &bodies {
        out.extend(body);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Some(out)
}

/// Все четыре начертания или ничего: смесь шрифтов выглядела бы хуже системного.
pub fn load() -> Vec<Vec<u8>> {
    let dir = api::data_dir().join("fonts");
    let cached: Option<Vec<Vec<u8>>> = FILES.iter().map(|name| fs::read(dir.join(format!("{name}.ttf"))).ok()).collect();
    if let Some(fonts) = cached {
        return fonts;
    }
    let Some(path) = asar() else { return Vec::new() };
    let fonts: Option<Vec<Vec<u8>>> = FILES.iter().map(|name| woff_to_sfnt(&read_from_asar(&path, &["app", "fonts"], &format!("{name}.woff"))?)).collect();
    let Some(fonts) = fonts else { return Vec::new() };
    if fs::create_dir_all(&dir).is_ok() {
        for (name, font) in FILES.iter().zip(&fonts) {
            let _ = fs::write(dir.join(format!("{name}.ttf")), font);
        }
    }
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn unpacks_compressed_and_plain_tables() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&[7u8; 100]).unwrap();
        let packed = encoder.finish().unwrap();
        let plain = [1u8, 2, 3, 4, 5];
        let mut woff = b"wOFF\x00\x01\x00\x00".to_vec();
        woff.extend([0u8; 4]);
        woff.extend(2u16.to_be_bytes());
        woff.resize(44, 0);
        let first = 44 + 40;
        for (tag, offset, comp, orig) in [(b"aaaa", first, packed.len(), 100usize), (b"bbbb", first + packed.len(), plain.len(), plain.len())] {
            woff.extend(tag);
            woff.extend((offset as u32).to_be_bytes());
            woff.extend((comp as u32).to_be_bytes());
            woff.extend((orig as u32).to_be_bytes());
            woff.extend(9u32.to_be_bytes());
        }
        woff.extend(&packed);
        woff.extend(plain);
        let sfnt = woff_to_sfnt(&woff).unwrap();
        assert_eq!(&sfnt[..4], &[0, 1, 0, 0]);
        assert_eq!(u16_at(&sfnt, 4), Some(2));
        assert_eq!(&sfnt[12..16], b"aaaa");
        assert_eq!(u32_at(&sfnt, 20), Some(44));
        assert_eq!(u32_at(&sfnt, 24), Some(100));
        assert_eq!(&sfnt[44..48], &[7, 7, 7, 7]);
        assert_eq!(u32_at(&sfnt, 36), Some(144));
        assert_eq!(&sfnt[144..149], &plain);
        assert!(woff_to_sfnt(b"nope").is_none());
    }
}
