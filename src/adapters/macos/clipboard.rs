//! Área de transferência do macOS pelo NSPasteboard.
//!
//! O pasteboard é pedido ao AppKit a cada chamada, e não guardado: assim
//! `MacClipboard` é `Send + Sync` sem `unsafe impl` e pode ser usado fora da
//! thread do GPUI.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "usado a partir da ligação na Task 7")
)]

use anyhow::{Context as _, Result, bail};
use objc2::{msg_send, rc::Retained};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG,
    NSPasteboardTypeString, NSPasteboardTypeTIFF,
};
use objc2_foundation::{NSData, NSDictionary, NSString};

use crate::{clip::Clip, ports::Clipboard};

/// `name`: `None` é a área geral do usuário; `Some` é um pasteboard só nosso (testes).
pub struct MacClipboard {
    name: Option<String>,
}

impl MacClipboard {
    /// A área de transferência do usuário.
    #[cfg_attr(
        test,
        expect(dead_code, reason = "os testes nunca tocam a área do usuário")
    )]
    pub fn general() -> Self {
        Self { name: None }
    }

    /// Pasteboard de nome único, solto no `Drop`.
    #[cfg(test)]
    fn private() -> Self {
        let name = NSPasteboard::pasteboardWithUniqueName().name().to_string();
        Self { name: Some(name) }
    }

    fn board(&self) -> Retained<NSPasteboard> {
        match &self.name {
            None => NSPasteboard::generalPasteboard(),
            Some(name) => NSPasteboard::pasteboardWithName(&NSString::from_str(name)),
        }
    }
}

impl Drop for MacClipboard {
    fn drop(&mut self) {
        if self.name.is_some() {
            // SAFETY: `releaseGlobally` existe no NSPasteboard, sem argumentos nem retorno.
            let () = unsafe { msg_send![&*self.board(), releaseGlobally] };
        }
    }
}

impl Clipboard for MacClipboard {
    fn read(&self) -> Result<Option<Clip>> {
        let board = self.board();
        // SAFETY: constantes do AppKit, só lidas.
        let (string, png, tiff) = unsafe {
            (
                NSPasteboardTypeString,
                NSPasteboardTypePNG,
                NSPasteboardTypeTIFF,
            )
        };
        if let Some(text) = board.stringForType(string) {
            return Ok(Some(Clip::Text(text.to_string())));
        }
        if let Some(data) = board.dataForType(png) {
            return Ok(Some(Clip::Png(data.to_vec())));
        }
        let Some(data) = board.dataForType(tiff) else {
            return Ok(None);
        };
        // Prints e apps antigos só põem TIFF; o vizinho recebe sempre PNG.
        let rep = NSBitmapImageRep::imageRepWithData(&data)
            .context("o AppKit não leu o TIFF da área de transferência")?;
        // SAFETY: dicionário vazio, então sem valores de tipo errado.
        let converted = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .context("o AppKit não converteu o TIFF em PNG")?;
        Ok(Some(Clip::Png(converted.to_vec())))
    }

    fn write(&self, clip: &Clip) -> Result<()> {
        let board = self.board();
        board.clearContents();
        // SAFETY: constantes do AppKit, só lidas.
        let (string, png_type) = unsafe { (NSPasteboardTypeString, NSPasteboardTypePNG) };
        let written = match clip {
            Clip::Text(text) => board.setString_forType(&NSString::from_str(text), string),
            Clip::Png(png) => board.setData_forType(Some(&NSData::with_bytes(png)), png_type),
        };
        if !written {
            bail!("o macOS recusou gravar na área de transferência");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use objc2_app_kit::{
        NSBitmapImageRep, NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardTypeTIFF,
    };
    use objc2_foundation::{NSData, NSString};

    use super::*;

    /// PNG de 1×1 pixel, gerado aqui para não depender de arquivo.
    fn png_1x1() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("cabeçalho do PNG falhou");
        writer
            .write_image_data(&[255, 0, 0, 255])
            .expect("pixel do PNG falhou");
        writer.finish().expect("fim do PNG falhou");
        bytes
    }

    #[test]
    fn text_written_reads_back_identical() {
        let clipboard = MacClipboard::private();
        let clip = Clip::Text("ação 🚀\r\n".to_string());
        clipboard.write(&clip).expect("gravar falhou");
        assert_eq!(clipboard.read().expect("ler falhou"), Some(clip));
    }

    #[test]
    fn png_written_reads_back_identical() {
        let clipboard = MacClipboard::private();
        let clip = Clip::Png(png_1x1());
        clipboard.write(&clip).expect("gravar falhou");
        assert_eq!(clipboard.read().expect("ler falhou"), Some(clip));
    }

    #[test]
    fn text_wins_over_an_image() {
        let clipboard = MacClipboard::private();
        let board = clipboard.board();
        board.clearContents();
        // SAFETY: constantes do AppKit, só lidas.
        let (string, png) = unsafe { (NSPasteboardTypeString, NSPasteboardTypePNG) };
        assert!(board.setString_forType(&NSString::from_str("texto"), string));
        assert!(board.setData_forType(Some(&NSData::with_bytes(&png_1x1())), png));
        assert_eq!(
            clipboard.read().expect("ler falhou"),
            Some(Clip::Text("texto".to_string()))
        );
    }

    #[test]
    fn tiff_only_reads_as_png() {
        let clipboard = MacClipboard::private();
        let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(&png_1x1()))
            .expect("o AppKit não leu o PNG");
        let tiff = rep.TIFFRepresentation().expect("o AppKit não gerou o TIFF");
        let board = clipboard.board();
        board.clearContents();
        // SAFETY: constante do AppKit, só lida.
        assert!(board.setData_forType(Some(&tiff), unsafe { NSPasteboardTypeTIFF }));

        let Some(Clip::Png(bytes)) = clipboard.read().expect("ler falhou") else {
            panic!("o TIFF não virou PNG");
        };
        let info = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .expect("o PNG convertido não decodifica");
        assert_eq!((info.info().width, info.info().height), (1, 1));
    }

    #[test]
    fn empty_board_reads_none() {
        let clipboard = MacClipboard::private();
        clipboard.board().clearContents();
        assert_eq!(clipboard.read().expect("ler falhou"), None);
    }
}
