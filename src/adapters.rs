//! Adapters: as implementações das portas de `ports` para cada sistema.
//!
//! Para dar suporte a outro sistema, crie um módulo aqui com as três portas
//! (`Monitor`, `Peripherals`, `Hotkeys`) e uma função `platform` para ele.

mod logi_bolt;
#[cfg(target_os = "macos")]
mod macos;

use std::{env, process::Command, sync::Arc};

use anyhow::{Context as _, Result, bail};
use async_channel::Sender;

use crate::ports::{Action, Hotkeys, Monitor, Peripherals};

pub struct Platform {
    pub monitor: Arc<dyn Monitor>,
    pub peripherals: Arc<dyn Peripherals>,
    pub hotkeys: Box<dyn Hotkeys>,
}

/// Monta os adapters deste sistema. Os atalhos pressionados saem por `actions`.
#[cfg(target_os = "macos")]
pub fn platform(actions: Sender<Action>) -> Result<Platform> {
    Ok(Platform {
        monitor: Arc::new(macos::MacMonitor),
        peripherals: Arc::new(logi_bolt::LogiBolt),
        hotkeys: Box::new(macos::MacHotkeys::new(actions)?),
    })
}

#[cfg(not(target_os = "macos"))]
compile_error!("kvm-switch ainda não tem adapter para este sistema: veja src/adapters.rs");

/// Roda um programa externo e devolve o que ele escreveu na saída padrão.
fn run(program: &str, args: &[&str]) -> Result<String> {
    // Apps abertos pelo Dock não herdam o PATH do shell, onde fica o Homebrew.
    let path = format!(
        "/opt/homebrew/bin:/usr/local/bin:{}",
        env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(program)
        .args(args)
        .env("PATH", path)
        .output()
        .with_context(|| format!("não consegui executar {program}; ele está instalado?"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{program}: {}", format!("{stdout}{stderr}").trim());
    }
    Ok(stdout.into_owned())
}
