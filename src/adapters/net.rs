//! A rede da área de transferência: um envio por travessia e a escuta do vizinho.
//!
//! Só `std::net`, igual em todo sistema. Quem recebe manda um desafio novo a cada
//! conexão, lê um único quadro amarrado a ele e fecha a conexão.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "usado a partir da ligação na Task 7")
)]

use std::io::{self, Read, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};

use crate::clip::{
    CHALLENGE_LEN, Clip, HEADER_LEN, Key, NoticeGate, PORT, Refusal, body_len, new_challenge, open,
    seal, should_write,
};
use crate::ports::Clipboard;

pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// O máximo que alguém espera um envio: conectar + desafio + escrever.
#[expect(dead_code, reason = "sem uso nem nos testes até a ligação na Task 7")]
pub const SEND_DEADLINE: Duration = Duration::from_millis(5_600);
/// De quanto em quanto a escuta parada confere se foi descartada.
const STOP_POLL: Duration = Duration::from_millis(50);

/// `address` é "host" (usa `PORT`) ou "host:porta". Uma conexão: lê o desafio,
/// cifra amarrado a ele e escreve o quadro.
pub fn send(address: &str, key: &Key, clip: &Clip) -> Result<()> {
    // Um IPv6 sem colchetes ("fe80::1") não traz porta.
    let has_port = address.parse::<SocketAddr>().is_ok()
        || address
            .rsplit_once(':')
            .is_some_and(|(host, port)| !host.contains(':') && port.parse::<u16>().is_ok());
    let sockets = if has_port {
        address.to_socket_addrs()
    } else {
        (address, PORT).to_socket_addrs()
    }
    .with_context(|| format!("não encontrei o endereço de {address}"))?;

    let mut stream = Err(anyhow!("{address} não tem endereço"));
    for socket in sockets {
        stream = TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT)
            .with_context(|| format!("não consegui conectar a {address}"));
        if stream.is_ok() {
            break;
        }
    }
    let mut stream = stream?;

    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    let mut challenge = [0u8; CHALLENGE_LEN];
    stream
        .read_exact(&mut challenge)
        .with_context(|| format!("{address} não mandou o desafio"))?;
    let frame = seal(clip, key, &challenge)?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream
        .write_all(&frame)
        .with_context(|| format!("não consegui enviar a área de transferência a {address}"))
}

/// Lê a área de transferência e envia; sem texto nem imagem, não faz nada.
pub fn share(clipboard: &dyn Clipboard, address: &str, key: &Key) -> Result<()> {
    match clipboard
        .read()
        .context("não consegui ler a área de transferência")?
    {
        Some(clip) => send(address, key, &clip),
        None => Ok(()),
    }
}

/// Escuta até ser descartado. `notify` recebe os avisos de recusa, já limitados
/// pelo `NoticeGate`.
pub fn listen(
    bind: SocketAddr,
    key: Key,
    clipboard: Arc<dyn Clipboard>,
    notify: Box<dyn Fn(String) + Send>,
) -> Result<ClipListener> {
    let listener =
        TcpListener::bind(bind).with_context(|| format!("não consegui ouvir em {bind}"))?;
    let address = listener.local_addr()?;
    // Sem bloquear no `accept`, a thread vê o pedido de parada a cada `STOP_POLL`.
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread = thread::spawn({
        let stop = stop.clone();
        move || {
            let mut gate = NoticeGate::default();
            while !stop.load(Ordering::Relaxed) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) => {
                        if error.kind() != io::ErrorKind::WouldBlock {
                            eprintln!("kvm-switch: área de transferência: {error}");
                        }
                        thread::sleep(STOP_POLL);
                        continue;
                    }
                };
                match receive(stream, &key) {
                    Ok(Ok(clip)) => {
                        if should_write(clipboard.read().ok().flatten().as_ref(), &clip)
                            && let Err(error) = clipboard.write(&clip)
                        {
                            eprintln!("kvm-switch: {error:#}");
                        }
                    }
                    Ok(Err(refusal)) => {
                        if let Some(notice) = refusal.notice()
                            && gate.allow(Instant::now())
                        {
                            notify(notice.to_string());
                        }
                    }
                    // Conexão que caiu ou estourou o prazo: só ela se perde.
                    Err(_) => {}
                }
            }
        }
    });
    Ok(ClipListener {
        address,
        stop,
        thread: Some(thread),
    })
}

/// Manda o desafio e lê um quadro, tudo dentro de `IO_TIMEOUT`.
fn receive(mut stream: TcpStream, key: &Key) -> io::Result<Result<Clip, Refusal>> {
    // A conexão aceita pode herdar o modo sem bloqueio do listener (macOS, Windows).
    stream.set_nonblocking(false)?;
    let challenge = new_challenge();
    stream.write_all(&challenge)?;
    let mut stream = Deadline {
        stream,
        until: Instant::now() + IO_TIMEOUT,
    };
    let mut header = [0u8; HEADER_LEN];
    stream.read_exact(&mut header)?;
    let len = match body_len(&header) {
        Ok(len) => len,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    Ok(open(&header, body, key, &challenge))
}

/// Leitura com prazo total, e não por chamada: quem manda byte a byte não prende a
/// escuta (nem o `drop` dela) além de `IO_TIMEOUT`.
struct Deadline {
    stream: TcpStream,
    until: Instant,
}

impl Read for Deadline {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(buffer)
    }
}

/// A escuta em andamento; descartá-la para a thread e solta a porta.
pub struct ClipListener {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ClipListener {
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for ClipListener {
    /// Espera a thread, dona do listener, terminar: na volta, a porta está livre.
    /// Leva até `STOP_POLL`, ou `IO_TIMEOUT` com uma conexão em curso.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // Um pânico da thread já saiu no stderr; não há o que fazer com ele aqui.
            thread.join().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Área de transferência em memória que conta as gravações.
    struct FakeClipboard {
        current: Mutex<Option<Clip>>,
        writes: Mutex<usize>,
    }

    impl FakeClipboard {
        fn with(clip: Option<Clip>) -> Arc<FakeClipboard> {
            Arc::new(FakeClipboard {
                current: Mutex::new(clip),
                writes: Mutex::new(0),
            })
        }

        fn current(&self) -> Option<Clip> {
            self.current
                .lock()
                .expect("lock da área falsa envenenado")
                .clone()
        }

        fn writes(&self) -> usize {
            *self.writes.lock().expect("lock das gravações envenenado")
        }
    }

    impl Clipboard for FakeClipboard {
        fn read(&self) -> Result<Option<Clip>> {
            Ok(self.current())
        }

        fn write(&self, clip: &Clip) -> Result<()> {
            *self.current.lock().expect("lock da área falsa envenenado") = Some(clip.clone());
            *self.writes.lock().expect("lock das gravações envenenado") += 1;
            Ok(())
        }
    }

    /// Um PC que ouve em `127.0.0.1:0`, com a área falsa e os avisos recebidos.
    struct Receiver {
        clipboard: Arc<FakeClipboard>,
        notices: Arc<Mutex<Vec<String>>>,
        listener: ClipListener,
        key: String,
    }

    impl Receiver {
        fn new(current: Option<Clip>) -> Receiver {
            let key = Key::generate();
            let clipboard = FakeClipboard::with(current);
            let notices = Arc::new(Mutex::new(Vec::new()));
            let listener = listen(
                ([127, 0, 0, 1], 0).into(),
                Key::parse(&key).expect("chave gerada inválida"),
                clipboard.clone(),
                Box::new({
                    let notices = notices.clone();
                    move |notice| {
                        notices
                            .lock()
                            .expect("lock dos avisos envenenado")
                            .push(notice)
                    }
                }),
            )
            .expect("não consegui ouvir");
            Receiver {
                clipboard,
                notices,
                listener,
                key,
            }
        }

        fn address(&self) -> String {
            self.listener.local_addr().to_string()
        }

        fn key(&self) -> Key {
            Key::parse(&self.key).expect("chave gerada inválida")
        }

        fn notices(&self) -> usize {
            self.notices
                .lock()
                .expect("lock dos avisos envenenado")
                .len()
        }

        /// Volta quando a escuta já atendeu tudo o que chegou antes: ela só manda o
        /// desafio de uma conexão depois de terminar a anterior.
        fn settle(&self) {
            let mut stream =
                TcpStream::connect(self.listener.local_addr()).expect("não consegui conectar");
            stream
                .read_exact(&mut [0; CHALLENGE_LEN])
                .expect("a escuta não mandou o desafio");
        }

        /// Espera até 2 s pelo conteúdo, conferindo a cada 10 ms.
        fn wait_for(&self, clip: &Clip) -> bool {
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                if self.clipboard.current().as_ref() == Some(clip) {
                    return true;
                }
                thread::sleep(Duration::from_millis(10));
            }
            false
        }
    }

    fn share_from(clip: Option<Clip>, receiver: &Receiver, key: &Key) {
        share(&*FakeClipboard::with(clip), &receiver.address(), key).expect("o envio falhou");
    }

    #[test]
    fn text_crosses_to_the_listener_clipboard() {
        let receiver = Receiver::new(None);
        let clip = Clip::Text("ação".into());
        share_from(Some(clip.clone()), &receiver, &receiver.key());
        assert!(receiver.wait_for(&clip));
        assert_eq!(receiver.clipboard.writes(), 1);
    }

    #[test]
    fn png_crosses_to_the_listener_clipboard() {
        let receiver = Receiver::new(Some(Clip::Text("antes".into())));
        let clip = Clip::Png(vec![0x89, b'P', b'N', b'G', 1, 2, 3]);
        share_from(Some(clip.clone()), &receiver, &receiver.key());
        assert!(receiver.wait_for(&clip));
        assert_eq!(receiver.clipboard.writes(), 1);
    }

    #[test]
    fn a_wrong_key_writes_nothing_and_notices_once() {
        let receiver = Receiver::new(None);
        let other = Key::parse(&Key::generate()).expect("chave gerada inválida");
        share_from(Some(Clip::Text("x".into())), &receiver, &other);
        share_from(Some(Clip::Text("x".into())), &receiver, &other);
        receiver.settle();
        assert_eq!(receiver.clipboard.writes(), 0);
        assert_eq!(receiver.notices(), 1);
    }

    #[test]
    fn a_replayed_frame_writes_nothing() {
        let receiver = Receiver::new(None);
        let mut stream =
            TcpStream::connect(receiver.listener.local_addr()).expect("não consegui conectar");
        stream
            .read_exact(&mut [0; CHALLENGE_LEN])
            .expect("a escuta não mandou o desafio");
        let frame = seal(&Clip::Text("x".into()), &receiver.key(), &new_challenge())
            .expect("não consegui cifrar");
        stream.write_all(&frame).expect("não consegui escrever");
        drop(stream);
        receiver.settle();
        assert_eq!(receiver.clipboard.writes(), 0);
    }

    #[test]
    fn equal_content_is_not_written_again() {
        let receiver = Receiver::new(Some(Clip::Text("a".into())));
        share_from(Some(Clip::Text("a".into())), &receiver, &receiver.key());
        receiver.settle();
        assert_eq!(receiver.clipboard.writes(), 0);
    }

    #[test]
    fn an_oversized_header_is_dropped_and_the_listener_keeps_serving() {
        let receiver = Receiver::new(None);
        let mut stream =
            TcpStream::connect(receiver.listener.local_addr()).expect("não consegui conectar");
        let mut header = b"KVMC\x01".to_vec();
        header.extend_from_slice(&[0; 12]);
        header.extend_from_slice(&u32::MAX.to_be_bytes());
        stream.write_all(&header).expect("não consegui escrever");
        drop(stream);
        let clip = Clip::Text("ok".into());
        share_from(Some(clip.clone()), &receiver, &receiver.key());
        assert!(receiver.wait_for(&clip));
    }

    #[test]
    fn send_to_a_closed_port_fails_fast() {
        let receiver = Receiver::new(None);
        let (address, key) = (receiver.address(), receiver.key());
        drop(receiver);
        let start = Instant::now();
        assert!(send(&address, &key, &Clip::Text("x".into())).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn dropping_the_listener_frees_the_port() {
        let receiver = Receiver::new(None);
        let address: SocketAddr = receiver.listener.local_addr();
        let key = receiver.key();
        drop(receiver);
        assert!(listen(address, key, FakeClipboard::with(None), Box::new(|_| ())).is_ok());
    }

    #[test]
    fn nothing_to_share_sends_nothing() {
        let receiver = Receiver::new(None);
        share_from(None, &receiver, &receiver.key());
        receiver.settle();
        assert_eq!(receiver.clipboard.writes(), 0);
    }
}
