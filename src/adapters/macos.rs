//! macOS: monitor por `m1ddc` (e DDPM para listar entradas), atalhos globais
//! pelo Carbon, via `global-hotkey`, o ícone na barra de menus pelo `tray-icon`
//! e a posição do cursor, para o modo Bordas, pelo Quartz.

use std::{
    collections::HashMap,
    fs,
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result, bail};
use async_channel::Sender;
use core_graphics::{
    display::CGDisplay,
    event::{CGEvent, CGEventFlags},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{HotKey, Modifiers},
};

use tray_icon::{TrayIcon, TrayIconBuilder, menu::MenuEvent};

use super::{mccs::capability_inputs, run};
use crate::{
    domain::{Config, InputCode, Rect},
    ports::{Action, Hotkeys, Monitor, StatusMenu},
    request, tray,
};

mod clipboard;
#[expect(unused_imports, reason = "usado a partir da ligação na Task 7")]
pub use clipboard::MacClipboard;

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

/// Posição do cursor e retângulos das telas ativas, no espaço global do Quartz
/// (origem no canto de cima da tela principal, y para baixo).
pub fn cursor_and_screens() -> Option<(f64, f64, Vec<Rect>)> {
    let at = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .and_then(CGEvent::new)
        .ok()?
        .location();
    let screens = CGDisplay::active_displays()
        .ok()?
        .into_iter()
        .map(|id| {
            let bounds = CGDisplay::new(id).bounds();
            Rect {
                x: bounds.origin.x,
                y: bounds.origin.y,
                width: bounds.size.width,
                height: bounds.size.height,
            }
        })
        .collect();
    Some((at.x, at.y, screens))
}

/// Ícone na barra de menus (NSStatusItem). Criado na thread principal com o app
/// já iniciado, dentro de `Application::run`, como o `tray-icon` exige.
pub struct MacStatusMenu {
    icon: TrayIcon,
}

impl MacStatusMenu {
    pub fn new(actions: Sender<Action>) -> Result<Self> {
        let icon = TrayIconBuilder::new()
            .with_id("kvm-switch")
            .with_tooltip("KVM Switch")
            .with_icon(tray::icon()?)
            .build()
            .context("não consegui criar o ícone na barra de menus")?;
        // O id de cada item é o pedido em texto (veja `tray::entries`).
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Ok(action) = request::parse(&event.id.0) {
                actions.try_send(action).ok();
            }
        }));
        Ok(Self { icon })
    }
}

impl StatusMenu for MacStatusMenu {
    fn show(&mut self, config: &Config) {
        match tray::menu(config) {
            Ok(menu) => self.icon.set_menu(Some(Box::new(menu))),
            Err(error) => eprintln!("kvm-switch: {error:#}"),
        }
    }
}

/// Identificador do app, o mesmo do `Info.plist` gerado pelo `macos/bundle.sh`.
const BUNDLE_ID: &str = "io.github.mpgxc.kvm-switch";

/// Faz o app abrir sozinho no login, sem janela (`start`), porque o modo Bordas só
/// funciona com ele aberto. É um LaunchAgent que roda `open -g -a <app> --args start`:
/// o `open` passa pelo LaunchServices, então segue havendo um só app aberto, com
/// ícone no Dock, e funciona com a assinatura ad hoc do `bundle.sh` sem outra
/// dependência (o `SMAppService` pediria bindings de Objective-C).
// shortcut: não há opção para desligar na janela; quem não quiser desliga o item
// em Ajustes → Geral → Itens de Início. Criar a opção se alguém pedir.
pub fn register_login_item() -> Result<()> {
    let home = dirs::home_dir().context("pasta pessoal desconhecida")?;
    let exe = std::env::current_exe().context("caminho do executável desconhecido")?;
    write_login_item(&exe, &home)
}

/// O trabalho de `register_login_item`, com os caminhos de fora para o teste.
fn write_login_item(exe: &Path, home: &Path) -> Result<()> {
    let Some(bundle) = installed_bundle(exe, home) else {
        return Ok(());
    };
    let bundle = bundle.to_str().context("o caminho do app não é UTF-8")?;
    let folder = home.join("Library/LaunchAgents");
    let path = folder.join(format!("{BUNDLE_ID}.plist"));
    let plist = login_item_plist(bundle);
    // Só escreve se mudou (outro lugar do app ou primeira vez): o arquivo fica
    // intocado nas aberturas seguintes.
    if fs::read_to_string(&path).is_ok_and(|current| current == plist) {
        return Ok(());
    }
    fs::create_dir_all(&folder)
        .with_context(|| format!("não consegui criar {}", folder.display()))?;
    fs::write(&path, plist).with_context(|| format!("não consegui gravar {}", path.display()))
}

/// O `.app` em que o executável roda, se estiver instalado em `/Applications` ou
/// `~/Applications`. Fora disso (`cargo run`, o `.app` em `target/`, o aberto direto
/// do .dmg ou transladado pelo Gatekeeper) o caminho não serve para o próximo login.
fn installed_bundle<'a>(exe: &'a Path, home: &Path) -> Option<&'a Path> {
    let bundle = exe.parent()?.parent()?.parent()?;
    let folder = bundle.parent()?;
    let installed = folder == Path::new("/Applications") || folder == home.join("Applications");
    (bundle.extension()? == "app" && installed).then_some(bundle)
}

/// O LaunchAgent que abre o app em `bundle` no login, em segundo plano e sem janela.
fn login_item_plist(bundle: &str) -> String {
    let bundle = bundle
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{BUNDLE_ID}</string>
  <key>AssociatedBundleIdentifiers</key><string>{BUNDLE_ID}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-g</string>
    <string>-a</string>
    <string>{bundle}</string>
    <string>--args</string>
    <string>start</string>
  </array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#
    )
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

    #[test]
    fn only_an_installed_app_becomes_a_login_item() {
        let home = Path::new("/Users/eu");
        let exe = |path: &str| Path::new(path).join("KVM Switch.app/Contents/MacOS/kvm-switch");
        assert_eq!(
            installed_bundle(&exe("/Applications"), home),
            Some(Path::new("/Applications/KVM Switch.app"))
        );
        assert_eq!(
            installed_bundle(&exe("/Users/eu/Applications"), home),
            Some(Path::new("/Users/eu/Applications/KVM Switch.app"))
        );
        assert_eq!(installed_bundle(&exe("/Volumes/KVM Switch"), home), None);
        assert_eq!(
            installed_bundle(&exe("/Users/eu/kvm-switch/target/release"), home),
            None
        );
        let cargo_run = Path::new("/Users/eu/kvm-switch/target/debug/kvm-switch");
        assert_eq!(installed_bundle(cargo_run, home), None);
    }

    #[test]
    fn login_item_is_a_valid_launch_agent_that_starts_without_a_window() {
        let home =
            std::env::temp_dir().join(format!("kvm-switch-test-{}-login-item", std::process::id()));
        let exe = home.join("Applications/K&V <M>.app/Contents/MacOS/kvm-switch");
        write_login_item(&exe, &home).expect("gravar o item de login");
        let path = home.join("Library/LaunchAgents/io.github.mpgxc.kvm-switch.plist");
        let lint = std::process::Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(&path)
            .output()
            .expect("rodar o plutil");
        let plist = fs::read_to_string(&path).expect("ler o item de login");
        fs::remove_dir_all(&home).ok();
        assert!(
            lint.status.success(),
            "{}",
            String::from_utf8_lossy(&lint.stdout)
        );
        assert!(plist.contains("<string>start</string>"));
        assert!(plist.contains("/Applications/K&amp;V &lt;M&gt;.app</string>"));
    }
}
