//! Кладёт видеоплеер (`vendor/mpv/libmpv-2.dll`) рядом с собранной программой.
//! Библиотека большая и в репозиторий не входит: сборка shinchiro/mpv-winbuild-cmake
//! (`mpv-dev-x86_64-*.7z`). Без неё Tune собирается и работает, только клипы
//! открываются в браузере.
//!
//! Ещё встраивает в программу значок и сведения о ней: по ним Windows подписывает Tune в
//! диспетчере задач, микшере громкости и Проводнике. Файл ресурсов (.res) собирается здесь
//! же, без rc.exe, — компоновщик MSVC принимает его как есть.

use std::path::PathBuf;

fn main() {
    resources();
    let source = PathBuf::from("vendor/mpv/libmpv-2.dll");
    println!("cargo:rerun-if-changed={}", source.display());
    let Ok(out) = std::env::var("OUT_DIR") else { return };
    // OUT_DIR — target/<профиль>/build/<пакет>/out, программа лежит в target/<профиль>.
    let Some(dir) = PathBuf::from(out).ancestors().nth(3).map(PathBuf::from) else { return };
    let target = dir.join("libmpv-2.dll");
    let Ok(meta) = std::fs::metadata(&source) else { return };
    let fresh = std::fs::metadata(&target).is_ok_and(|have| have.len() == meta.len());
    if !fresh {
        let _ = std::fs::copy(&source, &target);
    }
}

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
/// Русский, Юникод.
const LANGUAGE: u16 = 0x0419;
const CODEPAGE: u16 = 1200;

fn resources() {
    println!("cargo:rerun-if-changed=assets/tune.ico");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let Ok(ico) = std::fs::read("assets/tune.ico") else { return };
    let Ok(out) = std::env::var("OUT_DIR") else { return };
    let mut res = Vec::new();
    // Файл .res начинается с пустой записи.
    entry(&mut res, 0, 0, 0, &[]);
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut group = vec![0, 0, 1, 0];
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for i in 0..count {
        let at = 6 + i * 16;
        let length = u32::from_le_bytes(ico[at + 8..at + 12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(ico[at + 12..at + 16].try_into().unwrap()) as usize;
        let id = i as u16 + 1;
        entry(&mut res, RT_ICON, id, LANGUAGE, &ico[offset..offset + length]);
        // Запись группы — как в .ico, только вместо смещения номер картинки.
        group.extend_from_slice(&ico[at..at + 12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut res, RT_GROUP_ICON, 1, LANGUAGE, &group);
    entry(&mut res, RT_VERSION, 1, LANGUAGE, &version_info());
    let path = PathBuf::from(out).join("tune.res");
    if std::fs::write(&path, res).is_ok() {
        println!("cargo:rustc-link-arg-bins={}", path.display());
    }
}

/// Запись ресурса с числовыми типом и номером.
fn entry(res: &mut Vec<u8>, kind: u16, id: u16, language: u16, data: &[u8]) {
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&32u32.to_le_bytes());
    for value in [0xFFFF, kind, 0xFFFF, id] {
        res.extend_from_slice(&value.to_le_bytes());
    }
    res.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    res.extend_from_slice(&0x1030u16.to_le_bytes()); // MOVEABLE | PURE | DISCARDABLE
    res.extend_from_slice(&language.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // Version
    res.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    res.extend_from_slice(data);
    pad(res);
}

fn pad(bytes: &mut Vec<u8>) {
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
}

fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// Узел VS_VERSIONINFO: длина, длина значения, тип, ключ, значение и вложенные узлы,
/// каждое с выравниванием на 4 байта.
fn node(key: &str, value: &[u8], text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut body = vec![0; 6];
    body.extend(wide(key));
    pad(&mut body);
    body.extend_from_slice(value);
    for child in children {
        pad(&mut body);
        body.extend_from_slice(child);
    }
    let value_length = if text { value.len() / 2 } else { value.len() };
    let length = body.len() as u16;
    body[0..2].copy_from_slice(&length.to_le_bytes());
    body[2..4].copy_from_slice(&(value_length as u16).to_le_bytes());
    body[4..6].copy_from_slice(&u16::from(text).to_le_bytes());
    body
}

fn version_info() -> Vec<u8> {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let parts: Vec<u32> = version.split(['.', '-']).filter_map(|part| part.parse().ok()).chain([0, 0, 0]).take(3).collect();
    let (high, low) = ((parts[0] << 16) | parts[1], parts[2] << 16);
    let fixed: Vec<u8> = [0xFEEF_04BD, 0x0001_0000, high, low, high, low, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0].iter().flat_map(|value: &u32| value.to_le_bytes()).collect();
    let strings = [
        ("CompanyName", "Tune"),
        ("FileDescription", "Tune"),
        ("FileVersion", version.as_str()),
        ("InternalName", "tune-native"),
        ("LegalCopyright", "Tune"),
        ("OriginalFilename", "tune-native.exe"),
        ("ProductName", "Tune"),
        ("ProductVersion", version.as_str()),
    ];
    let table: Vec<Vec<u8>> = strings.iter().map(|(key, value)| node(key, &wide(value), true, &[])).collect();
    let table = node(&format!("{LANGUAGE:04X}{CODEPAGE:04X}"), &[], true, &table);
    let string_info = node("StringFileInfo", &[], true, &[table]);
    let translation: Vec<u8> = [LANGUAGE, CODEPAGE].iter().flat_map(|value| value.to_le_bytes()).collect();
    let var_info = node("VarFileInfo", &[], true, &[node("Translation", &translation, false, &[])]);
    node("VS_VERSION_INFO", &fixed, false, &[string_info, var_info])
}
