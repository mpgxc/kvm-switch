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

use anyhow::Result;
use gpui::{App, AppContext as _, Application, KeyBinding, Menu, MenuItem, actions};

use crate::{app::AppState, ui::settings};

actions!(kvm_switch, [Quit]);

fn main() {
    let app = Application::new();
    // Fechar a janela não encerra o app; clicar no ícone do Dock abre a configuração de novo.
    app.on_reopen(settings::open);
    app.run(|cx| {
        if let Err(error) = start(cx) {
            eprintln!("kvm-switch: {error:#}");
            cx.quit();
        }
    });
}

fn start(cx: &mut App) -> Result<()> {
    let (sender, actions) = async_channel::unbounded();
    let platform = adapters::platform(sender)?;
    let config_path = config::default_path()?;
    let state = cx.new(|_| AppState::new(platform, config_path));
    cx.set_global(settings::Shared::new(state.clone()));

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
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

    settings::open(cx);
    if state.read(cx).config.targets.is_empty() {
        state.update(cx, |state, cx| state.detect(cx));
    }
    Ok(())
}
