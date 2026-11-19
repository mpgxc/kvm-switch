//! Interface em GPUI.

pub mod overlay;
pub mod settings;

use gpui::{AnyElement, Pixels, SharedString, div, prelude::*};

/// Identidade das janelas no Wayland; liga o app ao `kvm-switch.desktop`.
const APP_ID: &str = "kvm-switch";

/// Desenha o ícone de um destino com `size` de altura.
///
/// No macOS é o próprio emoji. No Linux o GPUI resolve vários deles (🖥, 💻, 🎮)
/// numa fonte de símbolos sem cor, e pedir a fonte de emoji pelo nome a tira do
/// ar. Lá os ícones oferecidos na janela saem de imagens; outro texto qualquer
/// continua sendo desenhado como texto.
fn icon(emoji: SharedString, size: Pixels) -> AnyElement {
    #[cfg(target_os = "linux")]
    if let Some(image) = icons::image(&emoji) {
        return gpui::img(image).size(size).into_any_element();
    }
    div().text_size(size).child(emoji).into_any_element()
}

/// Os ícones em imagem: PNGs do Noto Emoji, em `assets/icons`.
#[cfg(target_os = "linux")]
mod icons {
    use std::sync::{Arc, LazyLock};

    use gpui::{Image, ImageFormat};

    static IMAGES: LazyLock<Vec<(&str, Arc<Image>)>> = LazyLock::new(|| {
        let png = |bytes: &[u8]| Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec()));
        vec![
            ("🖥", png(include_bytes!("../assets/icons/1f5a5.png"))),
            ("💻", png(include_bytes!("../assets/icons/1f4bb.png"))),
            ("🍎", png(include_bytes!("../assets/icons/1f34e.png"))),
            ("🐧", png(include_bytes!("../assets/icons/1f427.png"))),
            ("🪟", png(include_bytes!("../assets/icons/1fa9f.png"))),
            ("🎮", png(include_bytes!("../assets/icons/1f3ae.png"))),
        ]
    });

    pub fn image(emoji: &str) -> Option<Arc<Image>> {
        let (_, image) = IMAGES.iter().find(|(known, _)| *known == emoji)?;
        Some(image.clone())
    }
}

/// Cores do tema escuro, em RGB.
mod theme {
    pub const BACKGROUND: u32 = 0x1e1f22;
    pub const FIELD: u32 = 0x2b2d31;
    pub const HOVER: u32 = 0x35373c;
    pub const BORDER: u32 = 0x3f4147;
    pub const TEXT: u32 = 0xe6e6e6;
    pub const MUTED: u32 = 0x9aa0a6;
    pub const ACCENT: u32 = 0x4f8cff;
    pub const ERROR: u32 = 0xff6b6b;
}
