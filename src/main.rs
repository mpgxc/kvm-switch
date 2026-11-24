//! kvm-switch: troca a entrada do monitor e o canal dos periféricos por atalho.
//!
//! `domain` tem as regras, `ports` as interfaces que cada sistema precisa
//! fornecer, `adapters` as implementações por sistema e `ui` as janelas.

#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

mod adapters;
mod app;
mod config;
mod domain;
mod ports;
mod request;
mod tray;
mod ui;

use std::{process, time::Duration};

use anyhow::Result;
use gpui::{App, AppContext as _, Application, KeyBinding, Menu, MenuItem, actions};

use crate::{
    adapters::Launch,
    app::AppState,
    domain::{InputCode, Kvm, Side},
    ports::Action,
    ui::{
        overlay::{self, HudView, Question},
        settings,
    },
};

actions!(kvm_switch, [Quit]);

/// Código de saída quando o pedido da linha de comando é recusado ou não chega
/// ao app aberto.
const EXIT_REFUSED: i32 = 2;
/// Quanto um erro ao abrir fica na tela antes de esta execução sair.
const FAILURE_NOTICE: Duration = Duration::from_secs(4);

fn main() {
    // O ícone na barra do Linux é um processo à parte, sem GPUI.
    #[cfg(target_os = "linux")]
    if std::env::args().skip(1).eq(["tray"]) {
        if let Err(error) = adapters::run_tray() {
            eprintln!("kvm-switch: {error:#}");
            process::exit(EXIT_REFUSED);
        }
        return;
    }
    let launch = match adapters::first_action() {
        Ok(launch) => launch,
        Err(error) => {
            eprintln!("kvm-switch: {error:#}");
            process::exit(EXIT_REFUSED);
        }
    };
    // Uma troca pedida com o app fechado roda sem janela, sem esperar o GPUI
    // iniciar. O app só abre se sobrar um aviso para mostrar.
    let notice = match launch {
        // Já havia um app aberto e ele recebeu o pedido.
        Launch::Delivered => return,
        Launch::Run(Action::SwitchTo(input)) => switch_without_window(input),
        Launch::Run(Action::CrossEdge(side)) => cross_without_window(side),
        _ => None,
    };
    if matches!(
        launch,
        Launch::Run(Action::SwitchTo(_) | Action::CrossEdge(_))
    ) && notice.is_none()
    {
        return;
    }
    let app = Application::new();
    // No macOS fechar a janela não encerra o app, e clicar no ícone do Dock abre a
    // configuração de novo. No Windows, o ícone na área de notificação mantém o app
    // aberto quando a janela fecha. No Linux o GPUI encerra com a última janela.
    app.on_reopen(settings::open);
    app.run(move |cx| match launch {
        Launch::Run(first) => {
            if let Err(error) = start(cx, first, notice) {
                eprintln!("kvm-switch: {error:#}");
                cx.quit();
            }
        }
        Launch::AlreadyOpen => ask_to_replace(cx),
        Launch::ConfirmQuit => ask_to_quit_from_outside(cx),
        // Já tratado antes de o GPUI iniciar.
        Launch::Delivered => cx.quit(),
    });
    // Só no Linux a execução chega aqui: uma troca ainda rodando vai até o fim.
    app::finish_switches();
}

/// Abriram o app com outro já aberto: pergunta se encerra o aberto e abre este no
/// lugar dele, ou só mostra a janela do aberto.
fn ask_to_replace(cx: &mut App) {
    let question = Question {
        message: "O KVM Switch já está aberto".into(),
        detail: "Encerrar o que está aberto e abrir de novo?".into(),
        yes: "Encerrar e abrir de novo".into(),
        no: "Mostrar o aberto".into(),
    };
    let opened = overlay::open_question(question, cx, |replace, question, cx| {
        if !replace {
            // Melhor esforço: se o aberto fechou neste instante, não há janela para mostrar.
            adapters::deliver(Action::OpenSettings).ok();
            cx.quit();
            return;
        }
        cx.spawn(async move |cx| {
            let taken = cx
                .background_executor()
                .spawn(async { adapters::take_over() })
                .await;
            cx.update(|cx| {
                match taken.and_then(|()| start(cx, Action::OpenSettings, None)) {
                    // A configuração abre adiada (veja `AppState::handle`) e a pergunta só
                    // fecha depois dela: no Linux o app encerra quando fica sem janelas.
                    Ok(()) => cx.defer(move |cx| overlay::close(question, cx)),
                    Err(error) => fail(format!("{error:#}"), question, cx),
                }
            })
            .ok();
        })
        .detach();
    });
    if opened.is_none() {
        cx.quit();
    }
}

/// `kvm-switch quit`: pergunta aqui mesmo e, com o sim, encerra o app aberto. O
/// código de saída diz a quem chamou (o ícone na barra do Linux) se a pessoa confirmou.
fn ask_to_quit_from_outside(cx: &mut App) {
    let opened = overlay::open_question(app::quit_question(), cx, |quit, _, cx| {
        if !quit {
            process::exit(EXIT_REFUSED);
        }
        // Melhor esforço: sem app aberto não há o que encerrar.
        adapters::deliver(Action::QuitNow).ok();
        cx.quit();
    });
    if opened.is_none() {
        process::exit(EXIT_REFUSED);
    }
}

/// Mostra o erro no lugar da pergunta por um tempo e sai.
fn fail(message: String, question: gpui::AnyWindowHandle, cx: &mut App) {
    eprintln!("kvm-switch: {message}");
    overlay::open_hud(
        HudView {
            message: message.into(),
        },
        cx,
    );
    overlay::close(question, cx);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(FAILURE_NOTICE).await;
        cx.update(|cx| cx.quit()).ok();
    })
    .detach();
}

/// Faz a troca sem abrir janela. Devolve o aviso que sobrou para mostrar, se houver.
fn switch_without_window(input: InputCode) -> Option<String> {
    let attempt = || -> Result<Option<String>> {
        let config = config::load(&config::default_path()?)?;
        let Some(target) = config.target(input) else {
            return Ok(None);
        };
        if !config.monitor_mode {
            return Ok(Some(app::MONITOR_MODE_OFF.into()));
        }
        let (monitor, peripherals) = adapters::devices();
        let outcome = Kvm::new(monitor, peripherals).switch(target, config.local_input);
        Ok(app::notice(&outcome, &target.name))
    };
    attempt().unwrap_or_else(|error| Some(format!("{error:#}")))
}

/// Manda teclado e mouse para o vizinho de `side` sem abrir janela. É o caminho
/// da extensão do GNOME. Devolve o erro para mostrar, se houver.
fn cross_without_window(side: Side) -> Option<String> {
    let attempt = || -> Result<()> {
        let config = config::load(&config::default_path()?)?;
        let (monitor, peripherals) = adapters::devices();
        Kvm::new(monitor, peripherals).cross(&config.edges, side)?;
        Ok(())
    };
    attempt().err().map(|error| format!("{error:#}"))
}

fn start(cx: &mut App, first: Action, notice: Option<String>) -> Result<()> {
    let (sender, actions) = async_channel::unbounded();
    let platform = adapters::platform(sender)?;
    // Melhor esforço: sem o item de login o app só deixa de abrir sozinho no próximo login.
    #[cfg(target_os = "macos")]
    adapters::register_login_item().ok();
    let config_path = config::default_path()?;
    let state = cx.new(|_| AppState::new(platform, config_path));
    cx.set_global(settings::Shared::new(state.clone()));

    let asked = state.clone();
    cx.on_action(move |_: &Quit, cx| asked.update(cx, |state, cx| state.handle(Action::Quit, cx)));
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
