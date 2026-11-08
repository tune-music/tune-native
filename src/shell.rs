//! Как Windows узнаёт Tune: свой идентификатор приложения (AppUserModelID) и ярлык в меню
//! «Пуск» с ним. По ярлыку медиапанель Windows подписывает музыку «Tune» с нашим значком,
//! а не «Неизвестное приложение»; заодно Tune находится поиском в «Пуске».

use std::path::{Path, PathBuf};

use windows::core::{Interface, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{IShellLinkW, SHStrDupW, SetCurrentProcessExplicitAppUserModelID, ShellLink};

use crate::api;

pub const APP_ID: &str = "Tune.Native";
const SHORTCUT: &str = "Tune.lnk";
/// System.AppUserModel.ID.
const PKEY_APP_ID: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

/// Идентификатор ставится до первого окна: по нему окно, панель задач и медиапанель
/// узнают Tune.
pub fn identify() {
    // SAFETY: строка живёт до конца вызова.
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_ID)) } {
        api::log(&format!("идентификатор приложения: {e}"));
    }
}

/// Ярлык в «Пуске» с тем же идентификатором; обновляется, если программа переехала.
pub fn register() {
    std::thread::spawn(|| {
        let (Ok(exe), Some(dir)) = (std::env::current_exe(), programs_dir()) else { return };
        let link = dir.join(SHORTCUT);
        // Ярлык уже на эту программу — трогать нечего.
        let stamp = api::data_dir().join("shortcut.txt");
        if link.exists() && std::fs::read_to_string(&stamp).is_ok_and(|saved| saved == exe.to_string_lossy()) {
            return;
        }
        match write_shortcut(&exe, &link) {
            Ok(()) => {
                let _ = std::fs::write(&stamp, exe.to_string_lossy().as_bytes());
            }
            Err(e) => api::log(&format!("ярлык в «Пуске»: {e}")),
        }
    });
}

fn programs_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Microsoft\\Windows\\Start Menu\\Programs"))
}

fn write_shortcut(exe: &Path, link: &Path) -> windows::core::Result<()> {
    // SAFETY: COM в своём потоке; строки и PROPVARIANT живут до конца вызовов, значение
    // освобождается PropVariantClear.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        shell.SetPath(&HSTRING::from(exe.as_os_str()))?;
        if let Some(dir) = exe.parent() {
            shell.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        shell.SetDescription(&HSTRING::from("Tune — Яндекс Музыка"))?;
        shell.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        let store: IPropertyStore = shell.cast()?;
        let id = HSTRING::from(APP_ID);
        let text: PWSTR = SHStrDupW(PCWSTR(id.as_ptr()))?;
        let mut value = PROPVARIANT { Anonymous: PROPVARIANT_0 { Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 { vt: VT_LPWSTR, Anonymous: PROPVARIANT_0_0_0 { pwszVal: text }, ..Default::default() }) } };
        let saved = store.SetValue(&PKEY_APP_ID, &value).and_then(|()| store.Commit());
        let _ = PropVariantClear(&mut value);
        saved?;
        let file: IPersistFile = shell.cast()?;
        file.Save(&HSTRING::from(link.as_os_str()), true)
    }
}
