//! Одна копия программы. Вторая, запущенная ссылкой или ярлыком, отдаёт ссылку первой
//! через именованный канал и закрывается, а первая показывает окно и открывает ссылку.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use iced::futures::stream::{self, Stream, StreamExt};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, PIPE_ACCESS_INBOUND};
use windows_sys::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};
use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

const NAME: &str = r"\\.\pipe\tune-native";
/// Ссылка длиннее этого — уже не ссылка.
const LIMIT: usize = 8 * 1024;

/// Отдаёт ссылку уже запущенной копии; пустая строка — просто показать окно.
/// false — другой копии нет, запускаться надо самой.
pub fn forward(link: &str) -> bool {
    let Ok(mut pipe) = OpenOptions::new().write(true).open(NAME) else { return false };
    // Окно другой программы Windows выводит вперёд, только если ей это разрешили.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    pipe.write_all(link.as_bytes()).is_ok()
}

type Channel = (UnboundedSender<String>, Mutex<Option<UnboundedReceiver<String>>>);
static LINKS: OnceLock<Channel> = OnceLock::new();

/// Ссылки от других копий. Канал слушает свой поток, читать поток можно один раз.
pub fn links() -> impl Stream<Item = String> {
    let (tx, rx) = LINKS.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded();
        (tx, Mutex::new(Some(rx)))
    });
    match rx.lock().unwrap().take() {
        Some(rx) => {
            let tx = tx.clone();
            std::thread::spawn(move || listen(tx));
            rx.left_stream()
        }
        None => stream::pending().right_stream(),
    }
}

fn listen(tx: UnboundedSender<String>) {
    let name: Vec<u16> = NAME.encode_utf16().chain([0]).collect();
    loop {
        let pipe = unsafe { CreateNamedPipeW(name.as_ptr(), PIPE_ACCESS_INBOUND, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, PIPE_UNLIMITED_INSTANCES, 0, LIMIT as u32, 0, std::ptr::null()) };
        if pipe == INVALID_HANDLE_VALUE {
            return;
        }
        let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) != 0 || GetLastError() == ERROR_PIPE_CONNECTED };
        let mut text = Vec::new();
        let mut buffer = [0u8; 1024];
        while connected && text.len() < LIMIT {
            let mut read = 0u32;
            if unsafe { ReadFile(pipe, buffer.as_mut_ptr(), buffer.len() as u32, &mut read, std::ptr::null_mut()) } == 0 || read == 0 {
                break;
            }
            text.extend_from_slice(&buffer[..read as usize]);
        }
        unsafe {
            DisconnectNamedPipe(pipe);
            CloseHandle(pipe);
        }
        if connected && tx.unbounded_send(String::from_utf8_lossy(&text).into_owned()).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_copy_hands_link_to_first() {
        let mut links = Box::pin(links());
        // Канал поднимается в своём потоке: вторая копия ждёт, пока он появится.
        let sent = (0..50).any(|_| forward("yandexmusic://album/7") || { std::thread::sleep(std::time::Duration::from_millis(20)); false });
        assert!(sent);
        assert_eq!(wait(links.next()).as_deref(), Some("yandexmusic://album/7"));
    }

    /// Ждёт будущее в этом же потоке: поток спит, пока его не разбудят.
    fn wait<F: std::future::Future>(future: F) -> F::Output {
        struct Unpark(std::thread::Thread);
        impl std::task::Wake for Unpark {
            fn wake(self: std::sync::Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::sync::Arc::new(Unpark(std::thread::current())).into();
        let mut context = std::task::Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(100));
        }
    }
}
