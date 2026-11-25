//! Conversão entre o bitmap do Windows (DIB) e PNG.
//!
//! Sem chamadas ao sistema, para os testes rodarem em qualquer lugar: a área de
//! transferência do Windows guarda prints como DIB, e o vizinho só entende PNG.

use std::io::Cursor;

use png::{BitDepth, ColorType, Transformations};

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
/// BITMAPINFOHEADER; o BITMAPV5HEADER tem 124 bytes.
const INFO_HEADER: usize = 40;
const V5_HEADER: usize = 124;
/// Máscaras vermelho, verde e azul de 32 bits BGRA, logo depois dos 40 bytes
/// (dentro do cabeçalho V5, ou após o BITMAPINFOHEADER).
const STANDARD_MASKS: [u32; 3] = [0x00ff_0000, 0x0000_ff00, 0x0000_00ff];

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Conteúdo de CF_DIB ou CF_DIBV5: cabeçalho de 40 ou 124 bytes, 24 ou 32 bits,
/// BI_RGB ou BI_BITFIELDS com as máscaras padrão, de baixo para cima ou de cima
/// para baixo. Outro formato ou dado cortado: `None`.
pub fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>> {
    let size = u32_at(dib, 0)? as usize;
    if size != INFO_HEADER && size != V5_HEADER {
        return None;
    }
    let width = u32_at(dib, 4)? as i32;
    let height = u32_at(dib, 8)? as i32;
    let bits = u32_at(dib, 12)? >> 16;
    let compression = u32_at(dib, 16)?;
    let palette = u32_at(dib, 32)? as usize;
    let masks_after = match (compression, bits) {
        (BI_RGB, 24 | 32) => 0,
        (BI_BITFIELDS, 32) => {
            let masks = [u32_at(dib, 40)?, u32_at(dib, 44)?, u32_at(dib, 48)?];
            if masks != STANDARD_MASKS {
                return None;
            }
            if size == INFO_HEADER { 12 } else { 0 }
        }
        _ => return None,
    };
    let width = usize::try_from(width).ok().filter(|&w| w > 0)?;
    let rows = height.unsigned_abs() as usize;
    if rows == 0 {
        return None;
    }
    let pixel_len = bits as usize / 8;
    let line = width.checked_mul(pixel_len)?;
    let stride = line.checked_next_multiple_of(4)?;
    let start = size + masks_after + palette.checked_mul(4)?;
    let pixels = dib.get(start..start.checked_add(stride.checked_mul(rows)?)?)?;

    let mut rgba = Vec::with_capacity(width * rows * 4);
    for row in 0..rows {
        // Altura positiva: a primeira linha guardada é a de baixo.
        let stored = if height > 0 { rows - 1 - row } else { row };
        for px in pixels[stored * stride..][..line].chunks_exact(pixel_len) {
            let alpha = if pixel_len == 4 { px[3] } else { 255 };
            rgba.extend_from_slice(&[px[2], px[1], px[0], alpha]);
        }
    }
    // Muitos programas deixam o quarto byte zerado: lido como alfa, sumiria tudo.
    if pixel_len == 4 && rgba.iter().skip(3).step_by(4).all(|&alpha| alpha == 0) {
        rgba.iter_mut()
            .skip(3)
            .step_by(4)
            .for_each(|alpha| *alpha = 255);
    }

    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width as u32, rows as u32);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&rgba).ok()?;
    writer.finish().ok()?;
    Some(png)
}

/// BITMAPINFOHEADER de 32 bits, BI_RGB, de baixo para cima, BGRA.
pub fn png_to_dib(png: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixel_len = match info.color_type {
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Indexed => return None,
    };
    let image_len = u32::try_from(
        (info.width as usize)
            .checked_mul(info.height as usize)?
            .checked_mul(4)?,
    )
    .ok()?;

    let mut dib = Vec::with_capacity(INFO_HEADER + image_len as usize);
    dib.extend_from_slice(&(INFO_HEADER as u32).to_le_bytes());
    dib.extend_from_slice(&i32::try_from(info.width).ok()?.to_le_bytes());
    dib.extend_from_slice(&i32::try_from(info.height).ok()?.to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes()); // planos
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&BI_RGB.to_le_bytes());
    dib.extend_from_slice(&image_len.to_le_bytes());
    dib.resize(INFO_HEADER, 0);
    for line in buffer[..info.buffer_size()]
        .chunks_exact(info.line_size)
        .rev()
    {
        for px in line.chunks_exact(pixel_len) {
            let (r, g, b, a) = match *px {
                [v] => (v, v, v, 255),
                [v, a] => (v, v, v, a),
                [r, g, b] => (r, g, b, 255),
                [r, g, b, a] => (r, g, b, a),
                _ => unreachable!("chunks_exact de 1 a 4 bytes"),
            };
            dib.extend_from_slice(&[b, g, r, a]);
        }
    }
    Some(dib)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;

    /// BITMAPINFOHEADER (40 bytes) ou BITMAPV5HEADER (124, com as máscaras padrão).
    fn header(size: u32, width: i32, height: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend_from_slice(&size.to_le_bytes());
        header.extend_from_slice(&width.to_le_bytes());
        header.extend_from_slice(&height.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&bits.to_le_bytes());
        header.extend_from_slice(&compression.to_le_bytes());
        header.resize(40, 0);
        if size == 124 {
            for mask in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff, 0xff00_0000] {
                header.extend_from_slice(&mask.to_le_bytes());
            }
            header.resize(124, 0);
        }
        header
    }

    /// Largura, altura e pixels RGBA de um PNG.
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .expect("o PNG não decodifica");
        let mut pixels = vec![0; reader.output_buffer_size().expect("PNG grande demais")];
        let info = reader.next_frame(&mut pixels).expect("quadro do PNG");
        assert_eq!(info.color_type, png::ColorType::Rgba);
        pixels.truncate(info.buffer_size());
        (info.width, info.height, pixels)
    }

    #[test]
    fn a_bottom_up_24_bit_dib_becomes_the_same_pixels() {
        let mut dib = header(40, 2, 2, 24, BI_RGB);
        // De baixo para cima: a primeira linha guardada é a de baixo. Linhas de
        // 6 bytes completadas até 8.
        dib.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]); // baixo: vermelho, verde
        dib.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]); // cima: azul, branco
        let (width, height, pixels) = decode(&dib_to_png(&dib).expect("DIB recusado"));
        assert_eq!((width, height), (2, 2));
        assert_eq!(
            pixels,
            [
                0, 0, 255, 255, 255, 255, 255, 255, // cima
                255, 0, 0, 255, 0, 255, 0, 255, // baixo
            ]
        );
    }

    #[test]
    fn a_top_down_32_bit_dib_keeps_row_order() {
        let mut dib = header(40, 1, -2, 32, BI_RGB);
        dib.extend_from_slice(&[0, 0, 255, 128]); // cima: vermelho meio transparente
        dib.extend_from_slice(&[255, 0, 0, 255]); // baixo: azul
        let (width, height, pixels) = decode(&dib_to_png(&dib).expect("DIB recusado"));
        assert_eq!((width, height), (1, 2));
        assert_eq!(pixels, [255, 0, 0, 128, 0, 0, 255, 255]);
    }

    #[test]
    fn a_32_bit_dib_with_zero_alpha_is_opaque() {
        let mut dib = header(40, 2, 1, 32, BI_RGB);
        dib.extend_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]);
        let (_, _, pixels) = decode(&dib_to_png(&dib).expect("DIB recusado"));
        assert_eq!(pixels, [3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn a_bitfields_v5_dib_becomes_the_same_pixels() {
        let mut dib = header(124, 2, 1, 32, BI_BITFIELDS);
        dib.extend_from_slice(&[10, 20, 30, 255, 40, 50, 60, 200]);
        let (width, height, pixels) = decode(&dib_to_png(&dib).expect("DIB recusado"));
        assert_eq!((width, height), (2, 1));
        assert_eq!(pixels, [30, 20, 10, 255, 60, 50, 40, 200]);
    }

    #[test]
    fn png_to_dib_then_back_keeps_the_pixels() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 9, 8, 7, 255];
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("cabeçalho do PNG falhou");
        writer
            .write_image_data(&rgba)
            .expect("pixels do PNG falharam");
        writer.finish().expect("fim do PNG falhou");

        let dib = png_to_dib(&png).expect("PNG recusado");
        // Tamanho, largura, altura positiva (de baixo para cima), 32 bits, BI_RGB.
        assert_eq!(dib[..20], header(40, 2, 2, 32, BI_RGB)[..20]);
        let (width, height, pixels) = decode(&dib_to_png(&dib).expect("DIB recusado"));
        assert_eq!((width, height), (2, 2));
        assert_eq!(pixels, rgba);
    }

    #[test]
    fn unsupported_dibs_are_none() {
        let mut eight_bit = header(40, 1, 1, 8, BI_RGB);
        eight_bit.extend_from_slice(&[0; 4 * 256 + 4]);
        assert_eq!(dib_to_png(&eight_bit), None);

        let mut cut = header(40, 2, 2, 32, BI_RGB);
        cut.extend_from_slice(&[0; 12]);
        assert_eq!(dib_to_png(&cut), None);

        assert_eq!(dib_to_png(&header(40, 1, 1, 32, BI_RGB)[..20]), None);
    }
}
