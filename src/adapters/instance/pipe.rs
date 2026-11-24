//! Windows: o app atende num named pipe do usuário. Quem consegue criar a primeira
//! instância do pipe é o app; as outras execuções escrevem nele como num arquivo.

use std::{
    env,
    fs::{File, OpenOptions},
    io::{self, BufRead as _, BufReader, Write as _},
    os::windows::{
        ffi::OsStrExt as _,
        io::{FromRawHandle as _, OwnedHandle, RawHandle},
    },
    ptr, thread,
    time::Duration,
};

use anyhow::{Result, bail};
use windows_sys::Win32::{
    Foundation::{ERROR_PIPE_BUSY, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND},
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    },
};

/// Quantas vezes insistir quando todas as instâncias do pipe estão ocupadas.
const BUSY_RETRIES: u32 = 20;
const BUSY_WAIT: Duration = Duration::from_millis(50);
/// Tamanho do buffer do pipe: os pedidos são linhas curtas.
const BUFFER: u32 = 512;

/// A instância livre do pipe, à espera do próximo pedido.
pub struct Listener {
    name: String,
    pipe: OwnedHandle,
}

/// Vira o app. Erro se outro app deste usuário já atende.
pub fn claim() -> Result<Listener> {
    claim_at(&name())
}

/// Manda uma linha ao app aberto. `Ok(false)`: não há app aberto.
pub fn send(line: &str) -> Result<bool> {
    send_to(&name(), line)
}

impl Listener {
    /// Atende numa thread. Cada linha vai para `on_line`; `false` para de atender.
    pub fn serve(self, mut on_line: impl FnMut(&str) -> bool + Send + 'static) {
        thread::spawn(move || {
            let Listener { name, mut pipe } = self;
            loop {
                wait_for_client(&pipe);
                // A próxima instância nasce antes desta fechar: no intervalo, quem
                // procurasse o app acharia o pipe vazio e abriria outro.
                let Ok(next) = create(&name, false) else {
                    break;
                };
                let mut line = String::new();
                let read = BufReader::new(File::from(pipe)).read_line(&mut line);
                pipe = next;
                if read.is_ok() && !on_line(&line) {
                    break;
                }
            }
        });
    }
}

fn claim_at(name: &str) -> Result<Listener> {
    match create(name, true) {
        Ok(pipe) => Ok(Listener {
            name: name.to_string(),
            pipe,
        }),
        Err(_) => bail!("o kvm-switch já está aberto"),
    }
}

fn send_to(name: &str, line: &str) -> Result<bool> {
    for _ in 0..BUSY_RETRIES {
        match OpenOptions::new().write(true).open(name) {
            Ok(mut running) => {
                writeln!(running, "{line}")?;
                // Num pipe, esperar o esvaziamento é esperar o app ler a linha. Fechar
                // antes disso pode descartá-la. Melhor esforço: a linha já foi escrita.
                running.sync_all().ok();
                return Ok(true);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                thread::sleep(BUSY_WAIT);
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("o app aberto está ocupado e não recebeu o pedido")
}

/// Cria uma instância do pipe. `first`: só se ninguém tiver criado o pipe antes.
fn create(name: &str, first: bool) -> io::Result<OwnedHandle> {
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain([0])
        .collect();
    let open_mode = PIPE_ACCESS_INBOUND
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // SAFETY: `wide` termina em zero e vive durante a chamada; sem atributos de
    // segurança o pipe fica com as permissões padrão do usuário.
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            0,
            ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: o handle acabou de ser criado e passa a ter um único dono.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) })
}

/// Espera alguém abrir esta instância. O resultado não importa: quem escreveu e já
/// fechou (`ERROR_NO_DATA`) ainda deixa a linha para ler, e uma instância que não
/// conectou só devolve uma leitura vazia. Desistir aqui deixaria o app sem pipe.
fn wait_for_client(pipe: &OwnedHandle) {
    use std::os::windows::io::AsRawHandle as _;
    // SAFETY: o handle é válido enquanto `pipe` existir; sem OVERLAPPED a chamada bloqueia.
    unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) };
}

/// Um pipe por usuário: o mesmo Windows pode ter mais de uma pessoa logada.
fn name() -> String {
    let user = env::var("USERNAME").unwrap_or_default();
    format!(r"\\.\pipe\kvm-switch-{user}")
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn one_app_holds_the_pipe_and_receives_the_lines() {
        let name = format!(r"\\.\pipe\kvm-switch-test-{}", std::process::id());

        assert!(!send_to(&name, "open").expect("enviar"), "sem app aberto");
        let listener = claim_at(&name).expect("primeiro app");
        assert!(claim_at(&name).is_err(), "o segundo não vira app");

        let (lines, received) = mpsc::channel();
        listener.serve(move |line| lines.send(line.trim().to_string()).is_ok());
        for request in ["", "switch 27"] {
            assert!(send_to(&name, request).expect("enviar"));
        }
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("linha"),
            ""
        );
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("linha"),
            "switch 27"
        );
    }
}
