//! Системное окно выбора файла Windows.

use std::path::PathBuf;

use windows_sys::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};

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
