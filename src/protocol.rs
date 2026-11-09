//! Ссылки из браузера и других программ. Протокол `music-application://` (так ссылается
//! на свои страницы оригинал) Tune берёт себе, если он ничей. `yandexmusic://` — протокол
//! официального клиента: кнопка «Открыть в приложении» на music.yandex.ru. Его Tune берёт,
//! только когда это включено в настройках, а прежний обработчик запоминает и возвращает при
//! выключении. Всё в разделе пользователя (HKCU), без прав администратора.
//!
//! Обычные https-ссылки на music.yandex.ru браузер открывает сам: перехватить их программа
//! не может. Такую ссылку можно вставить в поиск Tune.

use windows_sys::Win32::System::Registry::{RegDeleteTreeW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

const OWN: &str = "music-application";
const YANDEX: &str = "yandexmusic";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn classes(scheme: &str, sub: &str) -> String {
    if sub.is_empty() { format!("Software\\Classes\\{scheme}") } else { format!("Software\\Classes\\{scheme}\\{sub}") }
}

/// Строковое значение; `None` — нет ключа или значения.
fn get(key: &str, name: &str) -> Option<String> {
    let (key, name) = (wide(key), wide(name));
    let name_ptr = if name.len() > 1 { name.as_ptr() } else { std::ptr::null() };
    let mut size = 0u32;
    // SAFETY: строки оканчиваются нулём; первый вызов узнаёт размер, второй читает.
    unsafe {
        if RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name_ptr, RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) != 0 {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        if RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name_ptr, RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size) != 0 {
            return None;
        }
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end]))
    }
}

fn set(key: &str, name: &str, value: &str) -> Result<(), String> {
    let (key, name, value) = (wide(key), wide(name), wide(value));
    let name_ptr = if name.len() > 1 { name.as_ptr() } else { std::ptr::null() };
    // SAFETY: строки оканчиваются нулём, длина значения — в байтах вместе с нулём.
    let status = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name_ptr, REG_SZ, value.as_ptr().cast(), (value.len() * 2) as u32) };
    if status == 0 { Ok(()) } else { Err(format!("Не удалось записать ссылки в реестр ({status})")) }
}

/// Команда запуска Tune со ссылкой.
fn command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(format!("\"{}\" \"%1\"", exe.display()))
}

fn handler(scheme: &str) -> Option<String> {
    get(&classes(scheme, "shell\\open\\command"), "")
}

fn write(scheme: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    set(&classes(scheme, ""), "", "URL:Tune")?;
    set(&classes(scheme, ""), "URL Protocol", "")?;
    set(&classes(scheme, "DefaultIcon"), "", &format!("\"{}\",0", exe.display()))?;
    set(&classes(scheme, "shell\\open\\command"), "", &command()?)
}

/// Обработчик — это Tune (возможно, из другой папки).
fn is_tune(command: &str) -> bool {
    command.to_lowercase().contains("tune-native")
}

/// `music-application://` — Tune, если протокол ничей или уже наш. Вызывается при запуске.
pub fn register_own() {
    std::thread::spawn(|| {
        let ours = command().ok();
        match handler(OWN) {
            Some(current) if Some(&current) == ours.as_ref() => {}
            Some(current) if !is_tune(&current) => {}
            _ => {
                if let Err(e) = write(OWN) {
                    crate::api::log(&e);
                }
            }
        }
    });
}

fn saved_path() -> std::path::PathBuf {
    crate::api::data_dir().join("yandexmusic-handler.txt")
}

/// Берёт `yandexmusic://` себе или возвращает прежнему обработчику.
pub fn set_yandex(on: bool) -> Result<(), String> {
    let current = handler(YANDEX);
    if on {
        if let Some(other) = current.as_ref().filter(|current| !is_tune(current)) {
            std::fs::create_dir_all(crate::api::data_dir()).map_err(|e| e.to_string())?;
            std::fs::write(saved_path(), other).map_err(|e| e.to_string())?;
        }
        return write(YANDEX);
    }
    if !current.as_deref().is_some_and(is_tune) {
        return Ok(());
    }
    match std::fs::read_to_string(saved_path()) {
        Ok(previous) => {
            set(&classes(YANDEX, "shell\\open\\command"), "", &previous)?;
            // Значок и подпись были наши — у прежнего обработчика они берутся из его программы.
            let exe = previous.trim_start_matches('"').split('"').next().unwrap_or_default().to_string();
            set(&classes(YANDEX, "DefaultIcon"), "", &format!("\"{exe}\",0"))?;
            set(&classes(YANDEX, ""), "", "URL:yandexmusic")?;
            let _ = std::fs::remove_file(saved_path());
            Ok(())
        }
        Err(_) => {
            let key = wide(&classes(YANDEX, ""));
            // SAFETY: строка оканчивается нулём, удаляется только наш ключ протокола.
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
            Ok(())
        }
    }
}
