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

use crate::{domain::InputCode, ports::Action};

const USAGE: &str = "uso: kvm-switch [open | switch <entrada> | switcher-next | switcher-previous]\n\
    <entrada> é o valor do VCP 0x60 em decimal, como no config.json";

/// O socket desta execução, de `first_action` até `listen` começar a atendê-lo.
static CLAIMED: Mutex<Option<UnixListener>> = Mutex::new(None);

/// Lê a linha de comando. `None`: o pedido foi entregue ao app já aberto.
pub fn first_action() -> Result<Option<Action>> {
    let request = env::args().skip(1).collect::<Vec<_>>().join(" ");
    let action = parse(&request)?;
    let path = socket_path()?;
    if let Ok(mut running) = UnixStream::connect(&path) {
        writeln!(running, "{}", arguments(action)).context("o app aberto não recebeu o pedido")?;
        return Ok(None);
    }
    // Quem vai abrir janela segura o socket já aqui: os toques seguidos do atalho
    // enquanto o app abre ficam na fila em vez de abrirem outro app. Uma troca
    // direta tenta antes rodar sem janela (veja `main`) e só o segura se abrir.
    if !matches!(action, Action::SwitchTo(_)) {
        *CLAIMED.lock().expect("lock do socket") = Some(claim(&path)?);
    }
    Ok(Some(action))
}

/// Passa a atender os pedidos das próximas execuções.
pub fn listen(actions: Sender<Action>) -> Result<()> {
    let claimed = CLAIMED.lock().expect("lock do socket").take();
    let listener = match claimed {
        Some(listener) => listener,
        None => claim(&socket_path()?)?,
    };
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            // Quem conecta e não escreve não pode travar os próximos pedidos.
            stream.set_read_timeout(Some(Duration::from_secs(1))).ok();
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

/// Argumentos que fazem uma nova execução pedir `action`.
pub fn arguments(action: Action) -> String {
    match action {
        Action::SwitchTo(input) => format!("switch {}", input.0),
        Action::SwitcherNext => "switcher-next".into(),
        Action::SwitcherPrevious => "switcher-previous".into(),
        Action::OpenSettings => "open".into(),
    }
}

/// Sem palavras, como numa execução sem argumentos, o pedido é abrir a configuração.
fn parse(request: &str) -> Result<Action> {
    let words: Vec<&str> = request.split_whitespace().collect();
    Ok(match words.as_slice() {
        [] | ["open"] => Action::OpenSettings,
        ["switch", input] => Action::SwitchTo(InputCode(
            input
                .parse()
                .with_context(|| format!("entrada inválida: {input}\n{USAGE}"))?,
        )),
        ["switcher-next"] => Action::SwitcherNext,
        ["switcher-previous"] => Action::SwitcherPrevious,
        _ => bail!("{USAGE}"),
    })
}

fn socket_path() -> Result<PathBuf> {
    let dir = dirs::runtime_dir().context("esta sessão não tem XDG_RUNTIME_DIR")?;
    Ok(dir.join("kvm-switch.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_survives_the_command_line() {
        // O atalho do GNOME roda `kvm-switch <arguments>`; se a volta falhar, o atalho não faz nada.
        for action in [
            Action::SwitchTo(InputCode(0x1b)),
            Action::SwitcherNext,
            Action::SwitcherPrevious,
            Action::OpenSettings,
        ] {
            assert_eq!(parse(&arguments(action)).expect("pedido"), action);
        }
    }

    #[test]
    fn unknown_requests_are_refused() {
        assert!(parse("switch").is_err());
        assert!(parse("switch usb-c").is_err());
        assert!(parse("switch 300").is_err());
        assert!(parse("reboot").is_err());
    }
}
