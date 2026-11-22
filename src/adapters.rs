//! Adapters: as implementações das portas de `ports` para cada sistema.
//!
//! Para dar suporte a outro sistema, crie um módulo aqui com as portas
//! (`Monitor`, `Peripherals`, `Hotkeys` e, se o ícone na barra morar no app,
//! `StatusMenu`) e as funções `platform` e `first_action` para ele.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod logi_bolt;
#[cfg(target_os = "macos")]
mod macos;
mod mccs;
#[cfg(target_os = "windows")]
mod windows;

use std::sync::Arc;

use anyhow::Result;
use async_channel::Sender;

use crate::ports::{Action, Hotkeys, Monitor, Peripherals, StatusMenu};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use anyhow::{Context as _, bail};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::Command;

pub struct Platform {
    pub monitor: Arc<dyn Monitor>,
    pub peripherals: Arc<dyn Peripherals>,
    pub hotkeys: Box<dyn Hotkeys>,
    /// Ícone na barra no processo do app. No Linux ele roda à parte (`kvm-switch tray`).
    pub status_menu: Option<Box<dyn StatusMenu>>,
}

/// Ícone do sistema em que o app roda, para o destino "este computador".
#[cfg(target_os = "macos")]
pub const LOCAL_ICON: &str = "🍎";

/// Ícone do sistema em que o app roda, para o destino "este computador".
#[cfg(target_os = "linux")]
pub const LOCAL_ICON: &str = "🐧";

/// Ícone do sistema em que o app roda, para o destino "este computador".
#[cfg(target_os = "windows")]
pub const LOCAL_ICON: &str = "🪟";

/// O que esta execução do programa faz ao abrir. `None`: o pedido foi entregue
/// a um app que já estava aberto e esta execução pode sair.
#[cfg(target_os = "macos")]
pub fn first_action() -> Result<Option<Action>> {
    Ok(Some(Action::OpenSettings))
}

#[cfg(target_os = "linux")]
pub use linux::{first_action, run_tray};

/// O que esta execução do programa faz ao abrir no Windows.
#[cfg(target_os = "windows")]
pub fn first_action() -> Result<Option<Action>> {
    Ok(Some(Action::OpenSettings))
}

/// Diz se o Windows deve iniciar o app somente com o ícone na área de notificação.
#[cfg(target_os = "windows")]
pub fn starts_in_background() -> bool {
    std::env::args().skip(1).eq(["--background"])
}

/// Monitor e periféricos deste sistema, sem janelas nem atalhos.
#[cfg(target_os = "macos")]
pub fn devices() -> (Arc<dyn Monitor>, Arc<dyn Peripherals>) {
    (Arc::new(macos::MacMonitor), Arc::new(logi_bolt::LogiBolt))
}

/// Monitor e periféricos deste sistema, sem janelas nem atalhos.
#[cfg(target_os = "linux")]
pub fn devices() -> (Arc<dyn Monitor>, Arc<dyn Peripherals>) {
    (
        Arc::new(linux::Ddcutil::new()),
        Arc::new(linux::LogiReceiver),
    )
}

/// Monitor e periféricos deste sistema, sem janelas nem atalhos.
#[cfg(target_os = "windows")]
pub fn devices() -> (Arc<dyn Monitor>, Arc<dyn Peripherals>) {
    (
        Arc::new(windows::WindowsMonitor),
        Arc::new(windows::LogiReceiver),
    )
}

/// Monta os adapters deste sistema. Os atalhos pressionados e os itens do ícone
/// na barra saem por `actions`. No macOS precisa rodar com o app já iniciado.
pub fn platform(actions: Sender<Action>) -> Result<Platform> {
    let (monitor, peripherals) = devices();
    Ok(Platform {
        monitor,
        peripherals,
        status_menu: status_menu(actions.clone())?,
        hotkeys: hotkeys(actions)?,
    })
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

#[cfg(target_os = "windows")]
fn status_menu(actions: Sender<Action>) -> Result<Option<Box<dyn StatusMenu>>> {
    Ok(Some(Box::new(windows::WindowsStatusMenu::new(actions)?)))
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

#[cfg(target_os = "windows")]
fn hotkeys(actions: Sender<Action>) -> Result<Box<dyn Hotkeys>> {
    Ok(Box::new(windows::WindowsHotkeys::new(actions)?))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
compile_error!("kvm-switch ainda não tem adapter para este sistema: veja src/adapters.rs");

/// Roda um programa externo e devolve o que ele escreveu na saída padrão.
#[cfg(any(target_os = "linux", target_os = "macos"))]
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
