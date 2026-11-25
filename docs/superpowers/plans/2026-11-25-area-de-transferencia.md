# Área de transferência junto com a borda — Plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ao cruzar a borda, o texto ou a imagem copiada vai, cifrada pela rede, para a área de transferência do PC vizinho, no macOS, no Windows e no GNOME.

**Architecture:** regras e formato do quadro sem sistema em `src/clip.rs`; porta `Clipboard` em `ports.rs` com um adapter por sistema (NSPasteboard, Win32, D-Bus para a extensão do GNOME); rede compartilhada em `src/adapters/net.rs` (envio de mão única e uma thread que escuta). Quem sai envia em paralelo com `Kvm::cross`; quem escuta é o app no macOS/Windows e o processo do ícone no Linux.

**Tech Stack:** Rust 2024, GPUI 0.2.2, `ring` 0.17 (ChaCha20-Poly1305), `base64` 0.22, `objc2-app-kit` 0.3 (macOS), `windows-sys` 0.61 (Windows), `zbus` 5 (Linux), `png` 0.18, GJS (extensão GNOME 45+).

**Spec:** `docs/superpowers/specs/2026-11-25-area-de-transferencia-design.md`

## Global Constraints

- Identificadores seguem o código vizinho (tipos e funções em inglês, como `Edges`, `Neighbor`, `cross`); comentários, textos, erros e `expect` em português; nomes de teste em inglês.
- Lints: nada de `.unwrap()`, `todo!`, `dbg!`; `#[allow]`/`#[expect]` com `reason`; todo `unsafe` com `// SAFETY:`.
- Erros com `anyhow` + `.context()`; mostrados com `format!("{error:#}")`.
- Campos novos de `Config` com `#[serde(default)]` e teste de arquivo antigo.
- Porta TCP **24811**. Limite **16 MiB** por conteúdo. Desafio de **16 bytes** por conexão. Conectar em **300 ms**; receber o desafio em **300 ms**; ler/escrever em **5 s**. Aviso de recusa no máximo **1 por minuto**.
- Chave: **32 bytes**, guardada em **base64** (44 caracteres) em `config.json`.
- Conexão: quem recebe manda primeiro o desafio (16 bytes de `SystemRandom`, novo por conexão); depois quem envia manda o quadro. Quadro: `"KVMC"` + versão `1` (u8) + nonce (12 bytes) + tamanho (u32 big-endian) + cifrado. Claro: tipo (u8: `1` texto UTF-8, `2` PNG) + dados. Dado associado: `"KVMC"` + versão + o desafio (21 bytes). Sem hora e sem memória de nonces: o desafio impede repetir um quadro.
- Só crates que já estão no `Cargo.lock`, nas versões dele. Depois de mexer no `Cargo.toml`, `cargo tree -d` não pode ganhar duplicata nova.
- A travessia nunca espera o envio e nunca toca o monitor.
- Verificação por sistema: macOS `cargo test && cargo clippy --all-targets -- -D warnings`; Windows `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` (como em `AGENTS.md`); Linux no Docker `rust:1` descrito em `AGENTS.md`, `cargo test` + o clippy.
- Itens novos que só a Task 7 passa a usar fora dos testes (Tasks 1–6) levam `#[expect(dead_code, reason = "usado a partir da ligação na Task 7")]`, para o clippy passar a cada commit; a Task 7 tira todos.

## Review Focus

1. **Vizinho fora do ar ou `.local` que não resolve:** teclado e mouse passam na hora, e `kvm-switch edge` sem janela sai em até ~5,6 s. Teste: `send_to_a_closed_port_fails_fast` (Task 3) e o prazo em `cross_without_window` (Task 7).
2. **Acentos, emoji e `\r\n`:** chegam byte a byte iguais. Testes: `seal_then_open_returns_the_same_text` (Task 2), `text_written_reads_back_identical` (Task 4).
3. **Chave colada de um chat, com espaço ou quebra de linha:** é aceita. Testes: `key_parse_trims_whitespace` (Task 2), `pasting_a_key_replaces_it` (Task 8).
4. **Lixo ou cabeçalho gigante na porta** (scanner da rede, outro programa): nada é gravado, nada é alocado além do limite, e a escuta continua atendendo. Teste: `an_oversized_header_is_dropped_and_the_listener_keeps_serving` (Task 3).
5. **Word/Excel com texto e imagem juntos:** vai o texto. Teste: `text_wins_over_an_image` (Task 4); no Windows e no GNOME, conferido no teste manual da Task 9.

---

### Task 1: Configuração: endereço do vizinho e modo área de transferência

**Files:**
- Modify: `src/domain.rs` (`Neighbor`, `Config`, `Default for Config`, testes do domínio)
- Modify: `src/config.rs` (`save`, testes)

**Interfaces:**
- Produces:
  - `Neighbor.address: String`: nome `.local` ou IP; vazio = aquele lado não envia.
  - `#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)] #[serde(default)] pub struct ClipboardMode { pub enabled: bool, pub key: String }`
  - `Config.clipboard: ClipboardMode`
  - `Config::clip_destination(&self, side: Side) -> Option<&str>`: endereço sem espaços nas pontas, só com `edges.channel(side).is_some()`, `clipboard.enabled` e endereço não vazio.
  - `Config::clip_key(&self) -> Option<&str>`: a chave sem espaços nas pontas, só com `edges.enabled`, `clipboard.enabled` e chave não vazia. É o "deve escutar?" do app e do ícone.

- [ ] **Step 1: Escrever os testes que falham**

Em `src/config.rs`:
```rust
#[test]
fn file_from_before_the_clipboard_keeps_it_off() {
    let config: Config = serde_json::from_str(
        r#"{"targets": [], "edges": {"enabled": true, "left": {"channel": 2, "name": "Linux"}}}"#,
    ).expect("arquivo antigo");
    assert_eq!(config.clipboard, ClipboardMode::default());
    assert_eq!(config.edges.left.address, "");
    assert_eq!(config.edges.left.channel, Some(Channel::Two));
}

#[cfg(unix)]
#[test]
fn saved_config_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt as _;
    let path = temp_file("private");
    save(&path, &Config::default()).expect("salvar");
    let mode = std::fs::metadata(&path).expect("metadados").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}
```
Em `saved_config_loads_back_identical`, preencher `edges.left.address = "pc.local"` e `clipboard = ClipboardMode { enabled: true, key: "k".into() }`.

Em `src/domain.rs`:
```rust
#[test]
fn clip_destination_needs_edges_clipboard_channel_and_address() {
    let mut config = Config::default();
    config.edges.left = Neighbor { channel: Some(Channel::Two), name: String::new(), address: " pc.local ".into() };
    assert_eq!(config.clip_destination(Side::Left), None); // bordas e área desligadas
    config.edges.enabled = true;
    assert_eq!(config.clip_destination(Side::Left), None); // área desligada
    config.clipboard.enabled = true;
    assert_eq!(config.clip_destination(Side::Left), Some("pc.local"));
    assert_eq!(config.clip_destination(Side::Right), None); // sem vizinho
    config.edges.left.channel = None;
    assert_eq!(config.clip_destination(Side::Left), None); // lado não atravessa
    config.edges.left = Neighbor { channel: Some(Channel::One), name: String::new(), address: "  ".into() };
    assert_eq!(config.clip_destination(Side::Left), None); // sem endereço
}

#[test]
fn clip_key_only_while_edges_and_clipboard_are_on() {
    let mut config = Config::default();
    config.clipboard.key = " abc\n".into();
    assert_eq!(config.clip_key(), None);
    config.clipboard.enabled = true;
    assert_eq!(config.clip_key(), None);
    config.edges.enabled = true;
    assert_eq!(config.clip_key(), Some("abc"));
    config.clipboard.key = String::new();
    assert_eq!(config.clip_key(), None);
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test clip_ file_from_before_the_clipboard saved_config`
Expected: erro de compilação (`address`, `clipboard`, `ClipboardMode` não existem).

- [ ] **Step 3: Implementar os campos e as duas funções em `src/domain.rs`; em `config::save`, criar o `.parcial` com permissão `0o600` no Unix (`OpenOptionsExt::mode`, `#[cfg(unix)]`) antes do `rename`**

Atualizar os construtores de `Neighbor` que já existem nos testes (`..Default::default()` onde couber).

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test`
Expected: PASS em tudo.

- [ ] **Step 5: Commit**

```bash
git add src/domain.rs src/config.rs
git commit -m "add the neighbor address and the clipboard mode to the config"
```

---

### Task 2: `src/clip.rs`: conteúdo, chave e quadro cifrado

**Files:**
- Create: `src/clip.rs`
- Modify: `src/main.rs` (`mod clip;`)
- Modify: `Cargo.toml` (`ring = "0.17"`, `base64 = "0.22"` em `[dependencies]`)

**Interfaces:**
- Produces (`src/clip.rs`):
```rust
pub const PORT: u16 = 24811;
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub const CHALLENGE_LEN: usize = 16;
pub const HEADER_LEN: usize = 21; // "KVMC" + versão + nonce + tamanho

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip { Text(String), Png(Vec<u8>) }

pub struct Key(/* ring::aead::LessSafeKey */);
impl Key {
    /// 32 bytes de `ring::rand::SystemRandom`, em base64 padrão (44 caracteres).
    pub fn generate() -> String;
    /// Aceita espaços e quebras de linha nas pontas; erro se não forem 32 bytes em base64.
    pub fn parse(text: &str) -> anyhow::Result<Key>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal { NotOurs, Version, TooBig, WrongKey, Kind }
impl Refusal {
    /// Só `WrongKey` merece aviso a quem está na frente do PC.
    pub fn notice(self) -> Option<&'static str>;
}

/// 16 bytes de `SystemRandom`, um por conexão recebida.
pub fn new_challenge() -> [u8; CHALLENGE_LEN];
/// Erro quando o conteúdo passa de `MAX_BYTES`.
pub fn seal(clip: &Clip, key: &Key, challenge: &[u8; CHALLENGE_LEN]) -> anyhow::Result<Vec<u8>>;
/// Confere "KVMC", versão 1 e o tamanho antes de alguém alocar o corpo.
pub fn body_len(header: &[u8; HEADER_LEN]) -> Result<usize, Refusal>;
pub fn open(header: &[u8; HEADER_LEN], body: Vec<u8>, key: &Key, challenge: &[u8; CHALLENGE_LEN]) -> Result<Clip, Refusal>;

#[derive(Default)] pub struct NoticeGate { /* último aviso */ }
impl NoticeGate { pub fn allow(&mut self, now: std::time::Instant) -> bool; } // 1 por 60 s

/// Grava só se o que chegou é diferente do que já está copiado.
pub fn should_write(current: Option<&Clip>, incoming: &Clip) -> bool;
```
- Ordem em `open`: decifrar (`WrongKey`, que também cobre quadro adulterado e quadro feito para outro desafio) → tipo desconhecido ou texto que não é UTF-8 (`Kind`).
- `body_len`: magia errada → `NotOurs`; versão ≠ 1 → `Version`; tamanho > `MAX_BYTES + 1 + 16` → `TooBig`.
- Texto de `notice`: `WrongKey` → "Chegou uma área de transferência com outra chave: confira a chave nos dois PCs".

- [ ] **Step 1: Escrever os testes que falham** (em `src/clip.rs`, `mod tests`; um helper de teste `frame(…)` separa cabeçalho e corpo do resultado de `seal`, e um `seal_kind(kind: u8, data, key, challenge)` privado monta quadros com tipo arbitrário)

```rust
seal_then_open_returns_the_same_text      // Clip::Text("ação 🚀\r\nfim") volta igual
seal_then_open_returns_the_same_png       // Clip::Png(vec![0x89, b'P', b'N', b'G', 1, 2, 3])
another_key_is_refused_as_wrong_key
a_flipped_byte_is_refused_as_wrong_key    // último byte do corpo trocado
a_frame_for_another_challenge_is_refused_as_wrong_key // seal com o desafio A, open com o desafio B → Err(WrongKey)
a_tampered_nonce_is_refused_as_wrong_key  // trocar um byte do nonce no cabeçalho → Err(WrongKey)
body_len_refuses_unknown_magic_version_and_oversize // "XXXX" → NotOurs; versão 2 → Version; MAX_BYTES + 17 → Ok; MAX_BYTES + 18 → TooBig
seal_refuses_a_clip_over_the_limit        // Clip::Png(vec![0; MAX_BYTES + 1]) → Err
an_unknown_kind_is_refused                // seal_kind(3, …) → Err(Kind); seal_kind(1, [0xff]) → Err(Kind)
generated_keys_parse                      // Key::parse(&Key::generate()).is_ok(); generate().len() == 44
key_parse_trims_whitespace                // Key::parse(&format!("  {k}\n")).is_ok()
key_parse_refuses_wrong_length_and_garbage // base64 de 16 bytes → Err; "não é base64" → Err; "" → Err
only_wrong_key_has_a_notice
notice_gate_allows_one_per_minute         // t: true; t+59s: false; t+60s: true
should_write_skips_equal_content          // (Some(Text("a")), Text("a")) → false; (None, Text("a")) → true;
                                          // (Some(Text("PNG")), Png(b"PNG")) → true
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test clip::`
Expected: erro de compilação (o módulo está vazio).

- [ ] **Step 3: Implementar `src/clip.rs`** com `ring::aead` (`CHACHA20_POLY1305`, `LessSafeKey`, `Nonce::assume_unique_for_key`, `Aad::from`, `seal_in_place_append_tag`/`open_in_place`), nonce de `SystemRandom::fill`, base64 de `base64::engine::general_purpose::STANDARD`.

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test clip:: && cargo clippy --all-targets -- -D warnings && cargo tree -d`
Expected: PASS; clippy limpo; `cargo tree -d` sem duplicata nova de `ring` ou `base64`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/clip.rs
git commit -m "add the clipboard frame: content, key, sealing and refusals"
```

---

### Task 3: Porta `Clipboard` e a rede (`src/adapters/net.rs`)

**Files:**
- Modify: `src/ports.rs` (trait `Clipboard`)
- Create: `src/adapters/net.rs`
- Modify: `src/adapters.rs` (`mod net;` e `pub use net::{ClipListener, listen, share, SEND_DEADLINE};`)

**Interfaces:**
- Consumes: `Clip`, `Key`, `new_challenge`, `seal`, `body_len`, `open`, `NoticeGate`, `should_write`, `PORT`, `HEADER_LEN`, `CHALLENGE_LEN` (Task 2).
- Produces:
```rust
// ports.rs
/// A área de transferência deste PC. Chamada fora da thread do GPUI.
pub trait Clipboard: Send + Sync {
    /// Texto, se houver; senão imagem PNG; `None` sem nenhum dos dois.
    fn read(&self) -> Result<Option<Clip>>;
    /// Ler logo depois devolve os mesmos bytes.
    fn write(&self, clip: &Clip) -> Result<()>;
}

// adapters/net.rs
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// O máximo que alguém espera um envio: conectar + desafio + escrever.
pub const SEND_DEADLINE: Duration = Duration::from_millis(5_600);
/// `address` é "host" (usa `PORT`) ou "host:porta". Uma conexão: lê o desafio, cifra amarrado a ele e escreve o quadro.
pub fn send(address: &str, key: &Key, clip: &Clip) -> Result<()>;
/// Lê a área de transferência e envia; sem texto nem imagem, não faz nada.
pub fn share(clipboard: &dyn Clipboard, address: &str, key: &Key) -> Result<()>;
/// Escuta até ser descartado. `notify` recebe os avisos de recusa, já limitados pelo `NoticeGate`.
pub fn listen(bind: SocketAddr, key: Key, clipboard: Arc<dyn Clipboard>, notify: Box<dyn Fn(String) + Send>) -> Result<ClipListener>;
pub struct ClipListener { /* thread + parada */ }
impl ClipListener { pub fn local_addr(&self) -> SocketAddr; }
impl Drop for ClipListener { /* para a thread e solta a porta antes de voltar */ }
```
- Escuta: uma conexão por vez, numa thread. Ao aceitar, manda `new_challenge()`; depois, com prazo `IO_TIMEOUT` para ler: cabeçalho → `body_len` → corpo → `open` com aquele desafio. `Ok(clip)` e `should_write(clipboard.read().ok().flatten().as_ref(), &clip)` → `write` (erro vai para o stderr). `Err(refusal)` com `notice()` e `NoticeGate::allow` → `notify`. Qualquer erro de uma conexão nunca derruba a escuta.
- `send`: resolve com `ToSocketAddrs`, tenta cada endereço com `connect_timeout(CONNECT_TIMEOUT)`, lê o desafio com `set_read_timeout(CONNECT_TIMEOUT)`, escreve com `set_write_timeout(IO_TIMEOUT)`.

- [ ] **Step 1: Escrever os testes que falham** (em `net.rs`; `FakeClipboard { current: Mutex<Option<Clip>>, writes: Mutex<usize> }`; avisos num `Arc<Mutex<Vec<String>>>`; tudo em `127.0.0.1:0`; um helper espera até 2 s por uma condição)

```rust
text_crosses_to_the_listener_clipboard        // share(Text("ação")) → current == Some(Text("ação")), writes == 1
png_crosses_to_the_listener_clipboard
a_wrong_key_writes_nothing_and_notices_once   // dois envios com outra chave → writes == 0, avisos == 1
a_replayed_frame_writes_nothing              // TcpStream lê o desafio, manda um quadro cifrado para outro desafio → writes == 0
equal_content_is_not_written_again            // current já é Text("a"); envia Text("a") → writes == 0
an_oversized_header_is_dropped_and_the_listener_keeps_serving
    // TcpStream manda "KVMC", 1, 12 zeros, u32::MAX e fecha; depois share(Text("ok")) → current == Some(Text("ok"))
send_to_a_closed_port_fails_fast              // porta de um listener já descartado → Err em menos de 1 s
dropping_the_listener_frees_the_port          // listen, drop, listen de novo no mesmo endereço → Ok
nothing_to_share_sends_nothing                // FakeClipboard com None → share Ok, listener writes == 0
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test adapters::net`
Expected: erro de compilação.

- [ ] **Step 3: Implementar o trait em `ports.rs` e `send`, `share`, `listen`, `ClipListener` em `net.rs`**

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test adapters::net && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy limpo.

- [ ] **Step 5: Commit**

```bash
git add src/ports.rs src/adapters.rs src/adapters/net.rs
git commit -m "add the clipboard port and the one-way encrypted transfer"
```

---

### Task 4: Adapter do macOS (NSPasteboard)

**Files:**
- Create: `src/adapters/macos/clipboard.rs`
- Modify: `src/adapters/macos.rs` (`mod clipboard; pub use clipboard::MacClipboard;`)
- Modify: `Cargo.toml` (`[target.'cfg(target_os = "macos")'.dependencies]`: `objc2 = "0.6"`, `objc2-foundation = "0.3"` e `objc2-app-kit = "0.3"`, com `default-features = false` e só as features de que o código precisar, como `NSPasteboard`, `NSImageRep`, `NSBitmapImageRep`, `NSString`, `NSData`)

**Interfaces:**
- Consumes: `Clipboard`, `Clip` (Tasks 2–3).
- Produces: `pub struct MacClipboard { name: Option<String> }`, com `MacClipboard::general()` (área geral) e, só em teste, `MacClipboard::private()` (`pasteboardWithUniqueName`, solto com `releaseGlobally` no `Drop`). O pasteboard é obtido a cada chamada, então o tipo é `Send + Sync` sem `unsafe impl`.
- Leitura: `NSPasteboardTypeString` → `Text`; senão `NSPasteboardTypePNG` → `Png` (bytes como estão); senão `NSPasteboardTypeTIFF` → `NSBitmapImageRep` → PNG; senão `None`.
- Gravação: `clearContents`, depois `setString` (texto) ou `setData` em `NSPasteboardTypePNG` (imagem).

- [ ] **Step 1: Escrever os testes que falham** (`#[cfg(test)]` no próprio arquivo; sempre em `MacClipboard::private()`, nunca na área do usuário; o PNG de 1×1 é gerado no teste com o crate `png`)

```rust
text_written_reads_back_identical   // "ação 🚀\r\n"
png_written_reads_back_identical    // os mesmos bytes
text_wins_over_an_image             // string e PNG postos direto no pasteboard → Text
tiff_only_reads_as_png              // TIFF (TIFFRepresentation de um NSBitmapImageRep do PNG) → Png que decodifica em 1×1
empty_board_reads_none
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test adapters::macos::clipboard`
Expected: erro de compilação.

- [ ] **Step 3: Implementar `MacClipboard`**

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo tree -d`
Expected: PASS; clippy limpo; sem duplicata nova de `objc2*`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/adapters/macos.rs src/adapters/macos/clipboard.rs
git commit -m "read and write the macOS pasteboard for the clipboard sharing"
```

---

### Task 5: Adapter do Windows e a conversão DIB ↔ PNG

**Files:**
- Create: `src/adapters/dib.rs` (`#[cfg(any(target_os = "windows", test))]` em `adapters.rs`, como o `Rect` do domínio, para testar no Mac)
- Create: `src/adapters/windows/clipboard.rs`
- Modify: `src/adapters/windows.rs` (`mod clipboard; pub use clipboard::WindowsClipboard;`)
- Modify: `Cargo.toml` (features do `windows-sys`: `Win32_System_DataExchange`, `Win32_System_Memory`, `Win32_System_Ole`)

**Interfaces:**
- Consumes: `Clipboard`, `Clip`.
- Produces:
```rust
// dib.rs
/// Conteúdo de CF_DIB ou CF_DIBV5: cabeçalho de 40 ou 124 bytes, 24 ou 32 bits,
/// BI_RGB ou BI_BITFIELDS com as máscaras padrão, de baixo para cima ou de cima
/// para baixo. Outro formato ou dado cortado: `None`.
pub fn dib_to_png(dib: &[u8]) -> Option<Vec<u8>>;
/// BITMAPINFOHEADER de 32 bits, BI_RGB, de baixo para cima, BGRA.
pub fn png_to_dib(png: &[u8]) -> Option<Vec<u8>>;

// windows/clipboard.rs
pub struct WindowsClipboard;
```
- Alfa: em 32 bits BI_RGB com todo alfa zero, a imagem é opaca.
- Leitura: `OpenClipboard(null)`, com até 10 tentativas a cada 10 ms (outro programa pode estar com ela aberta); `CF_UNICODETEXT` → `Text` (sem o NUL final, `\r\n` como está); senão o formato registrado `"PNG"` → `Png` (bytes como estão, o que garante "gravar e ler devolve os mesmos bytes"); senão `CF_DIBV5`/`CF_DIB` → `dib_to_png`; senão `None`.
- Gravação: `EmptyClipboard`; texto em `CF_UNICODETEXT` (UTF-16 + NUL, `GlobalAlloc(GMEM_MOVEABLE)`); imagem em `"PNG"` e em `CF_DIB` (`png_to_dib`). Se `SetClipboardData` recusar sem janela dona, abrir com uma janela de mensagens oculta (`HWND_MESSAGE`).

- [ ] **Step 1: Escrever os testes de `dib.rs` que falham** (DIBs montados à mão no teste, seguindo o layout do BITMAPINFOHEADER; pixels conferidos decodificando o PNG com o crate `png`)

```rust
a_bottom_up_24_bit_dib_becomes_the_same_pixels // 2×2 com preenchimento de linha; ordem das linhas invertida
a_top_down_32_bit_dib_keeps_row_order          // altura negativa
a_32_bit_dib_with_zero_alpha_is_opaque
a_bitfields_v5_dib_becomes_the_same_pixels     // cabeçalho de 124 bytes, BI_BITFIELDS
png_to_dib_then_back_keeps_the_pixels
unsupported_dibs_are_none                      // 8 bits; dado cortado no meio
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test adapters::dib`
Expected: erro de compilação.

- [ ] **Step 3: Implementar `dib.rs`**

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test adapters::dib`
Expected: PASS.

- [ ] **Step 5: Implementar `WindowsClipboard`** e conferir: `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` limpo.

- [ ] **Step 6: Num PC Windows, capturar um print real e virar fixture**

Win+Shift+S num trecho pequeno da tela, depois no PowerShell:
```powershell
Add-Type -AssemblyName System.Windows.Forms
$dib = [Windows.Forms.Clipboard]::GetData('DeviceIndependentBitmap')
[IO.File]::WriteAllBytes("$PWD\src\adapters\fixtures\recorte-win11.dib", $dib.ToArray())
```
Adicionar `a_real_snipping_tool_capture_becomes_png` (fixture com `include_bytes!`, origem anotada: Windows 11, Ferramenta de Captura, data). Ele confere que `dib_to_png` devolve `Some` e que o PNG decodifica com a largura e a altura do cabeçalho. Rodar `cargo test` no Windows.
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/adapters.rs src/adapters/dib.rs src/adapters/windows.rs src/adapters/windows/clipboard.rs src/adapters/fixtures/recorte-win11.dib
git commit -m "read and write the Windows clipboard, converting DIB images to PNG"
```

---

### Task 6: Linux: métodos D-Bus na extensão e o adapter `GnomeClipboard`

**Files:**
- Modify: `linux/gnome-extension/kvm-switch@mpgxc.github.io/extension.js`
- Create: `src/adapters/linux/clipboard.rs`
- Modify: `src/adapters/linux.rs` (`mod clipboard; pub use clipboard::GnomeClipboard;`)
- Modify: `Cargo.toml` (`[target.'cfg(target_os = "linux")'.dependencies]`: `zbus = { version = "5", default-features = false, features = [...] }`, com o runtime que o `cargo tree -e features -i zbus` mostra já ativo no Linux, mais `blocking-api`)

**Interfaces:**
- Consumes: `Clipboard`, `Clip`.
- Produces:
  - D-Bus: objeto `/io/github/mpgxc/KvmSwitch`, interface `io.github.mpgxc.KvmSwitch`, na conexão de sessão do gnome-shell (nome `org.gnome.Shell`):
    - `Ler() → (s tipo, ay dados)`: tipo `"texto"` (UTF-8), `"png"` ou `""`. Texto preferido quando há os dois.
    - `Gravar(s tipo, ay dados)`
    - `Avisar(s texto)`: `Main.notify('KVM Switch', texto)`.
  - Rust: `pub struct GnomeClipboard;` com `impl Clipboard` e `pub fn notify(&self, text: &str) -> Result<()>`.
  - Funções puras, testáveis fora do GNOME: `fn clip_from(kind: &str, data: Vec<u8>) -> Result<Option<Clip>>` e `fn wire(clip: &Clip) -> (&'static str, &[u8])`.
- Extensão: `Gio.DBusExportedObject.wrapJSObject(XML, objeto)`, `export(Gio.DBus.session, caminho)` no `enable()` e `unexport()` no `disable()`. `LerAsync(params, invocation)`: `St.Clipboard.get_default()`; `get_mimetypes(St.ClipboardType.CLIPBOARD)`; com um tipo de texto (`text/plain;charset=utf-8`, `UTF8_STRING` ou `text/plain`) usa `get_text`; senão, com `image/png`, `get_content`; responde `new GLib.Variant('(say)', [...])`. `Gravar` usa `set_text` ou `set_content(…, 'image/png', GLib.Bytes)`.

- [ ] **Step 1: Escrever os testes que falham** (em `clipboard.rs`)

```rust
text_kind_becomes_text            // clip_from("texto", "ação".into()) → Some(Text("ação"))
png_kind_becomes_png
empty_kind_is_nothing             // clip_from("", vec![]) → None
unknown_kind_or_bad_utf8_is_an_error
wire_names_match_the_extension    // wire(&Text("a")).0 == "texto"; wire(&Png(..)).0 == "png"
```

- [ ] **Step 2: Rodar e ver falhar no Docker do Linux**

Run: `cargo test adapters::linux::clipboard`
Expected: erro de compilação.

- [ ] **Step 3: Implementar `GnomeClipboard`** (`zbus::blocking::Connection::session()` guardada num `OnceLock`, `call_method` com destino `org.gnome.Shell`) e os três métodos na extensão

- [ ] **Step 4: Rodar e ver passar no Docker**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo tree -d`
Expected: PASS; clippy limpo; sem duplicata nova de `zbus`.

- [ ] **Step 5: Conferir a extensão num GNOME real**

Reinstalar a extensão (`linux/install.sh --app`), sair e entrar na sessão, copiar "teste" e rodar:
`gdbus call --session --dest org.gnome.Shell --object-path /io/github/mpgxc/KvmSwitch --method io.github.mpgxc.KvmSwitch.Ler`
Expected: `('texto', [byte 0x74, …])`. Depois `…Gravar "texto" "[0x6f, 0x6b]"` e Ctrl+V num editor cola "ok"; `…Avisar "oi"` mostra a notificação.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock linux/gnome-extension src/adapters/linux.rs src/adapters/linux/clipboard.rs
git commit -m "reach the GNOME clipboard through D-Bus methods on the extension"
```

---

### Task 7: Ligar tudo: envio na travessia, escuta e avisos

**Files:**
- Modify: `src/adapters.rs` (`Platform.clipboard`, `pub fn clipboard() -> Arc<dyn Clipboard>` por sistema)
- Modify: `src/app.rs` (`AppState`)
- Modify: `src/main.rs` (`start`, `cross_without_window`)
- Modify: `src/adapters/linux/tray.rs` (escuta no Linux)
- Modify: `src/ui/settings.rs` (só os testes: `Platform` e `AppState::new` novos; `NoHardware` ganha `impl Clipboard` com `Ok(None)`/`Ok(())`)
- Modify: `src/domain.rs`, `src/clip.rs`, `src/adapters/net.rs` e os adapters das Tasks 4–6 (tirar os `#[expect(dead_code)]`)

**Interfaces:**
- Consumes: `Config::clip_destination`, `Config::clip_key` (Task 1); `Key::parse`, `PORT` (Task 2); `share`, `listen`, `ClipListener`, `SEND_DEADLINE` (Task 3); `MacClipboard`, `WindowsClipboard`, `GnomeClipboard` (Tasks 4–6).
- Produces:
  - `Platform.clipboard: Arc<dyn Clipboard>`
  - `AppState::new(platform: Platform, config_path: PathBuf, notices: async_channel::Sender<String>) -> Self`
  - `AppState::show_hud` passa a `pub(crate)`.
- Regras:
  - **Escuta no macOS/Windows (`AppState::refresh_clipboard`, chamado em `new` e `save`):** com `clip_key()` e `Key::parse` ok, escuta em `0.0.0.0:PORT`. Guarda `Option<(String, ClipListener)>` com a chave usada e só reinicia se a chave mudou. Desligado ou chave inválida: descarta o listener. Erro ao escutar (porta ocupada) vai para `status`. Os avisos chegam por `notices`, e `main::start` roda um laço `cx.spawn` que chama `show_hud`. No Linux, `refresh_clipboard` não faz nada (`cfg`).
  - **Escuta no Linux (`tray.rs`):** a mesma regra, no início e a cada mudança do `config.json`; `notify` chama `GnomeClipboard::notify`.
  - **Envio em `AppState::cross`:** depois das guardas que já existem, junto com o spawn de `kvm.cross`, quando houver `clip_destination(side)` e chave válida, roda numa `std::thread` com `InFlight` (no Linux o GPUI não pode sair no meio): `share(&*clipboard, &endereço, &key)`. Erro vai para o stderr, sem HUD.
  - **Envio em `cross_without_window`:** a mesma regra. O envio roda numa thread iniciada antes de `Kvm::cross`; depois da troca, espera o fim do envio com `mpsc::Receiver::recv_timeout(SEND_DEADLINE)` e sai.

- [ ] **Step 1: Implementar a ligação** (`app.rs` e `main.rs` não têm testes, como diz o `AGENTS.md`; a cobertura é o compilador, o clippy nos três sistemas e o teste manual da Task 9)

- [ ] **Step 2: Rodar a verificação nos três sistemas**

Run (macOS): `cargo test && cargo clippy --all-targets -- -D warnings`
Run: `cargo xwin clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings`
Run (Docker Linux): `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS e clippy limpo nos três.

- [ ] **Step 3: Fumaça no Mac**

Pôr no `config.json` Bordas e a área de transferência ligadas, com uma chave de `Key::generate` (a janela só vem na Task 8). Abrir o app e conferir com `lsof -iTCP:24811 -sTCP:LISTEN`; depois desligar o modo no arquivo e conferir de novo. A troca de verdade entre dois PCs fica para a Task 9.
Expected: a porta aparece em LISTEN só com o modo ligado e some ao desligar.

- [ ] **Step 4: Commit**

```bash
git add src/
git commit -m "send the clipboard on every edge crossing and listen for the neighbors"
```

---

### Task 8: Janela de configuração

**Files:**
- Modify: `src/ui/settings.rs`

**Interfaces:**
- Consumes: `ClipboardMode`, `Neighbor.address` (Task 1); `Key::generate`, `Key::parse` (Task 2); `AppState::refresh_clipboard` via `save` (Task 7).
- Produces (seletores de teste): `"ClipboardMode"`, `"EdgeAddress(Left)"` (e os outros lados), `"ClipboardKey"`, `"CopyKey"`.
- Comportamento:
  - Em "Modos", o terceiro `toggle("ClipboardMode", "Área de transferência (junto com as bordas)", …)`. Ao ligar com a chave vazia, grava `Key::generate()`. Fica com `INACTIVE_OPACITY` com Bordas desligado.
  - `Field::EdgeAddress(Side)` abaixo do nome em `edge_cell`, com o placeholder "endereço (pc.local ou IP)", editado como o nome.
  - `Field::ClipboardKey`, editado como o nome. Nele, ⌘V (macOS) ou Ctrl+V (outros) troca o texto pelo da área de transferência (`cx.read_from_clipboard`), sem espaços nas pontas.
  - Botão "Copiar" (`"CopyKey"`) → `cx.write_to_clipboard(ClipboardItem::new_string(chave))`.
  - Com o modo ligado e `Key::parse` falhando, a linha de erro "Chave inválida: copie a chave de outro PC com o botão Copiar" (`theme::ERROR`).

- [ ] **Step 1: Escrever os testes que falham** (`#[gpui::test]`, com o `open()` que já existe)

```rust
turning_the_clipboard_on_creates_a_key_once
    // clica ClipboardMode → Key::parse(&config.clipboard.key) ok; desliga e liga de novo → a mesma chave
typing_edits_the_neighbor_address_and_saves_it
    // clica EdgeAddress(Left), "p c 1 enter" → edges.left.address == "pc1", também no arquivo salvo
pasting_a_key_replaces_it
    // cx.write_to_clipboard(new_string(format!("  {k}\n"))); clica ClipboardKey; "cmd-v" (macOS) ou "ctrl-v" → key == k
copy_key_puts_it_on_the_clipboard
    // clica CopyKey → read_from_clipboard().text() == Some(chave)
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cargo test ui::settings`
Expected: FAIL (`campo ClipboardMode não foi desenhado`).

- [ ] **Step 3: Implementar os campos, o interruptor e o botão**

- [ ] **Step 4: Rodar e ver passar**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: PASS; clippy limpo. Conferir com `cargo run --release` que o quadro 3×3 cabe na janela com o endereço.

- [ ] **Step 5: Commit**

```bash
git add src/ui/settings.rs
git commit -m "configure the clipboard sharing, neighbor addresses and the key in the window"
```

---

### Task 9: Documentação e teste no hardware

**Files:**
- Modify: `README.md` (tabela de arquivos: `src/clip.rs`, `src/adapters/net.rs`, `src/adapters/dib.rs`, `src/adapters/macos/clipboard.rs`, `src/adapters/windows/clipboard.rs`, `src/adapters/linux/clipboard.rs`; uso: ligar o modo, copiar a chave, endereço `.local`; limites: firewall)
- Modify: `AGENTS.md` (arquitetura: porta `Clipboard`, quem escuta em cada sistema, métodos D-Bus da extensão; invariantes: a travessia nunca espera o envio; quem recebe só grava conteúdo diferente; quem envia não guarda estado)

- [ ] **Step 1: Escrever no README**
  - Windows: a primeira escuta abre o pedido do firewall; aceitar em "Redes privadas".
  - macOS: o firewall, se ligado, pergunta na primeira escuta.
  - Ubuntu com `ufw` ligado: `sudo ufw allow 24811/tcp`.
  - Quem recebe manda um desafio a cada conexão: um quadro capturado na rede não serve para gravar de novo.
  - Senhas copiadas também vão para o vizinho, cifradas, e podem ficar no histórico da área de transferência dele.

- [ ] **Step 2: Atualizar o `AGENTS.md`**

- [ ] **Step 3: Teste manual nos três PCs** (a mesma chave nos três, endereços `.local`), nos dois sentidos de macOS ↔ Windows, macOS ↔ Linux e Windows ↔ Linux:
  - texto com acentos e emoji colado igual;
  - print (⌘⇧⌃4, Win+Shift+S, PrtSc do GNOME) e imagem copiada do navegador, colados no Preview, no Paint e no GIMP ou LibreOffice;
  - célula do Excel/Word chega como texto;
  - ir e voltar: a cópia original do PC de origem continua (cole formatado no Pages ou no Word depois de voltar);
  - vizinho desligado: teclado e mouse passam sem atraso perceptível;
  - chave diferente num PC: o aviso aparece no PC de destino, uma vez.
Expected: todos passam. Falha vira correção na task dona antes do commit.

- [ ] **Step 4: Commit**

```bash
git add README.md AGENTS.md
git commit -m "document the clipboard sharing"
```
