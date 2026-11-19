//! Adapters: as implementações das portas de `ports` para cada sistema.
//!
//! Para dar suporte a outro sistema, crie um módulo aqui com as três portas
//! (`Monitor`, `Peripherals`, `Hotkeys`) e as funções `platform` e
//! `first_action` para ele.

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

use crate::ports::{Action, Hotkeys, Monitor, Peripherals};

pub struct Platform {
    pub monitor: Arc<dyn Monitor>,
    pub peripherals: Arc<dyn Peripherals>,
    pub hotkeys: Box<dyn Hotkeys>,
}

/// O que esta execução do programa faz ao abrir. `None`: o pedido foi entregue
/// a um app que já estava aberto e esta execução pode sair.
#[cfg(target_os = "macos")]
pub fn first_action() -> Result<Option<Action>> {
    Ok(Some(Action::OpenSettings))
}

#[cfg(target_os = "linux")]
pub use linux::first_action;

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

/// Monta os adapters deste sistema. Os atalhos pressionados saem por `actions`.
pub fn platform(actions: Sender<Action>) -> Result<Platform> {
    let (monitor, peripherals) = devices();
    Ok(Platform {
        monitor,
        peripherals,
        hotkeys: hotkeys(actions)?,
    })
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
