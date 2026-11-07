//! Кладёт видеоплеер (`vendor/mpv/libmpv-2.dll`) рядом с собранной программой.
//! Библиотека большая и в репозиторий не входит: сборка shinchiro/mpv-winbuild-cmake
//! (`mpv-dev-x86_64-*.7z`). Без неё Tune собирается и работает, только клипы
//! открываются в браузере.

use std::path::PathBuf;

fn main() {
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
