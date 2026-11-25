//! Adapters: as implementações das portas de `ports` para cada sistema.
//!
//! Para dar suporte a outro sistema, crie um módulo aqui com as portas
//! (`Monitor`, `Peripherals`, `Hotkeys`, `EdgeSensor` e, se o ícone na barra
//! morar no app, `StatusMenu`) e a função `platform` para ele, e um transporte em
//! `instance` para o app atender as outras execuções.

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod cursor;
#[cfg(any(target_os = "windows", test))]
mod dib;
mod instance;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod logi_bolt;
#[cfg(target_os = "macos")]
mod macos;
mod mccs;
mod net;
#[cfg(target_os = "windows")]
mod windows;

use std::sync::Arc;

use anyhow::Result;
use async_channel::Sender;

use crate::ports::{Action, Clipboard, EdgeSensor, Hotkeys, Monitor, Peripherals, StatusMenu};

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
    /// Quem percebe o cursor na borda da tela, para o modo Bordas.
    pub edge_sensor: Box<dyn EdgeSensor>,
    /// A área de transferência deste PC, para o modo de mesmo nome.
    pub clipboard: Arc<dyn Clipboard>,
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

pub use instance::{Launch, deliver, first_action, take_over};
pub use net::{ClipListener, SEND_DEADLINE, relisten, share};

#[cfg(target_os = "linux")]
pub use linux::run_tray;

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

/// Monitor e periféricos deste sistema, sem janelas nem atalhos.
#[cfg(target_os = "windows")]
pub fn devices() -> (Arc<dyn Monitor>, Arc<dyn Peripherals>) {
    (
        Arc::new(windows::WindowsMonitor),
        Arc::new(windows::LogiReceiver),
    )
}

/// A área de transferência do usuário.
#[cfg(target_os = "macos")]
pub fn clipboard() -> Arc<dyn Clipboard> {
    Arc::new(macos::MacClipboard::general())
}

/// A área de transferência da sessão GNOME, pela extensão.
#[cfg(target_os = "linux")]
pub fn clipboard() -> Arc<dyn Clipboard> {
    Arc::new(linux::GnomeClipboard)
}

/// A área de transferência do usuário.
#[cfg(target_os = "windows")]
pub fn clipboard() -> Arc<dyn Clipboard> {
    Arc::new(windows::WindowsClipboard)
}

/// Monta os adapters deste sistema. Os atalhos pressionados, os itens do ícone na
/// barra e os pedidos das outras execuções do programa saem por `actions`. No
/// macOS precisa rodar com o app já iniciado.
pub fn platform(actions: Sender<Action>) -> Result<Platform> {
    instance::listen(actions.clone())?;
    let (monitor, peripherals) = devices();
    Ok(Platform {
        monitor,
        peripherals: app_peripherals(peripherals),
        status_menu: status_menu(actions.clone())?,
        edge_sensor: edge_sensor(actions.clone()),
        hotkeys: hotkeys(actions)?,
        clipboard: clipboard(),
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

/// Os periféricos com o app aberto: no Linux e no Windows, os mesmos da troca sem janela.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn app_peripherals(peripherals: Arc<dyn Peripherals>) -> Arc<dyn Peripherals> {
    peripherals
}

#[cfg(target_os = "macos")]
fn edge_sensor(actions: Sender<Action>) -> Box<dyn EdgeSensor> {
    Box::new(cursor::PollingEdges::new(
        actions,
        macos::cursor_and_screens,
    ))
}

#[cfg(target_os = "windows")]
fn edge_sensor(actions: Sender<Action>) -> Box<dyn EdgeSensor> {
    Box::new(cursor::PollingEdges::new(
        actions,
        windows::cursor_and_screens,
    ))
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

#[cfg(target_os = "windows")]
fn status_menu(actions: Sender<Action>) -> Result<Option<Box<dyn StatusMenu>>> {
    Ok(Some(Box::new(windows::WindowsStatusMenu::new(actions)?)))
}

#[cfg(target_os = "macos")]
fn hotkeys(actions: Sender<Action>) -> Result<Box<dyn Hotkeys>> {
    Ok(Box::new(macos::MacHotkeys::new(actions)?))
}

#[cfg(target_os = "linux")]
fn hotkeys(_: Sender<Action>) -> Result<Box<dyn Hotkeys>> {
    // No GNOME cada atalho roda este programa de novo; o pedido chega por `instance`.
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
