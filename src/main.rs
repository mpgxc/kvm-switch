//! kvm-switch: troca a entrada do monitor e o canal dos periféricos por atalho.
//!
//! `domain` tem as regras, `ports` as interfaces que cada sistema precisa
//! fornecer, `adapters` as implementações por sistema e `ui` as janelas.

mod adapters;
mod app;
mod config;
mod domain;
mod ports;
mod ui;

use std::process;

use anyhow::Result;
use gpui::{App, AppContext as _, Application, KeyBinding, Menu, MenuItem, actions};

use crate::{
    app::AppState,
    domain::{InputCode, Kvm},
    ports::Action,
    ui::settings,
};

actions!(kvm_switch, [Quit]);

/// Código de saída quando o pedido da linha de comando é recusado ou não chega
/// ao app aberto.
const EXIT_REFUSED: i32 = 2;

fn main() {
    let first = match adapters::first_action() {
        Ok(Some(action)) => action,
        // Já havia um app aberto e ele recebeu o pedido.
        Ok(None) => return,
        Err(error) => {
            eprintln!("kvm-switch: {error:#}");
            process::exit(EXIT_REFUSED);
        }
    };
    // Uma troca pedida com o app fechado roda sem janela, sem esperar o GPUI
    // iniciar. O app só abre se sobrar um aviso para mostrar.
    let mut notice = None;
    if let Action::SwitchTo(input) = first {
        notice = switch_without_window(input);
        if notice.is_none() {
            return;
        }
    }
    let app = Application::new();
    // No macOS fechar a janela não encerra o app, e clicar no ícone do Dock abre a
    // configuração de novo. No Linux o GPUI encerra o app com a última janela, e
    // abrir o programa outra vez mostra a configuração (veja `first_action`).
    app.on_reopen(settings::open);
    app.run(move |cx| {
        if let Err(error) = start(cx, first, notice) {
            eprintln!("kvm-switch: {error:#}");
            cx.quit();
        }
    });
    // Só no Linux a execução chega aqui: uma troca ainda rodando vai até o fim.
    app::finish_switches();
}

/// Faz a troca sem abrir janela. Devolve o aviso que sobrou para mostrar, se houver.
fn switch_without_window(input: InputCode) -> Option<String> {
    let attempt = || -> Result<Option<String>> {
        let config = config::load(&config::default_path()?)?;
        let Some(target) = config.target(input) else {
            return Ok(None);
        };
        let (monitor, peripherals) = adapters::devices();
        let outcome = Kvm::new(monitor, peripherals).switch(target, config.local_input);
        Ok(app::notice(&outcome, &target.name))
    };
    attempt().unwrap_or_else(|error| Some(format!("{error:#}")))
}

fn start(cx: &mut App, first: Action, notice: Option<String>) -> Result<()> {
    let (sender, actions) = async_channel::unbounded();
    let platform = adapters::platform(sender)?;
    let config_path = config::default_path()?;
    let state = cx.new(|_| AppState::new(platform, config_path));
    cx.set_global(settings::Shared::new(state.clone()));

    cx.on_action(|_: &Quit, cx| cx.quit());
    let quit_key = if cfg!(target_os = "macos") {
        "cmd-q"
    } else {
        "ctrl-q"
    };
    cx.bind_keys([KeyBinding::new(quit_key, Quit, None)]);
    cx.set_menus(vec![Menu {
        name: "KVM Switch".into(),
        items: vec![MenuItem::action("Encerrar KVM Switch", Quit)],
    }]);

    // Os atalhos globais chegam pelo canal e são tratados na thread da interface.
    let handler = state.clone();
    cx.spawn(async move |cx| {
        while let Ok(action) = actions.recv().await {
            handler
                .update(cx, |state, cx| state.handle(action, cx))
                .ok();
        }
    })
    .detach();

    state.update(cx, |state, cx| match notice {
        // A troca já rodou sem janela e sobrou este aviso.
        Some(message) => state.show_hud(message, cx),
        // Aberto pelo usuário, mostra a configuração; por um atalho, o atende.
        None => state.handle(first, cx),
    });
    if state.read(cx).config.targets.is_empty() {
        state.update(cx, |state, cx| state.detect(cx));
    }
    Ok(())
}
