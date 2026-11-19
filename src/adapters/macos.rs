//! macOS: monitor por `m1ddc` (e DDPM para listar entradas) e atalhos globais
//! pelo Carbon, via `global-hotkey`.

use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, bail};
use async_channel::Sender;
use core_graphics::{
    event::{CGEvent, CGEventFlags},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, Modifiers},
};

use super::{mccs::capability_inputs, run};
use crate::{
    domain::InputCode,
    ports::{Action, Hotkeys, Monitor},
};

/// Linha de comando do Dell Display and Peripheral Manager.
const DDPM: &str = "/Applications/DDPM/DDPM";

pub struct MacMonitor;

impl Monitor for MacMonitor {
    fn inputs(&self) -> Result<Vec<InputCode>> {
        // O m1ddc não lê a lista de entradas; o DDPM devolve a capability string do monitor.
        let reply = run(DDPM, &["/get", "-display=Capabilitiesstring"])?;
        ddpm_inputs(&reply)
    }

    fn current_input(&self) -> Result<InputCode> {
        m1ddc_input(&run("m1ddc", &["get", "input"])?)
    }

    fn set_input(&self, input: InputCode) -> Result<()> {
        // DDC direto: o DDPM leva ~6 s por troca, o m1ddc ~0,1 s.
        run("m1ddc", &["set", "input", &input.0.to_string()]).map(drop)
    }
}

/// Entradas listadas na resposta JSON do DDPM.
fn ddpm_inputs(reply: &str) -> Result<Vec<InputCode>> {
    let reply: serde_json::Value =
        serde_json::from_str(reply).context("o DDPM não respondeu em JSON")?;
    if reply["Result"] != "PASS" {
        bail!("o DDPM não leu o monitor: {}", reply["Message"]);
    }
    let capabilities = reply["Value"]
        .as_str()
        .context("resposta do DDPM sem capability string")?;
    capability_inputs(capabilities)
}

/// O `m1ddc get input` imprime o valor de 16 bits do VCP; a entrada é o byte baixo.
fn m1ddc_input(output: &str) -> Result<InputCode> {
    let value: u16 = output
        .trim()
        .parse()
        .with_context(|| format!("resposta inesperada do m1ddc: {}", output.trim()))?;
    Ok(InputCode(value.to_le_bytes()[0]))
}

pub struct MacHotkeys {
    manager: GlobalHotKeyManager,
    bound: Vec<HotKey>,
    actions: Arc<Mutex<HashMap<u32, Action>>>,
}

impl MacHotkeys {
    /// Precisa ser criado na thread principal, onde roda o laço de eventos do app.
    pub fn new(sender: Sender<Action>) -> Result<Self> {
        let manager =
            GlobalHotKeyManager::new().context("não consegui iniciar os atalhos globais")?;
        let actions: Arc<Mutex<HashMap<u32, Action>>> = Arc::default();
        let registered = actions.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() != HotKeyState::Pressed {
                return;
            }
            let action = registered
                .lock()
                .expect("lock dos atalhos")
                .get(&event.id())
                .copied();
            if let Some(action) = action {
                // O receptor só some quando o app está encerrando.
                sender.try_send(action).ok();
            }
        }));
        Ok(Self {
            manager,
            bound: Vec::new(),
            actions,
        })
    }
}

impl Hotkeys for MacHotkeys {
    fn rebind(&mut self, bindings: &[(String, Action)]) -> Vec<String> {
        let mut errors = Vec::new();
        if let Err(error) = self.manager.unregister_all(&self.bound) {
            errors.push(format!("não consegui liberar os atalhos antigos: {error}"));
        }
        self.bound.clear();
        let mut actions = self.actions.lock().expect("lock dos atalhos");
        actions.clear();
        for (shortcut, action) in bindings {
            let registered = parse(shortcut).and_then(|hotkey| {
                self.manager.register(hotkey)?;
                Ok(hotkey)
            });
            match registered {
                Ok(hotkey) => {
                    actions.insert(hotkey.id(), *action);
                    self.bound.push(hotkey);
                }
                Err(error) => errors.push(format!("{shortcut}: {error}")),
            }
        }
        errors
    }

    fn modifiers_held(&self, shortcut: &str) -> Option<bool> {
        let Ok(hotkey) = parse(shortcut) else {
            return Some(false);
        };
        // Um evento vazio carrega os modificadores pressionados neste instante.
        let Ok(event) =
            CGEventSource::new(CGEventSourceStateID::CombinedSessionState).and_then(CGEvent::new)
        else {
            return Some(false);
        };
        let held = event.get_flags();
        let all_held = [
            (Modifiers::META, CGEventFlags::CGEventFlagCommand),
            (Modifiers::ALT, CGEventFlags::CGEventFlagAlternate),
            (Modifiers::CONTROL, CGEventFlags::CGEventFlagControl),
            (Modifiers::SHIFT, CGEventFlags::CGEventFlagShift),
        ]
        .into_iter()
        .all(|(modifier, flag)| !hotkey.mods.contains(modifier) || held.contains(flag));
        Some(all_held)
    }
}

fn parse(shortcut: &str) -> Result<HotKey> {
    Ok(HotKey::from_str(shortcut)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Resposta real do DDPM 2.3.0 para um Dell P3225QE.
    const DDPM_REPLY: &str = r#"{
    "Model": "P3225QE",
    "Command": "GET",
    "TargetFeature": "Capabilitiesstring",
    "Value": "(PROT(MONITOR)TYPE(LCD)MODEL(P3225QE)CMDS(01 02 03 07 0C E3 F3)VCP(02 04 05 08 10 12 14(05 08 0B 0C) 16 18 1A 52 60( 1B 0F 11) 87 AA(00 01 04 02) AC AE B2 B6 C6 C8 C9 CA CC(02 03 04 06 09 0A 0D 0E) D6(01 04 05) DC(00 03 05) DF E0(03) E2(00 02 04 0E 12 14) EA(F800 F801) F1 F2 FE FD)MCCS_VER(2.1)MSWHQL(1))",
    "Result": "PASS",
    "Message": "N/A"
}"#;

    #[test]
    fn lists_the_inputs_the_monitor_reports() {
        let inputs = ddpm_inputs(DDPM_REPLY).expect("entradas");
        assert_eq!(inputs, [InputCode(0x1b), InputCode(0x0f), InputCode(0x11)]);
    }

    #[test]
    fn a_failed_ddpm_reply_is_an_error() {
        let failed = DDPM_REPLY.replace("PASS", "FAIL");
        assert!(ddpm_inputs(&failed).is_err());
    }

    #[test]
    fn input_is_the_low_byte_of_the_m1ddc_value() {
        // 6939 = 0x1B1B, o que o P3225QE devolve em USB-C.
        assert_eq!(m1ddc_input("6939\n").expect("entrada"), InputCode(0x1b));
        assert!(m1ddc_input("Could not find a suitable external display.").is_err());
    }

    #[test]
    fn every_key_the_recorder_can_produce_is_a_valid_hotkey() {
        for key in [
            "1", "a", "tab", "space", "enter", "f5", "-", "[", "up", "pagedown", "home",
        ] {
            let shortcut = format!("ctrl+alt+shift+cmd+{key}");
            assert!(parse(&shortcut).is_ok(), "{shortcut}");
        }
    }
}
