//! Área de transferência do Windows pela API Win32.
//!
//! Texto vai em `CF_UNICODETEXT`; imagem no formato registrado `"PNG"` (lido de
//! volta como está) e também em `CF_DIB`, porque vários programas só colam DIB.
//! Prints (Win+Shift+S, Paint) só põem DIB, convertido aqui para PNG.

use std::{iter, ptr, thread, time::Duration};

use anyhow::{Context as _, Result, bail};
use windows_sys::{
    Win32::{
        Foundation::{GlobalFree, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
                RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{CF_DIB, CF_DIBV5, CF_UNICODETEXT},
        },
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE},
    },
    w,
};

use crate::{
    adapters::dib::{dib_to_png, png_to_dib},
    clip::Clip,
    ports::Clipboard,
};

/// Tentativas de abrir a área de transferência, que outro programa pode estar usando.
const OPEN_ATTEMPTS: usize = 10;
const OPEN_INTERVAL: Duration = Duration::from_millis(10);

/// A área de transferência do usuário; sem estado, então `Send + Sync`.
pub struct WindowsClipboard;

/// A área de transferência aberta por nós; fecha no `Drop`, até em erro.
struct Opened;

impl Opened {
    fn new(owner: HWND) -> Result<Self> {
        for _ in 0..OPEN_ATTEMPTS {
            // SAFETY: `owner` é nulo ou uma janela nossa ainda viva.
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Opened);
            }
            thread::sleep(OPEN_INTERVAL);
        }
        bail!("a área de transferência do Windows está ocupada por outro programa")
    }

    /// Cópia dos bytes do formato, se houver.
    fn data(&self, format: u32) -> Option<Vec<u8>> {
        // SAFETY: a área está aberta (`self`); o handle é do sistema e só é lido.
        let handle = unsafe { GetClipboardData(format) };
        if handle.is_null() {
            return None;
        }
        // SAFETY: handle de memória global válido enquanto a área está aberta.
        let (pointer, len) = unsafe { (GlobalLock(handle), GlobalSize(handle)) };
        if pointer.is_null() {
            return None;
        }
        // SAFETY: `GlobalLock` deu `len` bytes legíveis até o `GlobalUnlock`.
        let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), len) }.to_vec();
        // SAFETY: mesmo handle travado logo acima.
        unsafe { GlobalUnlock(handle) };
        Some(bytes)
    }

    /// Entrega `bytes` ao sistema; `false` se `SetClipboardData` recusou.
    fn put(&self, format: u32, bytes: &[u8]) -> Result<bool> {
        // SAFETY: só aloca; o retorno nulo é tratado.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
        if handle.is_null() {
            bail!("sem memória para a área de transferência");
        }
        // SAFETY: handle recém-alocado com `bytes.len()` bytes.
        let pointer = unsafe { GlobalLock(handle) };
        if pointer.is_null() {
            // SAFETY: o handle ainda é nosso.
            unsafe { GlobalFree(handle) };
            bail!("o Windows não travou a memória da área de transferência");
        }
        // SAFETY: destino travado com `bytes.len()` bytes, sem sobreposição.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
            GlobalUnlock(handle);
        }
        // SAFETY: a área está aberta (`self`); em sucesso o handle passa a ser do sistema.
        if unsafe { SetClipboardData(format, handle) }.is_null() {
            // SAFETY: recusado, o handle continua nosso.
            unsafe { GlobalFree(handle) };
            return Ok(false);
        }
        Ok(true)
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: a área foi aberta por esta thread em `Opened::new`.
        unsafe { CloseClipboard() };
    }
}

/// Janela de mensagens oculta, dona da área de transferência quando o Windows
/// recusa gravar sem dono.
struct MessageWindow(HWND);

impl MessageWindow {
    fn new() -> Result<Self> {
        // SAFETY: classe "STATIC" do sistema, sem parâmetros apontando para nossos dados.
        let window = unsafe {
            CreateWindowExW(
                0,
                w!("STATIC"),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if window.is_null() {
            bail!("o Windows não criou a janela dona da área de transferência");
        }
        Ok(MessageWindow(window))
    }
}

impl Drop for MessageWindow {
    fn drop(&mut self) {
        // SAFETY: janela criada por esta thread em `MessageWindow::new`.
        unsafe { DestroyWindow(self.0) };
    }
}

/// Número do formato registrado `"PNG"`, o mesmo em todos os programas.
fn png_format() -> Result<u32> {
    // SAFETY: texto UTF-16 terminado em NUL, estático.
    match unsafe { RegisterClipboardFormatW(w!("PNG")) } {
        0 => bail!("o Windows não registrou o formato PNG"),
        format => Ok(format),
    }
}

/// Esvazia e grava todos os `formats`; `false` se o Windows recusou algum.
fn put_all(owner: HWND, formats: &[(u32, &[u8])]) -> Result<bool> {
    let opened = Opened::new(owner)?;
    // SAFETY: a área está aberta (`opened`).
    if unsafe { EmptyClipboard() } == 0 {
        bail!("o Windows não esvaziou a área de transferência");
    }
    for &(format, bytes) in formats {
        if !opened.put(format, bytes)? {
            return Ok(false);
        }
    }
    Ok(true)
}

impl Clipboard for WindowsClipboard {
    fn read(&self) -> Result<Option<Clip>> {
        let png = png_format()?;
        let opened = Opened::new(ptr::null_mut())?;
        if let Some(bytes) = opened.data(CF_UNICODETEXT.into()) {
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| u16::from_le_bytes(pair))
                .take_while(|&unit| unit != 0)
                .collect();
            let text = String::from_utf16(&units)
                .context("o texto da área de transferência não é UTF-16 válido")?;
            return Ok(Some(Clip::Text(text)));
        }
        if let Some(bytes) = opened.data(png) {
            return Ok(Some(Clip::Png(bytes)));
        }
        for format in [CF_DIBV5, CF_DIB] {
            if let Some(dib) = opened.data(format.into()) {
                return Ok(dib_to_png(&dib).map(Clip::Png));
            }
        }
        Ok(None)
    }

    fn write(&self, clip: &Clip) -> Result<()> {
        let text: Vec<u8>;
        let dib: Option<Vec<u8>>;
        let mut formats: Vec<(u32, &[u8])> = Vec::new();
        match clip {
            Clip::Text(value) => {
                text = value
                    .encode_utf16()
                    .chain(iter::once(0))
                    .flat_map(u16::to_le_bytes)
                    .collect();
                formats.push((CF_UNICODETEXT.into(), &text));
            }
            Clip::Png(png) => {
                formats.push((png_format()?, png));
                dib = png_to_dib(png);
                // Sem DIB (PNG que não decodifica), o "PNG" basta para quem o lê.
                if let Some(dib) = &dib {
                    formats.push((CF_DIB.into(), dib));
                }
            }
        }
        if put_all(ptr::null_mut(), &formats)? {
            return Ok(());
        }
        let window = MessageWindow::new()?;
        if !put_all(window.0, &formats)? {
            bail!("o Windows recusou gravar na área de transferência");
        }
        Ok(())
    }
}
