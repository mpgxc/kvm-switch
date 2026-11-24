//! Adapters: as implementações das portas de `ports` para cada sistema.
//!
//! Para dar suporte a outro sistema, crie um módulo aqui com as portas
//! (`Monitor`, `Peripherals`, `Hotkeys`, `EdgeSensor` e, se o ícone na barra
//! morar no app, `StatusMenu`) e as funções `platform` e `first_action` para ele.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod logi_bolt;
#[cfg(target_os = "macos")]
mod macos;
mod mccs;

use std::{process::Command, sync::Arc};

use anyhow::{Context as _, Result, bail};
use async_channel::Sender;

use crate::ports::{Action, EdgeSensor, Hotkeys, Monitor, Peripherals, StatusMenu};

pub struct Platform {
    pub monitor: Arc<dyn Monitor>,
    pub peripherals: Arc<dyn Peripherals>,
    pub hotkeys: Box<dyn Hotkeys>,
    /// Ícone na barra no processo do app. No Linux ele roda à parte (`kvm-switch tray`).
    pub status_menu: Option<Box<dyn StatusMenu>>,
    /// Quem percebe o cursor na borda da tela, para o modo Bordas.
    pub edge_sensor: Box<dyn EdgeSensor>,
}

/// Ícone do sistema em que o app roda, para o destino "este computador".
#[cfg(target_os = "macos")]
pub const LOCAL_ICON: &str = "🍎";

/// Ícone do sistema em que o app roda, para o destino "este computador".
#[cfg(target_os = "linux")]
pub const LOCAL_ICON: &str = "🐧";

/// O que esta execução do programa faz ao abrir. `None`: o pedido foi entregue
/// a um app que já estava aberto e esta execução pode sair.
#[cfg(target_os = "macos")]
pub fn first_action() -> Result<Option<Action>> {
    // O item de login abre o app com `start`, sem janela; o resto mostra a configuração.
    if std::env::args().skip(1).eq(["start"]) {
        return Ok(Some(Action::Start));
    }
    Ok(Some(Action::OpenSettings))
}

#[cfg(target_os = "linux")]
pub use linux::{first_action, run_tray};

#[cfg(target_os = "macos")]
pub use macos::register_login_item;

/// Monitor e periféricos deste sistema, sem janelas nem atalhos.
#[cfg(target_os = "macos")]
pub fn devices() -> (Arc<dyn Monitor>, Arc<dyn Peripherals>) {
    (
        Arc::new(macos::MacMonitor),
        Arc::new(logi_bolt::LogiBolt::default()),
    )
}

/// Monitor e periféricos deste sistema, sem janelas nem atalhos.
#[cfg(target_os = "linux")]
pub fn devices() -> (Arc<dyn Monitor>, Arc<dyn Peripherals>) {
    (
        Arc::new(linux::Ddcutil::new()),
        Arc::new(linux::LogiReceiver),
    )
}

/// Monta os adapters deste sistema. Os atalhos pressionados e os itens do ícone
/// na barra saem por `actions`. No macOS precisa rodar com o app já iniciado.
pub fn platform(actions: Sender<Action>) -> Result<Platform> {
    let (monitor, peripherals) = devices();
    Ok(Platform {
        monitor,
        peripherals: app_peripherals(peripherals),
        status_menu: status_menu(actions.clone())?,
        edge_sensor: edge_sensor(actions.clone()),
        hotkeys: hotkeys(actions)?,
    })
}

/// Os periféricos com o app aberto. No macOS a busca dos slots começa já, para a
/// primeira troca não esperar por ela; a troca sem janela (`devices`) não ganharia
/// nada com isso.
#[cfg(target_os = "macos")]
fn app_peripherals(_: Arc<dyn Peripherals>) -> Arc<dyn Peripherals> {
    let bolt = logi_bolt::LogiBolt::default();
    bolt.search_in_background();
    Arc::new(bolt)
}

/// Os periféricos com o app aberto: no Linux, os mesmos da troca sem janela.
#[cfg(target_os = "linux")]
fn app_peripherals(peripherals: Arc<dyn Peripherals>) -> Arc<dyn Peripherals> {
    peripherals
}

#[cfg(target_os = "macos")]
fn edge_sensor(actions: Sender<Action>) -> Box<dyn EdgeSensor> {
    Box::new(macos::MacEdges::new(actions))
}

#[cfg(target_os = "linux")]
fn edge_sensor(_: Sender<Action>) -> Box<dyn EdgeSensor> {
    // No GNOME quem vê a borda é a extensão, que roda `kvm-switch edge <lado>`.
    Box::new(linux::GnomeEdges::default())
}

#[cfg(target_os = "macos")]
fn status_menu(actions: Sender<Action>) -> Result<Option<Box<dyn StatusMenu>>> {
    Ok(Some(Box::new(macos::MacStatusMenu::new(actions)?)))
}

#[cfg(target_os = "linux")]
fn status_menu(_: Sender<Action>) -> Result<Option<Box<dyn StatusMenu>>> {
    // O ícone fica num processo à parte: o GPUI encerra o app com a última janela.
    Ok(None)
}

#[cfg(target_os = "macos")]
fn hotkeys(actions: Sender<Action>) -> Result<Box<dyn Hotkeys>> {
    Ok(Box::new(macos::MacHotkeys::new(actions)?))
}

#[cfg(target_os = "linux")]
fn hotkeys(actions: Sender<Action>) -> Result<Box<dyn Hotkeys>> {
    // No GNOME cada atalho roda este programa de novo; o pedido chega por aqui.
    linux::listen(actions)?;
    Ok(Box::new(linux::GnomeHotkeys::new()?))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("kvm-switch ainda não tem adapter para este sistema: veja src/adapters.rs");

/// Roda um programa externo e devolve o que ele escreveu na saída padrão.
fn run(program: &str, args: &[&str]) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args);
    // Apps abertos pelo Dock não herdam o PATH do shell, onde fica o Homebrew.
    #[cfg(target_os = "macos")]
    command.env(
        "PATH",
        format!(
            "/opt/homebrew/bin:/usr/local/bin:{}",
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    let output = command
        .output()
        .with_context(|| format!("não consegui executar {program}; ele está instalado?"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{program}: {}", format!("{stdout}{stderr}").trim());
    }
    Ok(stdout.into_owned())
}
