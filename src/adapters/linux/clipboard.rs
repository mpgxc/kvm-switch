//! Área de transferência do GNOME pelos métodos D-Bus da extensão do GNOME Shell.
//!
//! No Wayland um app sem foco não lê nem grava a área de transferência; o
//! gnome-shell pode, então a extensão exporta `Ler`, `Gravar` e `Avisar` na
//! conexão de sessão dele (`org.gnome.Shell`).

// Sem uso até a Task 7 também nos testes, que só cobrem `clip_from` e `wire`.
#![expect(dead_code, reason = "usado a partir da ligação na Task 7")]

use std::{sync::OnceLock, time::Duration};

use anyhow::{Context as _, Result, bail};
use serde::Serialize;
use zbus::{
    blocking::{Connection, connection::Builder},
    zvariant::DynamicType,
};

use crate::{clip::Clip, ports::Clipboard};

/// Onde a extensão exporta os métodos (ver `extension.js`).
const DESTINO: &str = "org.gnome.Shell";
const CAMINHO: &str = "/io/github/mpgxc/KvmSwitch";
const INTERFACE: &str = "io.github.mpgxc.KvmSwitch";

/// Um app dono da área de transferência travado deixa o `Ler` sem resposta; sem
/// limite a chamada (e a troca pela borda) esperaria para sempre.
const ESPERA: Duration = Duration::from_secs(2);

/// A área de transferência da sessão GNOME, pela extensão.
pub struct GnomeClipboard;

impl GnomeClipboard {
    /// Mostra uma notificação do GNOME com `text`.
    pub fn notify(&self, text: &str) -> Result<()> {
        call("Avisar", &(text,)).map(drop)
    }
}

impl Clipboard for GnomeClipboard {
    fn read(&self) -> Result<Option<Clip>> {
        let (kind, data): (String, Vec<u8>) = call("Ler", &())?
            .body()
            .deserialize()
            .context("resposta inesperada da extensão do GNOME")?;
        clip_from(&kind, data)
    }

    fn write(&self, clip: &Clip) -> Result<()> {
        call("Gravar", &wire(clip)).map(drop)
    }
}

/// Chama `method` da extensão, abrindo a conexão de sessão na primeira vez.
fn call<B: Serialize + DynamicType>(method: &str, body: &B) -> Result<zbus::Message> {
    static SESSAO: OnceLock<Connection> = OnceLock::new();
    let sessao = match SESSAO.get() {
        Some(sessao) => sessao,
        None => {
            let nova = Builder::session()
                .and_then(|builder| builder.method_timeout(ESPERA).build())
                .context("sem conexão D-Bus de sessão")?;
            SESSAO.get_or_init(|| nova)
        }
    };
    sessao
        .call_method(Some(DESTINO), CAMINHO, Some(INTERFACE), method, body)
        .with_context(|| {
            format!("a extensão do kvm-switch no GNOME Shell não respondeu a {method}")
        })
}

/// O `Clip` da resposta de `Ler`: tipo `"texto"`, `"png"` ou `""` (vazia).
fn clip_from(kind: &str, data: Vec<u8>) -> Result<Option<Clip>> {
    Ok(Some(match kind {
        "texto" => Clip::Text(
            String::from_utf8(data).context("o texto da área de transferência não é UTF-8")?,
        ),
        "png" => Clip::Png(data),
        "" => return Ok(None),
        _ => bail!("tipo desconhecido vindo da extensão do GNOME: {kind:?}"),
    }))
}

/// Os argumentos de `Gravar`: o tipo, com os nomes da extensão, e os bytes.
fn wire(clip: &Clip) -> (&'static str, &[u8]) {
    match clip {
        Clip::Text(text) => ("texto", text.as_bytes()),
        Clip::Png(png) => ("png", png),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_kind_becomes_text() -> Result<()> {
        let clip = clip_from("texto", "ação".into())?;
        assert_eq!(clip, Some(Clip::Text("ação".into())));
        Ok(())
    }

    #[test]
    fn png_kind_becomes_png() -> Result<()> {
        let clip = clip_from("png", vec![0x89, b'P', b'N', b'G'])?;
        assert_eq!(clip, Some(Clip::Png(vec![0x89, b'P', b'N', b'G'])));
        Ok(())
    }

    #[test]
    fn empty_kind_is_nothing() -> Result<()> {
        assert_eq!(clip_from("", vec![])?, None);
        Ok(())
    }

    #[test]
    fn unknown_kind_or_bad_utf8_is_an_error() {
        assert!(clip_from("gif", vec![1]).is_err());
        assert!(clip_from("texto", vec![0xff, 0xfe]).is_err());
    }

    #[test]
    fn wire_names_match_the_extension() {
        assert_eq!(wire(&Clip::Text("a".into())), ("texto", &b"a"[..]));
        assert_eq!(wire(&Clip::Png(vec![1, 2])), ("png", &[1, 2][..]));
    }
}
