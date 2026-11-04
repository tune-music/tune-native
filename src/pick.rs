//! Системное окно выбора файла Windows.

use std::path::PathBuf;

use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Картинка JPEG или PNG. Окно привязано к окну приложения и держит его, пока открыто,
/// поэтому зовётся в фоне. `None` — окно закрыли без выбора.
pub fn image(owner: Option<isize>, title: &str) -> Option<PathBuf> {
    let mut file = vec![0u16; 2048];
    // Фильтр — пары «подпись, маска», разделённые нулями, в конце двойной ноль.
    let filter = wide("Картинки JPEG и PNG\0*.jpg;*.jpeg;*.png\0");
    let title = wide(title);
    // SAFETY: структура из простых полей, нули в ней — «не задано».
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner.unwrap_or(0) as _;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    // SAFETY: все строки живут до конца вызова, буфер под путь передан с его длиной.
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = file.iter().position(|&c| c == 0)?;
    Some(PathBuf::from(String::from_utf16_lossy(&file[..length])))
}

/// Аудиофайлы, можно несколько сразу. Пустой список — окно закрыли без выбора.
pub fn audio(owner: Option<isize>, title: &str) -> Vec<PathBuf> {
    // Под много файлов нужен буфер побольше: в нём папка и все имена через нули.
    let mut file = vec![0u16; 65536];
    let filter = wide("Музыка\0*.mp3;*.m4a;*.aac;*.flac;*.ogg;*.opus;*.wav\0Все файлы\0*.*\0");
    let title = wide(title);
    // SAFETY: структура из простых полей, нули в ней — «не задано».
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner.unwrap_or(0) as _;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = file.as_mut_ptr();
    dialog.nMaxFile = file.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_ALLOWMULTISELECT;
    // SAFETY: все строки живут до конца вызова, буфер под пути передан с его длиной.
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return Vec::new();
    }
    chosen(&file)
}

/// Один файл приходит полным путём, несколько — папкой и именами; конец — двойной ноль.
fn chosen(buffer: &[u16]) -> Vec<PathBuf> {
    let parts: Vec<String> = buffer.split(|&c| c == 0).take_while(|part| !part.is_empty()).map(String::from_utf16_lossy).collect();
    match parts.as_slice() {
        [] => Vec::new(),
        [single] => vec![PathBuf::from(single)],
        [folder, names @ ..] => names.iter().map(|name| PathBuf::from(folder).join(name)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_files_join_folder() {
        let buffer: Vec<u16> = "C:\\Music\0a.mp3\0b.mp3\0\0".encode_utf16().collect();
        assert_eq!(chosen(&buffer), vec![PathBuf::from("C:\\Music\\a.mp3"), PathBuf::from("C:\\Music\\b.mp3")]);
        let single: Vec<u16> = "C:\\Music\\a.mp3\0\0".encode_utf16().collect();
        assert_eq!(chosen(&single), vec![PathBuf::from("C:\\Music\\a.mp3")]);
    }
}
