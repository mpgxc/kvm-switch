//! Um app por sessão, com pedidos pela linha de comando.
//!
//! No GNOME um atalho global roda um comando, então cada atalho é uma nova
//! execução deste programa. Ela entrega o pedido por um socket ao app que já
//! está aberto e sai; se não há app aberto, ela vira o app.

use std::{
    env, fs,
    io::{BufRead as _, BufReader, Write as _},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result, bail};
use async_channel::Sender;

use crate::{
    ports::Action,
    request::{arguments, parse},
};

/// Quem conecta e não escreve não pode travar os próximos pedidos: a leitura
/// desiste depois deste tempo.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

/// O socket desta execução, de `first_action` até `listen` começar a atendê-lo.
static CLAIMED: Mutex<Option<UnixListener>> = Mutex::new(None);

/// Lê a linha de comando. `None`: o pedido foi entregue ao app já aberto, ou
/// não havia o que fazer.
pub fn first_action() -> Result<Option<Action>> {
    let request = env::args().skip(1).collect::<Vec<_>>().join(" ");
    let action = parse(&request)?;
    let path = socket_path()?;
    if let Ok(mut running) = UnixStream::connect(&path) {
        writeln!(running, "{}", arguments(action)).context("o app aberto não recebeu o pedido")?;
        return Ok(None);
    }
    // Encerrar um app que não está aberto não pede nada.
    if action == Action::Quit {
        return Ok(None);
    }
    // Quem vai abrir janela segura o socket já aqui: os toques seguidos do atalho
    // enquanto o app abre ficam na fila em vez de abrirem outro app. Uma troca
    // direta tenta antes rodar sem janela (veja `main`) e só o segura se abrir.
    if !matches!(action, Action::SwitchTo(_)) {
        *CLAIMED.lock().expect("lock do socket envenenado") = Some(claim(&path)?);
    }
    Ok(Some(action))
}

/// Passa a atender os pedidos das próximas execuções.
pub fn listen(actions: Sender<Action>) -> Result<()> {
    let claimed = CLAIMED.lock().expect("lock do socket envenenado").take();
    let listener = match claimed {
        Some(listener) => listener,
        None => claim(&socket_path()?)?,
    };
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            stream.set_read_timeout(Some(REQUEST_TIMEOUT)).ok();
            let mut request = String::new();
            let read = BufReader::new(stream).read_line(&mut request);
            // Linha vazia é só alguém conferindo se há um app aberto (veja `claim`).
            if read.is_ok()
                && !request.trim().is_empty()
                && let Ok(action) = parse(&request)
                && actions.try_send(action).is_err()
            {
                // O receptor só some quando o app está encerrando.
                break;
            }
        }
    });
    Ok(())
}

/// Cria o socket do app.
fn claim(path: &Path) -> Result<UnixListener> {
    UnixListener::bind(path).or_else(|_| {
        // O caminho já existe: ou outro app abriu neste instante, ou sobrou de um que fechou.
        if UnixStream::connect(path).is_ok() {
            bail!("o kvm-switch já está aberto");
        }
        fs::remove_file(path).ok();
        UnixListener::bind(path).with_context(|| format!("não consegui criar {}", path.display()))
    })
}

fn socket_path() -> Result<PathBuf> {
    let dir = dirs::runtime_dir().context("esta sessão não tem XDG_RUNTIME_DIR")?;
    Ok(dir.join("kvm-switch.sock"))
}
