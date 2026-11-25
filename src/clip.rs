//! Área de transferência entre PCs vizinhos: o conteúdo, a chave e o quadro cifrado.
//!
//! Sem sistema operacional aqui: ler e gravar a área de transferência e a rede
//! ficam nos adaptadores. Quadro: `"KVMC"` + versão + nonce de 12 bytes + tamanho
//! `u32` (big-endian) + corpo cifrado com ChaCha20-Poly1305, cujo texto claro é o
//! tipo `u8` (1 texto, 2 PNG) + dados. O dado associado é `"KVMC"` + versão + o
//! desafio que quem recebe mandou nesta conexão: um quadro capturado não vale em
//! outra conexão, nem em outro PC, nem no próprio remetente.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "usado a partir da ligação na Task 7")
)]

use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom as _, SystemRandom};

/// Porta TCP fixa onde cada PC ouve o vizinho (ao lado da 24800 do Deskflow).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "usado a partir da ligação na Task 7")
)]
pub const PORT: u16 = 24811;
/// Maior conteúdo aceito; acima disso não se envia nem se aloca ao receber.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Tamanho do desafio aleatório que quem recebe manda a cada conexão.
pub const CHALLENGE_LEN: usize = 16;
/// "KVMC" + versão + nonce + tamanho
pub const HEADER_LEN: usize = 21;

const MAGIC: &[u8; 4] = b"KVMC";
const VERSION: u8 = 1;
/// "KVMC" + versão, os bytes do cabeçalho que entram no dado associado.
const PREFIX_LEN: usize = 5;
const NONCE_LEN: usize = 12;
/// O tipo (1 byte), antes dos dados.
const KIND_LEN: usize = 1;
const TAG_LEN: usize = 16;
const TEXT: u8 = 1;
const PNG: u8 = 2;
/// Avisos ao usuário espaçados assim, para um vizinho insistente não virar spam.
const NOTICE_EVERY: Duration = Duration::from_secs(60);

/// O que vai de um PC ao outro: texto ou imagem já em PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    Text(String),
    Png(Vec<u8>),
}

/// A chave comum aos PCs; quem não a tem não lê nem forja quadros.
pub struct Key(LessSafeKey);

impl Key {
    /// 32 bytes de `ring::rand::SystemRandom`, em base64 padrão (44 caracteres).
    pub fn generate() -> String {
        let mut bytes = [0u8; 32];
        SystemRandom::new()
            .fill(&mut bytes)
            .expect("o gerador aleatório do sistema falhou");
        STANDARD.encode(bytes)
    }

    /// Aceita espaços e quebras de linha nas pontas; erro se não forem 32 bytes em base64.
    pub fn parse(text: &str) -> anyhow::Result<Key> {
        let bytes = STANDARD
            .decode(text.trim())
            .context("a chave não está em base64")?;
        if bytes.len() != 32 {
            bail!("a chave tem {} bytes, e não 32", bytes.len());
        }
        let key = UnboundKey::new(&CHACHA20_POLY1305, &bytes)
            .map_err(|_| anyhow!("a chave não serve para ChaCha20-Poly1305"))?;
        Ok(Key(LessSafeKey::new(key)))
    }
}

/// Por que um quadro recebido foi descartado (sem resposta a quem enviou).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotOurs,
    Version,
    TooBig,
    WrongKey,
    Kind,
}

impl Refusal {
    /// Só `WrongKey` merece aviso a quem está na frente do PC.
    pub fn notice(self) -> Option<&'static str> {
        match self {
            Refusal::WrongKey => Some(
                "Chegou uma área de transferência com outra chave: confira a chave nos dois PCs",
            ),
            _ => None,
        }
    }
}

/// 16 bytes de `SystemRandom`, um por conexão recebida.
pub fn new_challenge() -> [u8; CHALLENGE_LEN] {
    let mut challenge = [0u8; CHALLENGE_LEN];
    SystemRandom::new()
        .fill(&mut challenge)
        .expect("o gerador aleatório do sistema falhou");
    challenge
}

/// O dado associado: o prefixo do cabeçalho seguido do desafio desta conexão.
fn aad(header: &[u8], challenge: &[u8; CHALLENGE_LEN]) -> [u8; PREFIX_LEN + CHALLENGE_LEN] {
    let mut aad = [0u8; PREFIX_LEN + CHALLENGE_LEN];
    aad[..PREFIX_LEN].copy_from_slice(&header[..PREFIX_LEN]);
    aad[PREFIX_LEN..].copy_from_slice(challenge);
    aad
}

/// Erro quando o conteúdo passa de `MAX_BYTES`.
pub fn seal(clip: &Clip, key: &Key, challenge: &[u8; CHALLENGE_LEN]) -> anyhow::Result<Vec<u8>> {
    let (kind, data) = match clip {
        Clip::Text(text) => (TEXT, text.as_bytes()),
        Clip::Png(png) => (PNG, png.as_slice()),
    };
    if data.len() > MAX_BYTES {
        bail!(
            "a área de transferência tem {} bytes, acima do limite de {MAX_BYTES}",
            data.len()
        );
    }
    seal_kind(kind, data, key, challenge)
}

/// Monta o quadro com qualquer tipo; separado de `seal` para os testes forjarem tipos.
fn seal_kind(
    kind: u8,
    data: &[u8],
    key: &Key,
    challenge: &[u8; CHALLENGE_LEN],
) -> anyhow::Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow!("o gerador aleatório do sistema falhou"))?;
    let body_len = u32::try_from(KIND_LEN + data.len() + TAG_LEN)
        .context("a área de transferência não cabe no tamanho do quadro")?;

    let mut frame = Vec::with_capacity(HEADER_LEN + body_len as usize);
    frame.extend_from_slice(MAGIC);
    frame.push(VERSION);
    frame.extend_from_slice(&nonce);
    frame.extend_from_slice(&body_len.to_be_bytes());
    frame.push(kind);
    frame.extend_from_slice(data);

    let (header, plain) = frame.split_at_mut(HEADER_LEN);
    let tag = key
        .0
        .seal_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad(header, challenge)),
            plain,
        )
        .map_err(|_| anyhow!("não foi possível cifrar a área de transferência"))?;
    frame.extend_from_slice(tag.as_ref());
    Ok(frame)
}

/// Confere "KVMC", versão 1 e o tamanho antes de alguém alocar o corpo.
pub fn body_len(header: &[u8; HEADER_LEN]) -> Result<usize, Refusal> {
    if header[..4] != MAGIC[..] {
        return Err(Refusal::NotOurs);
    }
    if header[4] != VERSION {
        return Err(Refusal::Version);
    }
    let len = u32::from_be_bytes([header[17], header[18], header[19], header[20]]) as usize;
    if len > MAX_BYTES + KIND_LEN + TAG_LEN {
        return Err(Refusal::TooBig);
    }
    Ok(len)
}

/// Decifra (chave, quadro intacto e desafio desta conexão) e confere o tipo.
pub fn open(
    header: &[u8; HEADER_LEN],
    mut body: Vec<u8>,
    key: &Key,
    challenge: &[u8; CHALLENGE_LEN],
) -> Result<Clip, Refusal> {
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&header[PREFIX_LEN..PREFIX_LEN + NONCE_LEN]);
    let plain_len = key
        .0
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad(header, challenge)),
            &mut body,
        )
        .map_err(|_| Refusal::WrongKey)?
        .len();
    // Reaproveita o corpo (até 16 MiB) em vez de copiar os dados.
    body.truncate(plain_len);
    // Com a chave certa, mas vazio: nem o tipo veio.
    if body.is_empty() {
        return Err(Refusal::Kind);
    }
    let kind = body.remove(0);
    match kind {
        TEXT => String::from_utf8(body)
            .map(Clip::Text)
            .map_err(|_| Refusal::Kind),
        PNG => Ok(Clip::Png(body)),
        _ => Err(Refusal::Kind),
    }
}

/// Último aviso mostrado, para avisar no máximo um por minuto.
#[derive(Default)]
pub struct NoticeGate(Option<Instant>);

impl NoticeGate {
    /// `true` (e conta como aviso dado) se o último foi há 60 s ou mais.
    pub fn allow(&mut self, now: Instant) -> bool {
        if self
            .0
            .is_some_and(|last| now.saturating_duration_since(last) < NOTICE_EVERY)
        {
            return false;
        }
        self.0 = Some(now);
        true
    }
}

/// Grava só se o que chegou é diferente do que já está copiado.
pub fn should_write(current: Option<&Clip>, incoming: &Clip) -> bool {
    current != Some(incoming)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHALLENGE: [u8; CHALLENGE_LEN] = [7; CHALLENGE_LEN];

    fn key() -> Key {
        Key::parse(&Key::generate()).expect("chave gerada inválida")
    }

    /// Separa cabeçalho e corpo de um quadro, como faz quem lê da rede.
    fn frame(bytes: Vec<u8>) -> ([u8; HEADER_LEN], Vec<u8>) {
        let header = bytes[..HEADER_LEN]
            .try_into()
            .expect("quadro sem cabeçalho");
        (header, bytes[HEADER_LEN..].to_vec())
    }

    fn roundtrip(clip: &Clip) -> Result<Clip, Refusal> {
        let key = key();
        let challenge = new_challenge();
        let (header, body) = frame(seal(clip, &key, &challenge).expect("seal falhou"));
        assert_eq!(body_len(&header), Ok(body.len()));
        open(&header, body, &key, &challenge)
    }

    #[test]
    fn seal_then_open_returns_the_same_text() {
        let clip = Clip::Text("ação 🚀\r\nfim".to_string());
        assert_eq!(roundtrip(&clip), Ok(clip));
    }

    #[test]
    fn seal_then_open_returns_the_same_png() {
        let clip = Clip::Png(vec![0x89, b'P', b'N', b'G', 1, 2, 3]);
        assert_eq!(roundtrip(&clip), Ok(clip));
    }

    #[test]
    fn another_key_is_refused_as_wrong_key() {
        let clip = Clip::Text("a".to_string());
        let (header, body) = frame(seal(&clip, &key(), &CHALLENGE).expect("seal falhou"));
        assert_eq!(
            open(&header, body, &key(), &CHALLENGE),
            Err(Refusal::WrongKey)
        );
    }

    #[test]
    fn a_flipped_byte_is_refused_as_wrong_key() {
        let key = key();
        let clip = Clip::Text("a".to_string());
        let (header, mut body) = frame(seal(&clip, &key, &CHALLENGE).expect("seal falhou"));
        *body.last_mut().expect("corpo vazio") ^= 1;
        assert_eq!(
            open(&header, body, &key, &CHALLENGE),
            Err(Refusal::WrongKey)
        );
    }

    #[test]
    fn a_frame_for_another_challenge_is_refused_as_wrong_key() {
        let key = key();
        let clip = Clip::Text("a".to_string());
        let (header, body) = frame(seal(&clip, &key, &CHALLENGE).expect("seal falhou"));
        let other = [8; CHALLENGE_LEN];
        assert_eq!(open(&header, body, &key, &other), Err(Refusal::WrongKey));
    }

    #[test]
    fn a_tampered_nonce_is_refused_as_wrong_key() {
        let key = key();
        let clip = Clip::Text("a".to_string());
        let (mut header, body) = frame(seal(&clip, &key, &CHALLENGE).expect("seal falhou"));
        header[5] ^= 1;
        assert_eq!(
            open(&header, body, &key, &CHALLENGE),
            Err(Refusal::WrongKey)
        );
    }

    #[test]
    fn body_len_refuses_unknown_magic_version_and_oversize() {
        let clip = Clip::Text("a".to_string());
        let (header, _) = frame(seal(&clip, &key(), &CHALLENGE).expect("seal falhou"));

        let mut magic = header;
        magic[..4].copy_from_slice(b"XXXX");
        assert_eq!(body_len(&magic), Err(Refusal::NotOurs));

        let mut version = header;
        version[4] = 2;
        assert_eq!(body_len(&version), Err(Refusal::Version));

        let mut size = header;
        let limit = u32::try_from(MAX_BYTES + 17).expect("limite cabe em u32");
        size[17..].copy_from_slice(&limit.to_be_bytes());
        assert_eq!(body_len(&size), Ok(MAX_BYTES + 17));
        size[17..].copy_from_slice(&(limit + 1).to_be_bytes());
        assert_eq!(body_len(&size), Err(Refusal::TooBig));
    }

    #[test]
    fn seal_refuses_a_clip_over_the_limit() {
        let clip = Clip::Png(vec![0; MAX_BYTES + 1]);
        assert!(seal(&clip, &key(), &CHALLENGE).is_err());
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let key = key();
        for (kind, data) in [(3, &b"a"[..]), (1, &[0xff][..])] {
            let sealed = seal_kind(kind, data, &key, &CHALLENGE).expect("seal falhou");
            let (header, body) = frame(sealed);
            let result = open(&header, body, &key, &CHALLENGE);
            assert_eq!(result, Err(Refusal::Kind), "tipo {kind}");
        }
    }

    #[test]
    fn generated_keys_parse() {
        let text = Key::generate();
        assert_eq!(text.len(), 44);
        assert!(Key::parse(&text).is_ok());
    }

    #[test]
    fn key_parse_trims_whitespace() {
        assert!(Key::parse(&format!("  {}\n", Key::generate())).is_ok());
    }

    #[test]
    fn key_parse_refuses_wrong_length_and_garbage() {
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        assert!(Key::parse(&short).is_err());
        assert!(Key::parse("não é base64").is_err());
        assert!(Key::parse("").is_err());
    }

    #[test]
    fn only_wrong_key_has_a_notice() {
        use Refusal::*;
        for refusal in [NotOurs, Version, TooBig, Kind] {
            assert_eq!(refusal.notice(), None, "{refusal:?}");
        }
        assert!(WrongKey.notice().is_some());
    }

    #[test]
    fn notice_gate_allows_one_per_minute() {
        let mut gate = NoticeGate::default();
        let t = Instant::now();
        assert!(gate.allow(t));
        assert!(!gate.allow(t + Duration::from_secs(59)));
        assert!(gate.allow(t + Duration::from_secs(60)));
    }

    #[test]
    fn should_write_skips_equal_content() {
        let a = Clip::Text("a".to_string());
        assert!(!should_write(Some(&a), &a));
        assert!(should_write(None, &a));
        let text = Clip::Text("PNG".to_string());
        assert!(should_write(Some(&text), &Clip::Png(b"PNG".to_vec())));
    }
}
