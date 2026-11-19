//! Estado do app e o que cada ação faz. Só conhece as portas, nunca um sistema específico.

use std::{
    path::PathBuf,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use gpui::{AnyWindowHandle, Context, Modifiers, WindowHandle};

use crate::{
    adapters::{LOCAL_ICON, Platform},
    config,
    domain::{Config, Cycle, InputCode, Kvm, Outcome},
    ports::{Action, Hotkeys},
    ui::{
        overlay::{self, Focus, HudView, SwitcherItem, SwitcherView},
        settings,
    },
};

/// Intervalo entre as checagens de "os modificadores do seletor ainda estão pressionados?".
const SWITCHER_POLL: Duration = Duration::from_millis(30);
/// Sem como ver o teclado, o seletor confirma depois deste tempo sem toques.
const SWITCHER_IDLE: Duration = Duration::from_millis(1200);
/// Tempo que um aviso fica na tela.
const HUD_DURATION: Duration = Duration::from_millis(1800);

/// Quantas trocas estão em andamento. No Linux o GPUI encerra o app quando a
/// última janela fecha, e sair no meio de uma troca deixaria o monitor numa
/// máquina e o teclado em outra: `main` espera por [`finish_switches`] antes de sair.
static IN_FLIGHT: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

/// Marca uma troca em andamento enquanto existir.
struct InFlight;

impl InFlight {
    fn begin() -> Self {
        *IN_FLIGHT.0.lock().expect("lock das trocas envenenado") += 1;
        Self
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        *IN_FLIGHT.0.lock().expect("lock das trocas envenenado") -= 1;
        IN_FLIGHT.1.notify_all();
    }
}

/// Espera as trocas em andamento terminarem.
pub fn finish_switches() {
    let mut running = IN_FLIGHT.0.lock().expect("lock das trocas envenenado");
    while *running > 0 {
        running = IN_FLIGHT
            .1
            .wait(running)
            .expect("lock das trocas envenenado");
    }
}

/// O aviso que o resultado de uma troca merece. `None`: ela só aconteceu.
pub fn notice(outcome: &Result<Outcome>, name: &str) -> Option<String> {
    match outcome {
        Ok(Outcome::Switched) => None,
        Ok(Outcome::AlreadyHere) => Some(format!("Você já está em {name}")),
        Err(error) => Some(format!("{error:#}")),
    }
}

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
    /// Entrada de cada destino mostrado, na ordem do painel; `cycle` indexa esta lista,
    /// não a de todos os destinos.
    inputs: Vec<InputCode>,
    cycle: Cycle,
    window: WindowHandle<SwitcherView>,
    /// Quando o atalho do seletor foi tocado pela última vez.
    stepped: Instant,
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
                        // Logo após uma troca o monitor responde lixo: só vale entrada conhecida.
                        if let Some(current) = detected
                            .current
                            .filter(|current| detected.inputs.contains(current))
                        {
                            this.config.adopt_local(current, LOCAL_ICON);
                        }
                        let current = detected
                            .current
                            .map_or_else(|| "desconhecida".into(), InputCode::label);
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
            // Adiado: a janela lê este estado ao ser montada, e ele está em uso agora.
            Action::OpenSettings => cx.defer(settings::open),
        }
    }

    pub fn switch_to(&mut self, input: InputCode, cx: &mut Context<Self>) {
        self.switch_then_close(input, None, cx);
    }

    /// Troca para `input` e só então fecha `finished`, a janela do seletor. Ela
    /// fecha depois de um eventual aviso abrir, para o app nunca ficar sem janela
    /// antes de mostrar o aviso.
    fn switch_then_close(
        &mut self,
        input: InputCode,
        finished: Option<AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) {
        // Cópia de propósito: a troca roda fora da thread da interface.
        let Some(target) = self.config.target(input).cloned() else {
            if let Some(window) = finished {
                overlay::close(window, cx);
            }
            return;
        };
        let (kvm, local) = (self.kvm.clone(), self.config.local_input);
        let name = target.name.clone();
        // A troca roda solta e devolve o resultado por um canal: ela vai até o fim
        // mesmo que a interface feche no meio do caminho.
        let (done, outcome) = async_channel::bounded(1);
        let in_flight = InFlight::begin();
        cx.background_executor()
            .spawn(async move {
                let _in_flight = in_flight;
                done.try_send(kvm.switch(&target, local)).ok();
            })
            .detach();
        cx.spawn(async move |this, cx| {
            // O canal só fecha sem resposta se a troca foi descartada com o app encerrando.
            let outcome = outcome.recv().await;
            this.update(cx, |this, cx| {
                if let Ok(outcome) = outcome {
                    match notice(&outcome, &name) {
                        Some(message) => this.show_hud(message, cx),
                        None => {
                            this.status = format!("Trocado para {name}");
                            cx.notify();
                        }
                    }
                }
                if let Some(window) = finished {
                    overlay::close(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Abre o seletor, ou anda na seleção se ele já estiver aberto.
    fn step_switcher(&mut self, step: impl FnOnce(&mut Cycle), cx: &mut Context<Self>) {
        if let Some(active) = &mut self.switcher {
            step(&mut active.cycle);
            active.stepped = Instant::now();
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
        let local = self.config.local_input;
        // Só os destinos ativos, partindo do computador atual: o primeiro toque já cai no vizinho.
        let Some((targets, mut cycle)) = self.config.switcher() else {
            return;
        };
        step(&mut cycle);
        let inputs = targets.iter().map(|target| target.input).collect();
        let items = targets
            .iter()
            .map(|target| SwitcherItem {
                icon: target.icon.clone().into(),
                name: target.name.clone().into(),
                port: target.input.label().into(),
                is_local: Some(target.input) == local,
            })
            .collect();
        // Onde o sistema só mostra o teclado à janela em foco, o seletor precisa do
        // foco para saber quando os modificadores foram soltos.
        let focus = match self.hotkeys.modifiers_held(&shortcut) {
            None => Focus::Take,
            Some(_) => Focus::Leave,
        };
        let Some(window) = overlay::open_switcher(items, cycle.selected(), focus, cx) else {
            return;
        };
        self.switcher = Some(ActiveSwitcher {
            inputs,
            cycle,
            window,
            stepped: Instant::now(),
        });

        // Soltar os modificadores confirma. A checagem é periódica, e não por evento
        // da janela, porque no macOS o seletor não rouba o foco do app em uso.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SWITCHER_POLL).await;
                let open = this.update(cx, |this, cx| {
                    if this.switcher_released(&shortcut, cx) {
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

    /// O usuário soltou os modificadores do atalho do seletor?
    fn switcher_released(&self, shortcut: &str, cx: &mut Context<Self>) -> bool {
        let Some(active) = &self.switcher else {
            return false;
        };
        if let Some(held) = self.hotkeys.modifiers_held(shortcut) {
            return !held;
        }
        let seen = active
            .window
            .update(cx, |_, window, _| {
                window.is_window_active().then(|| window.modifiers())
            })
            .ok()
            .flatten();
        match seen {
            Some(held) => !holds(held, shortcut),
            // A janela não recebeu o foco: não há como ver o teclado.
            None => active.stepped.elapsed() > SWITCHER_IDLE,
        }
    }

    fn commit_switcher(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.switcher.take() else {
            return;
        };
        let window = active.window.into();
        match active.inputs.get(active.cycle.selected()) {
            Some(&input) => self.switch_then_close(input, Some(window), cx),
            None => overlay::close(window, cx),
        }
    }

    pub fn show_hud(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = message.clone();
        cx.notify();
        let previous = self.hud.take();
        let opened = overlay::open_hud(
            HudView {
                message: message.into(),
            },
            cx,
        );
        // O aviso anterior só fecha com o novo já aberto: no Linux o app encerra
        // quando fica sem janelas.
        if let Some(previous) = previous {
            overlay::close(previous, cx);
        }
        let Some(window) = opened else {
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

/// Os modificadores do atalho estão todos em `held`?
fn holds(held: Modifiers, shortcut: &str) -> bool {
    shortcut.split('+').all(|part| match part {
        "ctrl" => held.control,
        "alt" => held.alt,
        "shift" => held.shift,
        "cmd" | "super" => held.platform,
        _ => true,
    })
}
