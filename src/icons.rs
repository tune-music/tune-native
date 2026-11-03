//! Значки Яндекса. Как и шрифты, в приложение они не входят: набор берётся из клиента,
//! который лежит на этом компьютере у Tune, и один раз сохраняется рядом с настройками.

use std::collections::HashMap;
use std::fs;

use crate::{api, fonts};

const SPRITE: &str = "sprite.svg";

/// Каждый значок набора отдельной картинкой. Нет клиента — пусто, и остаются свои значки.
pub fn load() -> HashMap<String, Vec<u8>> {
    let cached = api::data_dir().join("icons").join(SPRITE);
    let sprite = fs::read_to_string(&cached).ok().or_else(|| {
        let bytes = fonts::read_from_asar(&fonts::asar()?, &["app", "icons"], SPRITE)?;
        let text = String::from_utf8(bytes).ok()?;
        if fs::create_dir_all(cached.parent()?).is_ok() {
            let _ = fs::write(&cached, &text);
        }
        Some(text)
    });
    sprite.map_or_else(HashMap::new, |sprite| symbols(&sprite))
}

/// Из `<symbol id=… viewBox=…>` делается самостоятельный svg. Цвет задаёт программа,
/// поэтому всё закрашенное становится белым, а ссылки на вырезанные из набора маски убираются:
/// с битой ссылкой фигура не нарисовалась бы вовсе.
fn symbols(sprite: &str) -> HashMap<String, Vec<u8>> {
    let mut icons = HashMap::new();
    for part in sprite.split("<symbol").skip(1) {
        let Some((open, rest)) = part.split_once('>') else { continue };
        let Some(body) = rest.split("</symbol>").next() else { continue };
        let (Some(id), Some(view)) = (attribute(open, "id"), attribute(open, "viewBox")) else { continue };
        let body = without(&without(body, "clip-path=\"", "\""), "<defs>", "</defs>").replace("currentColor", "#fff");
        icons.insert(id.to_string(), format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}" fill="none">{body}</svg>"#).into_bytes());
    }
    icons
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    Some(&tag[start..start + tag[start..].find('"')?])
}

/// Вырезает все куски от `from` до `to` включительно.
fn without(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(from) {
        out.push_str(&rest[..start]);
        match rest[start + from.len()..].find(to) {
            Some(end) => rest = &rest[start + from.len() + end + to.len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_standalone_white_icons() {
        let sprite = r#"<svg><symbol viewBox="0 0 20 20" fill="none" id="like_xs"><g clip-path="url(#c1)"><path d="M1 1" fill="currentColor" /></g><defs><clipPath id="c1"/></defs></symbol><symbol id="bad"></symbol></svg>"#;
        let icons = symbols(sprite);
        assert_eq!(icons.len(), 1);
        let like = String::from_utf8(icons["like_xs"].clone()).unwrap();
        assert!(like.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none">"#));
        assert!(like.contains(r##"<g ><path d="M1 1" fill="#fff" /></g>"##));
        assert!(!like.contains("defs") && !like.contains("clip-path"));
    }
}
