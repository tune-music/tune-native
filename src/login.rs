//! Вход как в оригинальном клиенте: страница OAuth Яндекса возвращает токен на адрес
//! music-application://desktop/oauth. Окно входа живёт в отдельном процессе: когда он
//! закрывается, память WebView2 освобождается, а основное окно о нём не знает.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::{WebContext, WebViewBuilder};

use crate::api;

const AUTHORIZE: &str = "https://oauth.yandex.ru/authorize";
const CLIENT_ID: &str = "97fe03033fa34407ac9bcf91d5afed5b";
const REDIRECT: &str = "music-application://desktop/oauth";

fn encode(value: &str) -> String {
    value.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
        out
    })
}

fn authorize_url(state: &str) -> String {
    format!(
        "{AUTHORIZE}?response_type=token&display=popup&scope=music:content&scope=music:read&scope=music:write&client_id={CLIENT_ID}&redirect_uri={}&state={state}&origin=music_desktop&language=ru",
        encode(REDIRECT)
    )
}

/// Токен из адреса возврата. Чужой state — не наш ответ.
fn parse_token(url: &str, state: &str) -> Option<String> {
    let fragment = url.split_once('#')?.1;
    let param = |name: &str| fragment.split('&').find_map(|pair| pair.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v));
    if param("state")? != state {
        return None;
    }
    param("access_token").filter(|t| !t.is_empty()).map(str::to_string)
}

/// Показывает окно входа и завершает процесс: 0 — токен сохранён, 1 — вход не удался.
pub fn run() -> ! {
    let state = format!("{:016x}", RandomState::new().build_hasher().finish());
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Вход в Яндекс")
        .with_inner_size(LogicalSize::new(480.0, 720.0))
        .build(&event_loop)
        .expect("окно входа");
    let mut context = WebContext::new(Some(api::login_data_dir()));
    let expected = state.clone();
    let _webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(authorize_url(&state))
        .with_navigation_handler(move |url| {
            if !url.starts_with("music-application:") {
                return true;
            }
            let saved = parse_token(&url, &expected).is_some_and(|token| api::save_token(&token).is_ok());
            std::process::exit(if saved { 0 } else { 1 });
        })
        .build(&window)
        .expect("WebView2 для входа");
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            std::process::exit(1);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_token_only_with_our_state() {
        let url = "music-application://desktop/oauth?language=ru#access_token=abc&token_type=bearer&expires_in=100&state=s1";
        assert_eq!(parse_token(url, "s1").as_deref(), Some("abc"));
        assert_eq!(parse_token(url, "s2"), None);
        assert_eq!(parse_token("music-application://desktop/oauth#error=access_denied&state=s1", "s1"), None);
    }

    #[test]
    fn url_encodes_redirect() {
        assert!(authorize_url("s").contains("redirect_uri=music-application%3A%2F%2Fdesktop%2Foauth&"));
    }
}
