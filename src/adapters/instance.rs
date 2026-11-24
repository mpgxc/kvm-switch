//! Um app por sessão, com pedidos pela linha de comando.
//!
//! No GNOME um atalho global, a extensão das bordas e cada item do ícone na barra
//! rodam este programa de novo. Cada execução ou vira o app, ou entrega o pedido ao
//! app que já está aberto e sai. Abrir o app à mão com ele já aberto não entrega
//! nada: pergunta se é para encerrar o aberto e abrir de novo.

#[cfg(windows)]
#[path = "instance/pipe.rs"]
mod transport;
#[cfg(unix)]
#[path = "instance/socket.rs"]
mod transport;

use std::{
    env,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use async_channel::Sender;

use crate::{
    ports::Action,
    request::{arguments, parse},
};

/// Quanto esperar o app aberto encerrar quando outro vai ocupar o lugar dele. Ele
/// termina antes as trocas em andamento.
const TAKE_OVER_TIMEOUT: Duration = Duration::from_secs(10);
const TAKE_OVER_POLL: Duration = Duration::from_millis(100);

/// O que esta execução do programa faz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// O pedido foi entregue ao app aberto, ou não havia o que fazer: pode sair.
    Delivered,
    /// Esta execução vira o app (ou faz a troca sem janela), começando por esta ação.
    Run(Action),
    /// A pessoa abriu o app com outro já aberto: perguntar se troca um pelo outro.
    AlreadyOpen,
    /// Pediram para encerrar o app: perguntar antes.
    ConfirmQuit,
}

/// O que fazer com um pedido, sabendo se há app aberto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Deliver,
    Nothing,
    /// Rodar aqui; `claim` segura já o lugar de app.
    Run {
        claim: bool,
    },
    AlreadyOpen,
    ConfirmQuit,
}

fn plan(action: Action, running: bool) -> Plan {
    match action {
        Action::Quit => Plan::ConfirmQuit,
        Action::QuitNow if running => Plan::Deliver,
        Action::QuitNow => Plan::Nothing,
        Action::OpenSettings if running => Plan::AlreadyOpen,
        _ if running => Plan::Deliver,
        // Uma troca tenta antes rodar sem janela (veja `main`) e só vira app se abrir.
        Action::SwitchTo(_) | Action::CrossEdge(_) => Plan::Run { claim: false },
        // Quem vai abrir janela segura o lugar já aqui: os toques seguidos do atalho
        // enquanto o app abre ficam na fila em vez de abrirem outro app.
        _ => Plan::Run { claim: true },
    }
}

/// O lugar de app desta execução, de `first_action` (ou `take_over`) até `listen`.
static CLAIMED: Mutex<Option<transport::Listener>> = Mutex::new(None);

/// Lê a linha de comando e decide o que esta execução faz.
pub fn first_action() -> Result<Launch> {
    let request = env::args().skip(1).collect::<Vec<_>>().join(" ");
    let action = parse(&request)?;
    // Linha vazia: só confere se há app aberto (veja `listen`).
    let running = transport::send("")?;
    Ok(match plan(action, running) {
        Plan::Deliver => {
            deliver(action)?;
            Launch::Delivered
        }
        Plan::Nothing => Launch::Delivered,
        Plan::Run { claim } => {
            if claim {
                hold(transport::claim()?);
            }
            Launch::Run(action)
        }
        Plan::AlreadyOpen => Launch::AlreadyOpen,
        Plan::ConfirmQuit => Launch::ConfirmQuit,
    })
}

/// Entrega o pedido ao app aberto. `Ok(false)`: não havia app aberto.
pub fn deliver(action: Action) -> Result<bool> {
    transport::send(&arguments(action))
}

/// Encerra o app aberto sem perguntar e fica com o lugar dele.
pub fn take_over() -> Result<()> {
    deliver(Action::QuitNow)?;
    let deadline = Instant::now() + TAKE_OVER_TIMEOUT;
    while transport::send("")? {
        if Instant::now() > deadline {
            bail!("o app aberto não encerrou");
        }
        thread::sleep(TAKE_OVER_POLL);
    }
    hold(transport::claim()?);
    Ok(())
}

/// Passa a atender os pedidos das próximas execuções.
pub fn listen(actions: Sender<Action>) -> Result<()> {
    let claimed = CLAIMED
        .lock()
        .expect("lock do lugar de app envenenado")
        .take();
    let listener = match claimed {
        Some(listener) => listener,
        None => transport::claim()?,
    };
    listener.serve(move |line| {
        // Linha vazia é só alguém conferindo se há um app aberto.
        if line.trim().is_empty() {
            return true;
        }
        let Ok(action) = parse(line) else {
            return true;
        };
        // O receptor só some quando o app está encerrando.
        actions.try_send(action).is_ok()
    });
    Ok(())
}

fn hold(listener: transport::Listener) {
    *CLAIMED.lock().expect("lock do lugar de app envenenado") = Some(listener);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{InputCode, Side};

    #[test]
    fn opening_by_hand_with_an_app_open_asks_instead_of_delivering() {
        assert_eq!(plan(Action::OpenSettings, true), Plan::AlreadyOpen);
        assert_eq!(plan(Action::OpenSettings, false), Plan::Run { claim: true });
    }

    #[test]
    fn shortcuts_edges_and_menu_items_go_straight_to_the_open_app() {
        for action in [
            Action::SwitchTo(InputCode(27)),
            Action::CrossEdge(Side::Right),
            Action::SwitcherNext,
            Action::SwitcherPrevious,
            Action::Start,
            Action::QuitNow,
        ] {
            assert_eq!(plan(action, true), Plan::Deliver, "{action:?}");
        }
    }

    #[test]
    fn switches_without_an_open_app_run_here_without_holding_the_place() {
        assert_eq!(
            plan(Action::SwitchTo(InputCode(27)), false),
            Plan::Run { claim: false }
        );
        assert_eq!(
            plan(Action::CrossEdge(Side::Left), false),
            Plan::Run { claim: false }
        );
        assert_eq!(plan(Action::Start, false), Plan::Run { claim: true });
    }

    #[test]
    fn quitting_always_asks_and_quitting_now_needs_an_open_app() {
        assert_eq!(plan(Action::Quit, true), Plan::ConfirmQuit);
        assert_eq!(plan(Action::Quit, false), Plan::ConfirmQuit);
        assert_eq!(plan(Action::QuitNow, false), Plan::Nothing);
    }
}
