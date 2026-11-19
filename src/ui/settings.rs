//! Janela de configuração: destinos, nomes, ícones, canais e atalhos.

use gpui::{
    App, AppContext as _, Bounds, Context, Div, ElementId, Entity, FocusHandle, Global,
    KeyDownEvent, Keystroke, MouseButton, MouseDownEvent, Render, SharedString, Stateful,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, div, prelude::*, px, rgb,
    size,
};

use super::theme;
use crate::{
    app::AppState,
    domain::{Channel, InputCode, Target},
};

/// Ícones oferecidos para um destino; cada clique passa para o próximo.
const ICONS: [&str; 6] = ["🖥", "💻", "🍎", "🐧", "🪟", "🎮"];

const NAME_WIDTH: f32 = 180.;
const PORT_WIDTH: f32 = 70.;
const LOCAL_WIDTH: f32 = 170.;
const CHANNEL_WIDTH: f32 = 30.;
const HOTKEY_WIDTH: f32 = 170.;

/// O que o app inteiro compartilha: o estado e a janela de configuração aberta.
pub struct Shared {
    state: Entity<AppState>,
    settings: Option<WindowHandle<SettingsView>>,
}

impl Shared {
    pub fn new(state: Entity<AppState>) -> Self {
        Self {
            state,
            settings: None,
        }
    }
}

impl Global for Shared {}

/// Abre a janela de configuração, ou traz para a frente a que já existe.
pub fn open(cx: &mut App) {
    let Some(shared) = cx.try_global::<Shared>() else {
        return;
    };
    let (state, existing) = (shared.state.clone(), shared.settings);
    if let Some(window) = existing
        && window
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        cx.activate(true);
        return;
    }

    let bounds = Bounds::centered(None, size(px(960.), px(440.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("KVM Switch".into()),
            ..Default::default()
        }),
        app_id: Some(super::APP_ID.into()),
        ..Default::default()
    };
    let window = cx.open_window(options, |window, cx| {
        cx.new(|cx| SettingsView::new(state, window, cx))
    });
    cx.global_mut::<Shared>().settings = window.ok();
    cx.activate(true);
}

/// Campo que está recebendo o teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name(usize),
    Hotkey(usize),
    SwitcherHotkey,
}

impl Field {
    fn id(self) -> ElementId {
        match self {
            Field::Name(index) => ("name", index).into(),
            Field::Hotkey(index) => ("hotkey", index).into(),
            Field::SwitcherHotkey => "switcher-hotkey".into(),
        }
    }

    fn records_hotkey(self) -> bool {
        !matches!(self, Field::Name(_))
    }
}

pub struct SettingsView {
    state: Entity<AppState>,
    focus: FocusHandle,
    editing: Option<Field>,
}

impl SettingsView {
    fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        // O teclado chega à raiz da janela, que repassa ao campo em edição.
        window.focus(&focus);
        Self {
            state,
            focus,
            editing: None,
        }
    }

    fn start_editing(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_editing(cx);
        if field.records_hotkey() {
            // Com os atalhos registrados, o sistema entregaria a tecla a eles e não a esta janela.
            self.state.update(cx, |state, _| state.suspend_hotkeys());
        }
        self.editing = Some(field);
        window.focus(&self.focus);
        cx.notify();
    }

    fn stop_editing(&mut self, cx: &mut Context<Self>) {
        if self.editing.take().is_some_and(Field::records_hotkey) {
            self.state.update(cx, |state, cx| {
                state.rebind();
                cx.notify();
            });
        }
        cx.notify();
    }

    fn edit_target(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Target),
    ) {
        self.state.update(cx, |state, cx| {
            if let Some(target) = state.config.targets.get_mut(index) {
                edit(target);
            }
            state.save(cx);
        });
    }

    fn set_hotkey(&mut self, field: Field, shortcut: Option<String>, cx: &mut Context<Self>) {
        match field {
            Field::Hotkey(index) => self.edit_target(index, cx, |target| target.hotkey = shortcut),
            Field::SwitcherHotkey => self.state.update(cx, |state, cx| {
                state.config.switcher_hotkey = shortcut;
                state.save(cx);
            }),
            Field::Name(_) => {}
        }
        self.stop_editing(cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.editing else { return };
        // A tecla foi consumida pelo campo: não segue para o sistema de entrada de texto.
        cx.stop_propagation();
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let chord = modifiers.control || modifiers.alt || modifiers.platform;

        if let Field::Name(index) = field {
            // shortcut: sem cursor, seleção nem composição de acentos; trocar por um
            // campo de texto completo quando os nomes precisarem de mais que digitar e apagar.
            match keystroke.key.as_str() {
                "enter" | "escape" | "tab" => self.stop_editing(cx),
                "backspace" => self.edit_target(index, cx, |target| {
                    target.name.pop();
                }),
                _ => {
                    if let Some(text) = &keystroke.key_char
                        && !chord
                    {
                        self.edit_target(index, cx, |target| target.name.push_str(text));
                    }
                }
            }
            return;
        }

        if chord {
            self.set_hotkey(field, Some(shortcut_from(keystroke)), cx);
        } else {
            match keystroke.key.as_str() {
                "escape" => self.stop_editing(cx),
                "backspace" | "delete" => self.set_hotkey(field, None, cx),
                // Tecla sozinha não vira atalho global: tomaria a tecla de todos os apps.
                _ => {}
            }
        }
    }

    fn field(
        &self,
        field: Field,
        text: String,
        placeholder: &'static str,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let editing = self.editing == Some(field);
        let (label, muted) = match (editing, field) {
            (true, Field::Name(_)) => (format!("{text}▏"), false),
            (true, _) => ("pressione o atalho…".to_string(), true),
            (false, _) if text.is_empty() => (placeholder.to_string(), true),
            (false, _) => (text, false),
        };
        div()
            .id(field.id())
            .debug_selector(move || format!("{field:?}"))
            .h(px(30.))
            .px_2()
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(rgb(if editing {
                theme::ACCENT
            } else {
                theme::BORDER
            }))
            .bg(rgb(theme::FIELD))
            .cursor_pointer()
            .when(muted, |field| field.text_color(rgb(theme::MUTED)))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.start_editing(field, window, cx)))
    }

    fn row(
        &self,
        index: usize,
        target: &Target,
        local: Option<InputCode>,
        cx: &Context<Self>,
    ) -> Div {
        let input = target.input;
        let local_mark = if local == Some(input) {
            "● este computador"
        } else {
            "○ este computador"
        };
        let channels = [
            None,
            Some(Channel::One),
            Some(Channel::Two),
            Some(Channel::Three),
        ];
        let channel_buttons = channels.into_iter().enumerate().map(|(position, channel)| {
            let label = channel.map_or("–".to_string(), |channel| channel.number().to_string());
            button(("channel", index * channels.len() + position), label)
                .w(px(CHANNEL_WIDTH))
                .when(target.channel == channel, |button| {
                    button.bg(rgb(theme::ACCENT))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit_target(index, cx, |target| target.channel = channel);
                }))
        });

        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                button(("icon", index), target.icon.clone())
                    .w(px(44.))
                    .px_0()
                    .text_xl()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_target(index, cx, |target| target.icon = next_icon(&target.icon))
                    })),
            )
            .child(
                self.field(Field::Name(index), target.name.clone(), "nome", cx)
                    .w(px(NAME_WIDTH)),
            )
            .child(div().w(px(PORT_WIDTH)).child(input.label()))
            .child(
                div()
                    .id(("local", index))
                    .w(px(LOCAL_WIDTH))
                    .cursor_pointer()
                    .text_color(rgb(if local == Some(input) {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    }))
                    .child(local_mark)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.update(cx, |state, cx| {
                            state.config.local_input = Some(input);
                            state.save(cx);
                        });
                    })),
            )
            .child(div().flex().gap_1().children(channel_buttons))
            .child(
                self.field(
                    Field::Hotkey(index),
                    pretty(target.hotkey.as_deref()),
                    "definir atalho",
                    cx,
                )
                .w(px(HOTKEY_WIDTH)),
            )
            .child(
                button(("go", index), "Ir").on_click(cx.listener(move |this, _, _, cx| {
                    this.state
                        .update(cx, |state, cx| state.switch_to(input, cx));
                })),
            )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let config = &state.config;
        let rows: Vec<Div> = config
            .targets
            .iter()
            .enumerate()
            .map(|(index, target)| self.row(index, target, config.local_input, cx))
            .collect();
        let errors: Vec<Div> = state
            .hotkey_errors
            .iter()
            .map(|error| {
                div()
                    .text_color(rgb(theme::ERROR))
                    .child(format!("Atalho recusado: {error}"))
            })
            .collect();

        let headings = div()
            .flex()
            .gap_3()
            .text_sm()
            .text_color(rgb(theme::MUTED))
            .child(div().w(px(44.)).child("Ícone"))
            .child(div().w(px(NAME_WIDTH)).child("Nome"))
            .child(div().w(px(PORT_WIDTH)).child("Entrada"))
            .child(div().w(px(LOCAL_WIDTH)).child("Onde o app roda"))
            .child(
                div()
                    .w(px(CHANNEL_WIDTH * 4. + 12.))
                    .child("Canal Logitech"),
            )
            .child(div().w(px(HOTKEY_WIDTH)).child("Atalho"));

        let has_targets = !rows.is_empty();
        div()
            .id("settings")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            // Clicar fora de um campo encerra a edição; clicar em outro campo a reinicia nele.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.stop_editing(cx)),
            )
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().child("Destinos"))
                    .child(button("detect", "Detectar entradas").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.state.update(cx, |state, cx| state.detect(cx));
                        },
                    ))),
            )
            .child(headings)
            .when(!has_targets, |page| {
                page.child(div().text_color(rgb(theme::MUTED)).child(
                    "Nenhuma entrada ainda. Conecte o monitor e clique em Detectar entradas.",
                ))
            })
            .children(rows)
            .when(has_targets, |page| {
                page.child(div().text_sm().text_color(rgb(theme::MUTED)).child(
                    "Canal Logitech: o canal Easy-Switch para onde teclado e mouse vão junto com a tela. Com –, eles ficam neste computador.",
                ))
            })
            .child(
                div()
                    .mt_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child("Atalho do seletor")
                    .child(
                        self.field(
                            Field::SwitcherHotkey,
                            pretty(config.switcher_hotkey.as_deref()),
                            "definir atalho",
                            cx,
                        )
                        .w(px(HOTKEY_WIDTH)),
                    )
                    .child(div().text_sm().text_color(rgb(theme::MUTED)).child(
                        "Segure os modificadores e toque a tecla para avançar; solte para trocar.",
                    )),
            )
            .children(errors)
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(theme::MUTED))
                    .child(state.status.clone()),
            )
    }
}

fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.))
        .px_3()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .border_1()
        .border_color(rgb(theme::BORDER))
        .bg(rgb(theme::FIELD))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::HOVER)))
        .child(label.into())
}

fn next_icon(current: &str) -> String {
    let position = ICONS
        .iter()
        .position(|icon| *icon == current)
        .map_or(0, |at| at + 1);
    ICONS[position % ICONS.len()].to_string()
}

/// Atalho no formato da porta `Hotkeys`, a partir da tecla capturada pela janela.
fn shortcut_from(keystroke: &Keystroke) -> String {
    let modifiers = keystroke.modifiers;
    let platform = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "super"
    };
    [
        (modifiers.control, "ctrl"),
        (modifiers.alt, "alt"),
        (modifiers.shift, "shift"),
        (modifiers.platform, platform),
    ]
    .into_iter()
    .filter_map(|(held, name)| held.then_some(name))
    .chain([keystroke.key.as_str()])
    .collect::<Vec<_>>()
    .join("+")
}

/// Atalho como o usuário lê: símbolos no macOS, o próprio texto nos demais.
fn pretty(shortcut: Option<&str>) -> String {
    let Some(shortcut) = shortcut else {
        return String::new();
    };
    if !cfg!(target_os = "macos") {
        return shortcut.to_string();
    }
    shortcut
        .split('+')
        .map(|part| match part {
            "ctrl" => "⌃".to_string(),
            "alt" => "⌥".to_string(),
            "shift" => "⇧".to_string(),
            "cmd" => "⌘".to_string(),
            key => format!(" {}", key.to_uppercase()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use anyhow::Result;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    use super::*;
    use crate::{
        adapters::Platform,
        config,
        domain::Config,
        ports::{Action, Hotkeys, Monitor, Peripherals},
    };

    const USB_C: InputCode = InputCode(0x1b);
    const SWITCHER: &str = "ctrl+alt+tab";

    type Bindings = Arc<Mutex<Vec<(String, Action)>>>;

    struct NoHardware;

    impl Monitor for NoHardware {
        fn inputs(&self) -> Result<Vec<InputCode>> {
            Ok(Vec::new())
        }

        fn current_input(&self) -> Result<InputCode> {
            Ok(USB_C)
        }

        fn set_input(&self, _: InputCode) -> Result<()> {
            Ok(())
        }
    }

    impl Peripherals for NoHardware {
        fn switch_to(&self, _: Channel) -> Result<()> {
            Ok(())
        }
    }

    /// Guarda os atalhos que o app pediu para registrar por último.
    struct RecordingHotkeys(Bindings);

    impl Hotkeys for RecordingHotkeys {
        fn rebind(&mut self, bindings: &[(String, Action)]) -> Vec<String> {
            *self.0.lock().expect("lock") = bindings.to_vec();
            Vec::new()
        }

        fn modifiers_held(&self, _: &str) -> Option<bool> {
            Some(false)
        }
    }

    struct Opened<'a> {
        state: Entity<AppState>,
        bindings: Bindings,
        config_path: PathBuf,
        cx: &'a mut VisualTestContext,
    }

    impl Opened<'_> {
        fn click(&mut self, field: &'static str) {
            let bounds = self
                .cx
                .debug_bounds(field)
                .unwrap_or_else(|| panic!("campo {field} não foi desenhado"));
            self.cx.simulate_click(bounds.center(), Modifiers::none());
        }

        fn config(&mut self) -> Config {
            self.state
                .read_with(self.cx, |state, _| state.config.clone())
        }

        fn bindings(&self) -> Vec<(String, Action)> {
            self.bindings.lock().expect("lock").clone()
        }
    }

    /// Abre a janela com um destino "Mac" sem atalho e o seletor em `SWITCHER`.
    fn open<'a>(test: &str, cx: &'a mut TestAppContext) -> Opened<'a> {
        let config_path = std::env::temp_dir()
            .join(format!("kvm-switch-ui-{}-{test}", std::process::id()))
            .join("config.json");
        let config = Config {
            targets: vec![Target {
                input: USB_C,
                name: "Mac".into(),
                icon: "🍎".into(),
                channel: None,
                hotkey: None,
            }],
            local_input: Some(USB_C),
            switcher_hotkey: Some(SWITCHER.into()),
        };
        config::save(&config_path, &config).expect("salvar configuração");

        let bindings = Bindings::default();
        let platform = Platform {
            monitor: Arc::new(NoHardware),
            peripherals: Arc::new(NoHardware),
            hotkeys: Box::new(RecordingHotkeys(bindings.clone())),
        };
        let state = cx.new(|_| AppState::new(platform, config_path.clone()));
        let view_state = state.clone();
        let (_, cx) = cx.add_window_view(|window, cx| SettingsView::new(view_state, window, cx));
        Opened {
            state,
            bindings,
            config_path,
            cx,
        }
    }

    fn switcher_bindings() -> Vec<(String, Action)> {
        vec![
            (SWITCHER.to_string(), Action::SwitcherNext),
            (format!("shift+{SWITCHER}"), Action::SwitcherPrevious),
        ]
    }

    #[gpui::test]
    fn typing_edits_the_name_and_saves_it(cx: &mut TestAppContext) {
        let mut window = open("name", cx);
        window.click("Name(0)");
        // Depois do Enter a edição acabou: o "z" não pode entrar no nome.
        window
            .cx
            .simulate_keystrokes("backspace backspace backspace shift-l i n u x enter z");
        assert_eq!(window.config().targets[0].name, "Linux");
        let saved = config::load(&window.config_path).expect("carregar");
        assert_eq!(saved.targets[0].name, "Linux");
    }

    #[gpui::test]
    fn recording_a_chord_sets_the_hotkey_and_registers_it(cx: &mut TestAppContext) {
        let mut window = open("record", cx);
        window.click("Hotkey(0)");
        assert_eq!(
            window.bindings(),
            [],
            "os atalhos ficam soltos durante a gravação"
        );

        window.cx.simulate_keystrokes("cmd-alt-1");
        let platform = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "super"
        };
        let shortcut = format!("alt+{platform}+1");
        assert_eq!(
            window.config().targets[0].hotkey.as_deref(),
            Some(shortcut.as_str())
        );
        let mut expected = vec![(shortcut, Action::SwitchTo(USB_C))];
        expected.extend(switcher_bindings());
        assert_eq!(window.bindings(), expected);
    }

    #[gpui::test]
    fn a_bare_key_is_not_a_hotkey_and_escape_keeps_the_old_one(cx: &mut TestAppContext) {
        let mut window = open("escape", cx);
        window.click("SwitcherHotkey");
        window.cx.simulate_keystrokes("a");
        assert_eq!(
            window.bindings(),
            [],
            "uma tecla sozinha não encerra a gravação"
        );

        window.cx.simulate_keystrokes("escape");
        assert_eq!(window.config().switcher_hotkey.as_deref(), Some(SWITCHER));
        assert_eq!(window.bindings(), switcher_bindings());
    }

    #[gpui::test]
    fn backspace_removes_the_hotkey(cx: &mut TestAppContext) {
        let mut window = open("clear", cx);
        window.click("SwitcherHotkey");
        window.cx.simulate_keystrokes("backspace");
        assert_eq!(window.config().switcher_hotkey, None);
        assert_eq!(
            config::load(&window.config_path)
                .expect("carregar")
                .switcher_hotkey,
            None
        );
        assert_eq!(window.bindings(), []);
    }

    #[test]
    fn icons_cycle_and_unknown_icons_restart_the_list() {
        assert_eq!(next_icon("🖥"), "💻");
        assert_eq!(next_icon("🎮"), "🖥");
        assert_eq!(next_icon("?"), "🖥");
    }
}
