//! Windows: DDC/CI nativo, receptor Bolt por HIDAPI e atalhos globais.

use std::{
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail};
use async_channel::Sender;
use ddc_hi::{Ddc, Display};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, Modifiers},
};
use hidapi::{HidApi, HidDevice};
use tray_icon::{TrayIcon, TrayIconBuilder, menu::MenuEvent};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

use super::mccs::capability_inputs;
use crate::{
    domain::{Channel, Config, InputCode},
    ports::{Action, Hotkeys, Monitor, Peripherals, StatusMenu},
    request, tray,
};

/// Feature MCCS que seleciona a entrada de vídeo do monitor.
const INPUT_SOURCE: u8 = 0x60;
/// Vendor e product ID do receptor Logi Bolt.
const RECEIVER: (u16, u16) = (0x046d, 0xc548);
/// Página de uso e uso da interface HID++ do receptor Bolt.
const HIDPP_USAGE: (u16, u16) = (0xff00, 0x0002);
/// Quantos periféricos um receptor Logitech pode parear.
const SLOTS: u8 = 6;
/// Tamanho dos relatórios longos HID++.
const REPORT_LEN: usize = 20;
/// Tempo máximo para um periférico responder, inclusive se estiver em repouso.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1100);

/// Monitor que responde a DDC/CI pelo Windows Monitor Configuration API.
pub struct WindowsMonitor;

impl Monitor for WindowsMonitor {
    fn inputs(&self) -> Result<Vec<InputCode>> {
        let mut display = find_monitor()?;
        let capabilities = display
            .handle
            .capabilities_string()
            .context("não consegui ler as entradas informadas pelo monitor")?;
        capability_inputs(&String::from_utf8_lossy(&capabilities))
    }

    fn current_input(&self) -> Result<InputCode> {
        let mut display = find_monitor()?;
        let value = display
            .handle
            .get_vcp_feature(INPUT_SOURCE)
            .context("não consegui ler a entrada atual do monitor")?;
        Ok(InputCode(value.sl))
    }

    fn set_input(&self, input: InputCode) -> Result<()> {
        let mut display = find_monitor()?;
        display
            .handle
            .set_vcp_feature(INPUT_SOURCE, u16::from(input.0))
            .context("não consegui trocar a entrada do monitor")
    }
}

/// Encontra a primeira tela conectada que responde ao VCP 0x60.
fn find_monitor() -> Result<Display> {
    let mut last_error = None;
    for mut display in Display::enumerate() {
        // Telas internas e monitores sem DDC/CI podem aparecer na enumeração; o primeiro
        // que responde à feature de entrada é o monitor que o app consegue controlar.
        match display.handle.get_vcp_feature(INPUT_SOURCE) {
            Ok(_) => return Ok(display),
            Err(error) => last_error = Some(error),
        }
    }
    if let Some(error) = last_error {
        return Err(error).context(
            "nenhum monitor respondeu por DDC/CI; confira o cabo e ative DDC/CI no monitor",
        );
    }
    bail!("nenhum monitor DDC/CI foi encontrado; confira o cabo e ative DDC/CI no monitor")
}

/// Teclado e mouse Logitech pelo protocolo HID++ 2.0 do receptor Bolt.
pub struct LogiReceiver;

impl Peripherals for LogiReceiver {
    fn switch_to(&self, channel: Channel) -> Result<()> {
        let api = HidApi::new().context("não consegui iniciar a comunicação HID do Windows")?;
        let receivers = api
            .device_list()
            .filter(|device| {
                (device.vendor_id(), device.product_id()) == RECEIVER
                    && (device.usage_page(), device.usage()) == HIDPP_USAGE
            })
            .map(|device| {
                api.open_path(device.path())
                    .context("não consegui abrir o receptor Logitech Bolt")
            })
            .collect::<Result<Vec<_>>>()?;
        if receivers.is_empty() {
            bail!("nenhum receptor Logitech Bolt (046D:C548) nesta máquina");
        }

        let mut moved = 0;
        for receiver in &receivers {
            for slot in 1..=SLOTS {
                if let Some(index) = change_host_index(receiver, slot)? {
                    set_current_host(receiver, slot, index, channel)?;
                    moved += 1;
                }
            }
        }
        if moved == 0 {
            bail!("nenhum periférico Logitech respondeu neste receptor Bolt");
        }
        Ok(())
    }
}

/// Pergunta ao slot o índice da feature CHANGE_HOST e devolve-o se houver resposta.
fn change_host_index(device: &HidDevice, slot: u8) -> Result<Option<u8>> {
    let report = long_report(&[slot, 0x00, 0x0e, 0x18, 0x14]);
    send_report(device, &report)?;

    let deadline = Instant::now() + REPLY_TIMEOUT;
    let mut reply = [0; 64];
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout = i32::try_from(remaining.as_millis())
            .unwrap_or(i32::MAX)
            .max(1);
        let len = device
            .read_timeout(&mut reply, timeout)
            .context("não consegui ler a resposta do receptor Logitech Bolt")?;
        if len == 0 {
            return Ok(None);
        }
        match parse_change_host_reply(&reply[..len], slot) {
            FeatureReply::Index(index) => return Ok(Some(index)),
            FeatureReply::NoDevice => return Ok(None),
            FeatureReply::Unrelated => {}
        }
    }
    Ok(None)
}

/// Envia CHANGE_HOST.setCurrentHost para o periférico encontrado no slot.
fn set_current_host(device: &HidDevice, slot: u8, index: u8, channel: Channel) -> Result<()> {
    let report = long_report(&[slot, index, 0x1e, channel.host_index()]);
    send_report(device, &report)
}

/// Monta um relatório HID++ longo de 20 bytes.
fn long_report(payload: &[u8]) -> [u8; REPORT_LEN] {
    let mut report = [0; REPORT_LEN];
    report[0] = 0x11;
    report[1..=payload.len()].copy_from_slice(payload);
    report
}

/// Escreve um relatório completo e trata escrita parcial como erro.
fn send_report(device: &HidDevice, report: &[u8; REPORT_LEN]) -> Result<()> {
    let written = device
        .write(report)
        .context("o receptor Logitech Bolt não aceitou o comando HID++")?;
    if written != report.len() {
        bail!(
            "o receptor Logitech Bolt recebeu apenas {written} de {} bytes",
            report.len()
        );
    }
    Ok(())
}

/// Resposta da consulta CHANGE_HOST para um slot.
#[derive(Debug, PartialEq, Eq)]
enum FeatureReply {
    /// O índice da feature CHANGE_HOST no periférico.
    Index(u8),
    /// O slot está vazio ou o periférico não implementa a feature.
    NoDevice,
    /// Resposta de outro slot ou de outra conversa HID++.
    Unrelated,
}

/// Interpreta a resposta HID++ da consulta CHANGE_HOST.
fn parse_change_host_reply(report: &[u8], slot: u8) -> FeatureReply {
    match report {
        [0x11, from, 0x00, 0x0e, 0x18, 0x14] if *from == slot => FeatureReply::Unrelated,
        [0x11, from, 0x00, 0x0e, index, ..] if *from == slot => match index {
            0 => FeatureReply::NoDevice,
            index => FeatureReply::Index(*index),
        },
        [_, from, 0x8f | 0xff, 0x00, 0x0e, ..] if *from == slot => FeatureReply::NoDevice,
        _ => FeatureReply::Unrelated,
    }
}

/// Atalhos globais no loop de mensagens Win32 que também hospeda o GPUI.
pub struct WindowsHotkeys {
    manager: GlobalHotKeyManager,
    bound: Vec<HotKey>,
    actions: Arc<Mutex<std::collections::HashMap<u32, Action>>>,
}

impl WindowsHotkeys {
    /// Registra o callback na thread principal, onde roda o loop de mensagens do Windows.
    pub fn new(sender: Sender<Action>) -> Result<Self> {
        let manager =
            GlobalHotKeyManager::new().context("não consegui iniciar os atalhos globais")?;
        let actions: Arc<Mutex<std::collections::HashMap<u32, Action>>> = Arc::default();
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

impl Hotkeys for WindowsHotkeys {
    fn rebind(&mut self, bindings: &[(String, Action)]) -> Vec<String> {
        let mut errors = Vec::new();
        if let Err(error) = self.manager.unregister_all(&self.bound) {
            errors.push(format!("não consegui liberar os atalhos antigos: {error}"));
        }
        self.bound.clear();
        let mut actions = self.actions.lock().expect("lock dos atalhos");
        actions.clear();
        for (shortcut, action) in bindings {
            let registered = parse_hotkey(shortcut).and_then(|hotkey| {
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
        let Ok(hotkey) = parse_hotkey(shortcut) else {
            return Some(false);
        };
        let held = [
            (Modifiers::META, key_held(VK_LWIN) || key_held(VK_RWIN)),
            (Modifiers::ALT, key_held(VK_MENU)),
            (Modifiers::CONTROL, key_held(VK_CONTROL)),
            (Modifiers::SHIFT, key_held(VK_SHIFT)),
        ];
        Some(
            held.into_iter()
                .all(|(modifier, is_held)| !hotkey.mods.contains(modifier) || is_held),
        )
    }
}

/// Consulta o estado atual de uma tecla virtual do Windows.
fn key_held(key: u16) -> bool {
    // SAFETY: GetAsyncKeyState não recebe ponteiros; a tecla virtual é validada pelo tipo.
    unsafe { GetAsyncKeyState(i32::from(key)) < 0 }
}

/// Converte o texto comum dos atalhos para o formato do global-hotkey.
fn parse_hotkey(shortcut: &str) -> Result<HotKey> {
    Ok(HotKey::from_str(shortcut)?)
}

/// Ícone da área de notificação do Windows e seu menu de destinos.
pub struct WindowsStatusMenu {
    icon: TrayIcon,
}

impl WindowsStatusMenu {
    /// Cria o ícone no loop de mensagens da thread principal do app.
    pub fn new(actions: Sender<Action>) -> Result<Self> {
        let icon = TrayIconBuilder::new()
            .with_id("kvm-switch")
            .with_tooltip("KVM Switch")
            .with_icon(tray::icon()?)
            .build()
            .context("não consegui criar o ícone na área de notificação")?;
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Ok(action) = request::parse(&event.id.0) {
                actions.try_send(action).ok();
            }
        }));
        Ok(Self { icon })
    }
}

impl StatusMenu for WindowsStatusMenu {
    fn show(&mut self, config: &Config) {
        match tray::menu(config) {
            Ok(menu) => self.icon.set_menu(Some(Box::new(menu))),
            Err(error) => eprintln!("kvm-switch: {error:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_feature_index_in_a_long_hidpp_reply() {
        let reply = [0x11, 0x01, 0x00, 0x0e, 0x09, 0x00, 0x02];
        assert_eq!(
            parse_change_host_reply(&reply, 1),
            FeatureReply::Index(0x09)
        );
    }

    #[test]
    fn ignores_echoes_empty_slots_and_other_devices() {
        let echo = [0x11, 0x01, 0x00, 0x0e, 0x18, 0x14];
        let empty_slot = [0x10, 0x02, 0x8f, 0x00, 0x0e, 0x09];
        let other_slot = [0x11, 0x02, 0x00, 0x0e, 0x09];
        assert_eq!(parse_change_host_reply(&echo, 1), FeatureReply::Unrelated);
        assert_eq!(
            parse_change_host_reply(&empty_slot, 2),
            FeatureReply::NoDevice
        );
        assert_eq!(
            parse_change_host_reply(&other_slot, 1),
            FeatureReply::Unrelated
        );
    }

    #[test]
    fn accepts_the_shortcuts_used_by_the_settings_view_on_windows() {
        for key in ["1", "a", "tab", "space", "enter", "f5", "-", "[", "up"] {
            let shortcut = format!("ctrl+alt+shift+super+{key}");
            assert!(parse_hotkey(&shortcut).is_ok(), "{shortcut}");
        }
    }
}
