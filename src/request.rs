//! Pedidos em texto: a linha de comando, o socket do app aberto no Linux e os
//! itens do ícone na barra falam a mesma língua.

use anyhow::{Context as _, Result, bail};

use crate::{
    domain::{InputCode, Side},
    ports::Action,
};

const USAGE: &str = "uso: kvm-switch [open | switch <entrada> | edge <lado> | switcher-next | switcher-previous | start | quit | tray]\n\
    <entrada> é o valor do VCP 0x60 em decimal, como no config.json\n\
    <lado> é esquerda, direita, cima ou baixo";

/// O texto que pede `action`.
pub fn arguments(action: Action) -> String {
    match action {
        Action::SwitchTo(input) => format!("switch {}", input.0),
        Action::SwitcherNext => "switcher-next".into(),
        Action::SwitcherPrevious => "switcher-previous".into(),
        Action::OpenSettings => "open".into(),
        Action::Quit => "quit".into(),
        Action::CrossEdge(side) => format!("edge {}", side.word()),
        Action::Start => "start".into(),
    }
}

/// Sem palavras, como numa execução sem argumentos, o pedido é abrir a configuração.
pub fn parse(request: &str) -> Result<Action> {
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
        ["quit"] => Action::Quit,
        ["edge", side] => Action::CrossEdge(
            Side::from_word(side).with_context(|| format!("lado inválido: {side}\n{USAGE}"))?,
        ),
        ["start"] => Action::Start,
        _ => bail!("{USAGE}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_survives_the_round_trip() {
        // O atalho do GNOME e o menu do ícone mandam `arguments`; se a volta falhar, nada acontece.
        for action in [
            Action::SwitchTo(InputCode(0x1b)),
            Action::SwitcherNext,
            Action::SwitcherPrevious,
            Action::OpenSettings,
            Action::Quit,
            Action::CrossEdge(Side::Left),
            Action::CrossEdge(Side::Bottom),
            Action::Start,
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
        assert!(parse("quit now").is_err());
        assert!(parse("edge").is_err());
        assert!(parse("edge right").is_err());
    }
}
