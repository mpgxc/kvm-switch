//! Janela de configuração: destinos, nomes, ícones, canais e atalhos.

use gpui::{
    App, AppContext as _, Bounds, BoxShadow, Context, CursorStyle, Decorations, Div, ElementId,
    Entity, FocusHandle, FontWeight, Global, HitboxBehavior, KeyDownEvent, Keystroke, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ResizeEdge, Size, Stateful, Tiling, TitlebarOptions,
    Window, WindowBounds, WindowDecorations, WindowHandle, WindowOptions, canvas, div, hsla, point,
    prelude::*, px, rgb, size,
};

use super::theme;
use crate::{
    app::AppState,
    domain::{Channel, InputCode, Target},
};

/// Ícones oferecidos para um destino; cada clique passa para o próximo.
const ICONS: [&str; 6] = ["🖥", "💻", "🍎", "🐧", "🪟", "🎮"];

const WINDOW_WIDTH: f32 = 1010.;
const WINDOW_HEIGHT: f32 = 460.;
/// Altura dos campos e dos botões.
const CONTROL_HEIGHT: f32 = 30.;
const ACTIVE_WIDTH: f32 = 40.;
const CHECKBOX_SIZE: f32 = 18.;
/// Opacidade de uma linha fora do seletor.
const INACTIVE_OPACITY: f32 = 0.45;
const ICON_WIDTH: f32 = 44.;
const NAME_WIDTH: f32 = 180.;
const PORT_WIDTH: f32 = 70.;
const LOCAL_WIDTH: f32 = 170.;
const CHANNEL_WIDTH: f32 = 30.;
const HOTKEY_WIDTH: f32 = 170.;
/// Moldura desenhada pelo próprio app quando o sistema não desenha uma (GNOME no Wayland).
const TITLE_BAR_HEIGHT: f32 = 40.;
const WINDOW_BUTTON_SIZE: f32 = 24.;
/// Faixa transparente em volta da janela: mostra a sombra e é por onde se redimensiona.
const SHADOW: f32 = 10.;
const ROUNDING: f32 = 12.;

/// O que o app inteiro compartilha: o estado e a janela de configuração aberta.
#[derive(Debug)]
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

    // No Linux a sombra e a barra de título desenhadas pelo app ocupam espaço dentro da janela.
    let decorated = cfg!(target_os = "linux");
    let (width, height) = if decorated {
        (
            WINDOW_WIDTH + 2. * SHADOW,
            WINDOW_HEIGHT + TITLE_BAR_HEIGHT + 2. * SHADOW,
        )
    } else {
        (WINDOW_WIDTH, WINDOW_HEIGHT)
    };
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("KVM Switch".into()),
            ..Default::default()
        }),
        app_id: Some(super::APP_ID.into()),
        // O GNOME não desenha moldura para apps Wayland (não tem o xdg-decoration); pedindo a
        // do sistema, a janela ficaria sem nenhuma. Quem tiver a do sistema avisa em
        // `window_decorations()`, e aí o app não desenha nada.
        window_decorations: decorated.then_some(WindowDecorations::Client),
        // No macOS isso apaga o botão verde e o clique duplo na barra deixa de dar zoom. No
        // Linux o GPUI ignora a opção e não tem como pedir tamanho máximo ao compositor; o
        // maximizar que vier de lá é desfeito em `SettingsView::new`.
        is_resizable: false,
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

#[derive(Debug)]
pub struct SettingsView {
    state: Entity<AppState>,
    focus: FocusHandle,
    editing: Option<Field>,
}

impl SettingsView {
    fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        // A página tem layout fixo e não deve ser maximizada. O menu da janela, Super+↑ e
        // arrastar até o topo da tela pedem ao compositor sem passar pelo app, então a janela
        // volta ao tamanho de antes assim que chega maximizada.
        if cfg!(target_os = "linux") {
            cx.observe_window_bounds(window, |_, window, _| {
                if window.is_maximized() {
                    window.zoom_window();
                }
            })
            .detach();
        }
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
            .h(px(CONTROL_HEIGHT))
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
            let label =
                channel.map_or_else(|| "–".to_string(), |channel| channel.number().to_string());
            button(("channel", index * channels.len() + position), label)
                .w(px(CHANNEL_WIDTH))
                .when(target.channel == channel, |button| {
                    button.bg(rgb(theme::ACCENT))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit_target(index, cx, |target| target.channel = channel);
                }))
        });

        let enabled = target.enabled;
        let rest = div()
            .flex()
            .items_center()
            .gap_3()
            // Fora do seletor a linha fica apagada, mas continua editável.
            .when(!enabled, |rest| rest.opacity(INACTIVE_OPACITY))
            .child(
                button(
                    ("icon", index),
                    super::icon(target.icon.clone().into(), px(20.)),
                )
                .w(px(ICON_WIDTH))
                .px_0()
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
            );

        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                checkbox(("enabled", index), enabled)
                    .debug_selector(move || format!("Enabled({index})"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_target(index, cx, |target| target.enabled = !target.enabled);
                    })),
            )
            .child(rest)
    }

    /// Barra de título para quando o app desenha a moldura: arrasta a janela, abre o menu da
    /// janela com o botão direito e traz os botões de minimizar e fechar do GNOME.
    fn title_bar(window: &Window) -> Div {
        let controls = window.window_controls();
        let buttons = div()
            .flex_1()
            .flex()
            .justify_end()
            .gap_3()
            .when(controls.minimize, |buttons| {
                buttons.child(window_button("minimize", minimize_icon(), |window| {
                    window.minimize_window();
                }))
            })
            .child(window_button(
                "close",
                div().text_size(px(13.)).child("✕"),
                |window| window.remove_window(),
            ));

        div()
            .flex_none()
            .h(px(TITLE_BAR_HEIGHT))
            .px_2()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .text_color(rgb(theme::TEXT))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .when(controls.window_menu, |bar| {
                bar.on_mouse_down(MouseButton::Right, |event, window, _| {
                    window.show_window_menu(event.position);
                })
            })
            // Os vãos dos lados têm a mesma largura, então o título fica no centro, como no GNOME.
            .child(div().flex_1())
            .child(div().font_weight(FontWeight::BOLD).child("KVM Switch"))
            .child(buttons)
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(div().w(px(ACTIVE_WIDTH)).child("Ativo"))
            .child(div().w(px(ICON_WIDTH)).child("Ícone"))
            .child(div().w(px(NAME_WIDTH)).child("Nome"))
            .child(div().w(px(PORT_WIDTH)).child("Entrada"))
            .child(div().w(px(LOCAL_WIDTH)).child("Onde o app roda"))
            .child(
                div()
                    // Os quatro botões de canal e os três vãos de 4 px entre eles.
                    .w(px(CHANNEL_WIDTH * 4. + 12.))
                    .child("Canal Logitech"),
            )
            .child(div().w(px(HOTKEY_WIDTH)).child("Atalho"));

        let has_targets = !rows.is_empty();
        let page = div()
            .id("settings")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            // Clicar fora de um campo encerra a edição; clicar em outro campo a reinicia nele.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.stop_editing(cx)),
            )
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
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
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .text_sm()
                        .text_color(rgb(theme::MUTED))
                        .child("Ativo: só os destinos marcados aparecem no seletor. O atalho de cada um e o Ir funcionam mesmo desmarcados.")
                        .child("Canal Logitech: o canal Easy-Switch para onde teclado e mouse vão junto com a tela. Com –, eles ficam neste computador."),
                )
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
            );

        match window.window_decorations() {
            Decorations::Server => page
                .size_full()
                .bg(rgb(theme::BACKGROUND))
                .into_any_element(),
            Decorations::Client { tiling } => {
                let title_bar = Self::title_bar(window);
                frame(title_bar, page.flex_1(), tiling, window).into_any_element()
            }
        }
    }
}

/// Moldura da janela desenhada pelo app: sombra, borda, cantos redondos e as bordas que
/// redimensionam. Nos lados encostados na tela ou em outra janela (`tiling`) não há sombra
/// nem borda.
fn frame(
    title_bar: impl IntoElement,
    page: impl IntoElement,
    tiling: Tiling,
    window: &mut Window,
) -> Div {
    // A sombra fica dentro da superfície; o inset diz ao compositor onde a janela começa.
    window.set_client_inset(px(SHADOW));
    let (shadow, rounding) = (px(SHADOW), px(ROUNDING));
    let resize_cursor = canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            let edge = resize_edge(window.mouse_position(), window.viewport_size(), tiling);
            if let Some(edge) = edge {
                window.set_cursor_style(resize_cursor(edge), &hitbox);
            }
        },
    )
    .size_full()
    .absolute();

    div()
        .size_full()
        .when(!tiling.top, |frame| frame.pt(shadow))
        .when(!tiling.bottom, |frame| frame.pb(shadow))
        .when(!tiling.left, |frame| frame.pl(shadow))
        .when(!tiling.right, |frame| frame.pr(shadow))
        .child(resize_cursor)
        // Redesenha para o cursor acompanhar a borda sob o mouse.
        .on_mouse_move(|_, window, _| window.refresh())
        .on_mouse_down(MouseButton::Left, move |event, window, _| {
            if let Some(edge) = resize_edge(event.position, window.viewport_size(), tiling) {
                window.start_window_resize(edge);
            }
        })
        .child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .cursor(CursorStyle::Arrow)
                .bg(rgb(theme::BACKGROUND))
                .border_color(rgb(theme::BORDER))
                .when(!tiling.top, |inner| inner.border_t_1())
                .when(!tiling.bottom, |inner| inner.border_b_1())
                .when(!tiling.left, |inner| inner.border_l_1())
                .when(!tiling.right, |inner| inner.border_r_1())
                .when(!(tiling.top || tiling.left), |inner| {
                    inner.rounded_tl(rounding)
                })
                .when(!(tiling.top || tiling.right), |inner| {
                    inner.rounded_tr(rounding)
                })
                .when(!(tiling.bottom || tiling.left), |inner| {
                    inner.rounded_bl(rounding)
                })
                .when(!(tiling.bottom || tiling.right), |inner| {
                    inner.rounded_br(rounding)
                })
                .when(!tiling.is_tiled(), |inner| {
                    inner.shadow(vec![BoxShadow {
                        color: hsla(0., 0., 0., 0.4),
                        blur_radius: shadow / 2.,
                        spread_radius: px(0.),
                        offset: point(px(0.), px(0.)),
                    }])
                })
                // Dentro da janela o mouse não é borda de redimensionar.
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .child(title_bar)
                .child(page),
        )
}

/// Borda ou canto da janela sob `position`, contando só a faixa da sombra dos lados soltos.
fn resize_edge(position: Point<Pixels>, size: Size<Pixels>, tiling: Tiling) -> Option<ResizeEdge> {
    let shadow = px(SHADOW);
    let top = !tiling.top && position.y < shadow;
    let bottom = !tiling.bottom && position.y > size.height - shadow;
    let left = !tiling.left && position.x < shadow;
    let right = !tiling.right && position.x > size.width - shadow;
    let edge = match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (.., true, _) => ResizeEdge::Left,
        (.., true) => ResizeEdge::Right,
        _ => return None,
    };
    Some(edge)
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Botão redondo da barra de título, no estilo do GNOME.
fn window_button(
    id: &'static str,
    icon: impl IntoElement,
    action: fn(&mut Window),
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(WINDOW_BUTTON_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(theme::FIELD))
        .hover(|style| style.bg(rgb(theme::HOVER)))
        // O clique é do botão: não arrasta a janela.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, _| action(window))
        .child(icon)
}

/// Traço do botão de minimizar.
fn minimize_icon() -> Div {
    div().mt(px(6.)).w(px(10.)).h(px(1.)).bg(rgb(theme::TEXT))
}

/// Caixa de marcar desenhada com bordas: a fonte do GPUI no Linux não tem os símbolos ☑ e ☐.
fn checkbox(id: impl Into<ElementId>, checked: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(ACTIVE_WIDTH))
        .h(px(CONTROL_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(
            div()
                .size(px(CHECKBOX_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .border_1()
                .border_color(rgb(if checked {
                    theme::ACCENT
                } else {
                    theme::BORDER
                }))
                .bg(rgb(if checked { theme::ACCENT } else { theme::FIELD }))
                .hover(|style| style.border_color(rgb(theme::ACCENT)))
                .text_size(px(13.))
                .text_color(rgb(theme::TEXT))
                .when(checked, |mark| mark.child("✓")),
        )
}

fn button(id: impl Into<ElementId>, label: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(CONTROL_HEIGHT))
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
        .child(label)
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
                enabled: true,
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
            status_menu: None,
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
    fn unchecking_active_hides_the_target_from_the_switcher_only(cx: &mut TestAppContext) {
        let mut window = open("enabled", cx);
        window.click("Hotkey(0)");
        window.cx.simulate_keystrokes("ctrl-alt-1");
        let registered = window.bindings();
        assert!(registered.contains(&("ctrl+alt+1".to_string(), Action::SwitchTo(USB_C))));

        window.click("Enabled(0)");
        assert!(!window.config().targets[0].enabled);
        assert!(
            !config::load(&window.config_path).expect("carregar").targets[0].enabled,
            "a mudança é salva na hora"
        );
        assert_eq!(window.config().switcher(), None, "sem destinos ativos");
        assert_eq!(
            window.bindings(),
            registered,
            "o atalho do destino continua"
        );

        window.click("Enabled(0)");
        assert!(window.config().targets[0].enabled);
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

    #[cfg(target_os = "linux")]
    #[test]
    fn every_icon_on_offer_has_an_image() {
        // Sem imagem o ícone cai no texto, que no Linux sai sem cor ou vira um quadrado.
        for icon in ICONS {
            assert!(crate::ui::icons::image(icon).is_some(), "{icon}");
        }
    }
}
