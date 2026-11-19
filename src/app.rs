//! Estado do app e o que cada ação faz. Só conhece as portas, nunca um sistema específico.

use std::{path::PathBuf, time::Duration};

use gpui::{AnyWindowHandle, Context, WindowHandle};

use crate::{
    adapters::Platform,
    config,
    domain::{Config, Cycle, InputCode, Kvm, Outcome},
    ports::{Action, Hotkeys},
    ui::overlay::{self, HudView, SwitcherItem, SwitcherView},
};

/// Intervalo entre as checagens de "os modificadores do seletor ainda estão pressionados?".
const SWITCHER_POLL: Duration = Duration::from_millis(30);
/// Tempo que um aviso fica na tela.
const HUD_DURATION: Duration = Duration::from_millis(1800);

pub struct AppState {
    pub config: Config,
    /// Última mensagem de estado ou erro, mostrada na janela de configuração.
    pub status: String,
    /// Atalhos que o sistema recusou no último registro.
    pub hotkey_errors: Vec<String>,
    config_path: PathBuf,
    kvm: Kvm,
    hotkeys: Box<dyn Hotkeys>,
    switcher: Option<ActiveSwitcher>,
    hud: Option<AnyWindowHandle>,
}

struct ActiveSwitcher {
    cycle: Cycle,
    window: WindowHandle<SwitcherView>,
}

impl AppState {
    pub fn new(platform: Platform, config_path: PathBuf) -> Self {
        let (config, status) = match config::load(&config_path) {
            Ok(config) => (config, String::new()),
            Err(error) => (Config::default(), format!("{error:#}")),
        };
        let mut state = Self {
            config,
            status,
            hotkey_errors: Vec::new(),
            config_path,
            kvm: Kvm::new(platform.monitor, platform.peripherals),
            hotkeys: platform.hotkeys,
            switcher: None,
            hud: None,
        };
        state.rebind();
        state
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = config::save(&self.config_path, &self.config) {
            self.status = format!("{error:#}");
        }
        cx.notify();
    }

    /// Registra os atalhos da configuração atual.
    pub fn rebind(&mut self) {
        let mut bindings: Vec<(String, Action)> = self
            .config
            .targets
            .iter()
            .filter_map(|target| Some((target.hotkey.clone()?, Action::SwitchTo(target.input))))
            .collect();
        if let Some(shortcut) = &self.config.switcher_hotkey {
            bindings.push((shortcut.clone(), Action::SwitcherNext));
            // Como no ⌘Tab: o mesmo atalho com Shift anda para trás.
            if !shortcut.split('+').any(|part| part == "shift") {
                bindings.push((format!("shift+{shortcut}"), Action::SwitcherPrevious));
            }
        }
        self.hotkey_errors = self.hotkeys.rebind(&bindings);
    }

    /// Solta todos os atalhos, para que a gravação de um novo receba as teclas.
    pub fn suspend_hotkeys(&mut self) {
        self.hotkeys.rebind(&[]);
    }

    /// Lê as entradas do monitor e cria os destinos que faltam.
    pub fn detect(&mut self, cx: &mut Context<Self>) {
        self.status = "Lendo as entradas do monitor…".into();
        cx.notify();
        let kvm = self.kvm.clone();
        cx.spawn(async move |this, cx| {
            let detected = cx
                .background_executor()
                .spawn(async move { kvm.detect() })
                .await;
            this.update(cx, |this, cx| {
                match detected {
                    Ok(detected) => {
                        this.config.merge_inputs(&detected.inputs);
                        // A tela em que o app acabou de abrir é, quase sempre, a deste computador.
                        if this.config.local_input.is_none() {
                            // Logo após uma troca o monitor responde lixo: só vale entrada conhecida.
                            this.config.local_input = detected
                                .current
                                .filter(|current| detected.inputs.contains(current));
                        }
                        let current = detected
                            .current
                            .map_or("desconhecida".into(), InputCode::label);
                        this.status = format!(
                            "{} entradas no monitor; exibindo agora: {current}",
                            detected.inputs.len()
                        );
                    }
                    Err(error) => this.status = format!("{error:#}"),
                }
                this.save(cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn handle(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::SwitchTo(input) => self.switch_to(input, cx),
            Action::SwitcherNext => self.step_switcher(Cycle::advance, cx),
            Action::SwitcherPrevious => self.step_switcher(Cycle::back, cx),
        }
    }

    pub fn switch_to(&mut self, input: InputCode, cx: &mut Context<Self>) {
        // Cópia de propósito: a troca roda fora da thread da interface.
        let Some(target) = self.config.target(input).cloned() else {
            return;
        };
        let (kvm, local) = (self.kvm.clone(), self.config.local_input);
        let name = target.name.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { kvm.switch(&target, local) })
                .await;
            this.update(cx, |this, cx| match outcome {
                Ok(Outcome::Switched) => {
                    this.status = format!("Trocado para {name}");
                    cx.notify();
                }
                Ok(Outcome::AlreadyHere) => this.show_hud(format!("Você já está em {name}"), cx),
                Err(error) => this.show_hud(format!("{error:#}"), cx),
            })
            .ok();
        })
        .detach();
    }

    /// Abre o seletor, ou anda na seleção se ele já estiver aberto.
    fn step_switcher(&mut self, step: fn(&mut Cycle), cx: &mut Context<Self>) {
        if let Some(active) = &mut self.switcher {
            step(&mut active.cycle);
            let selected = active.cycle.selected();
            active
                .window
                .update(cx, |view, _, cx| view.select(selected, cx))
                .ok();
            return;
        }

        let Some(shortcut) = self.config.switcher_hotkey.clone() else {
            return;
        };
        let targets = &self.config.targets;
        let local = self.config.local_input;
        // Parte do computador atual, então o primeiro toque já cai no vizinho.
        let here = targets
            .iter()
            .position(|target| Some(target.input) == local);
        let Some(mut cycle) = Cycle::new(
            targets.len(),
            here.unwrap_or(targets.len().saturating_sub(1)),
        ) else {
            return;
        };
        step(&mut cycle);
        let items = targets
            .iter()
            .map(|target| SwitcherItem {
                icon: target.icon.clone().into(),
                name: target.name.clone().into(),
                port: target.input.label().into(),
                is_local: Some(target.input) == local,
            })
            .collect();
        let Some(window) = overlay::open_switcher(items, cycle.selected(), cx) else {
            return;
        };
        self.switcher = Some(ActiveSwitcher { cycle, window });

        // Soltar os modificadores confirma. A checagem é por estado do teclado, e não
        // por evento da janela, porque o seletor não rouba o foco do app em uso.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SWITCHER_POLL).await;
                let open = this.update(cx, |this, cx| {
                    if this.switcher.is_some() && !this.hotkeys.modifiers_held(&shortcut) {
                        this.commit_switcher(cx);
                    }
                    this.switcher.is_some()
                });
                if !open.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    fn commit_switcher(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.switcher.take() else {
            return;
        };
        overlay::close(active.window.into(), cx);
        if let Some(target) = self.config.targets.get(active.cycle.selected()) {
            self.switch_to(target.input, cx);
        }
    }

    fn show_hud(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = message.clone();
        cx.notify();
        if let Some(previous) = self.hud.take() {
            overlay::close(previous, cx);
        }
        let Some(window) = overlay::open_hud(
            HudView {
                message: message.into(),
            },
            cx,
        ) else {
            return;
        };
        self.hud = Some(window);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HUD_DURATION).await;
            this.update(cx, |this, cx| {
                // Um aviso mais novo pode ter ocupado o lugar deste.
                if this.hud == Some(window) {
                    this.hud = None;
                    overlay::close(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}
