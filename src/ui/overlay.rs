//! Painéis flutuantes no centro da tela: o seletor de destinos e os avisos.

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, Pixels, Render, SharedString,
    Size, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind,
    WindowOptions, div, prelude::*, px, rgb, rgba, size,
};

use super::theme;

const CARD: Pixels = px(150.);
const GAP: Pixels = px(12.);
const PADDING: Pixels = px(16.);

pub struct SwitcherItem {
    pub icon: SharedString,
    pub name: SharedString,
    pub port: SharedString,
    pub is_local: bool,
}

pub struct SwitcherView {
    items: Vec<SwitcherItem>,
    selected: usize,
}

impl SwitcherView {
    pub fn select(&mut self, selected: usize, cx: &mut Context<Self>) {
        self.selected = selected;
        cx.notify();
    }
}

impl Render for SwitcherView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let cards = self.items.iter().enumerate().map(|(index, item)| {
            let port = if item.is_local {
                format!("{} · aqui", item.port)
            } else {
                item.port.to_string()
            };
            div()
                .size(CARD)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .rounded_lg()
                .border_2()
                .border_color(rgba(0x00000000))
                .when(index == self.selected, |card| {
                    card.bg(rgba(0xffffff26)).border_color(rgb(theme::ACCENT))
                })
                .child(super::icon(item.icon.clone(), px(52.)))
                .child(div().text_lg().child(item.name.clone()))
                .child(div().text_sm().text_color(rgb(theme::MUTED)).child(port))
        });
        panel().gap(GAP).p(PADDING).children(cards)
    }
}

pub struct HudView {
    pub message: SharedString,
}

impl Render for HudView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        panel().px_6().py_4().text_lg().child(self.message.clone())
    }
}

/// Fundo comum dos painéis: escuro, translúcido, ocupando a janela toda.
fn panel() -> gpui::Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded_xl()
        .bg(rgba(0x1e1f22f0))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .text_color(rgb(theme::TEXT))
}

/// `takes_focus`: o seletor recebe o teclado, para o app saber dos modificadores.
pub fn open_switcher(
    items: Vec<SwitcherItem>,
    selected: usize,
    takes_focus: bool,
    cx: &mut App,
) -> Option<WindowHandle<SwitcherView>> {
    let count = items.len() as f32;
    let width = CARD * count + GAP * (count - 1.).max(0.) + PADDING * 2.;
    open(size(width, CARD + PADDING * 2.), takes_focus, cx, |cx| {
        cx.new(|_| SwitcherView { items, selected })
    })
}

pub fn open_hud(view: HudView, cx: &mut App) -> Option<AnyWindowHandle> {
    open(size(px(560.), px(96.)), false, cx, |cx| cx.new(|_| view)).map(Into::into)
}

pub fn close(window: AnyWindowHandle, cx: &mut App) {
    // Falha só se a janela já foi fechada, que é o estado desejado.
    window
        .update(cx, |_, window, _| window.remove_window())
        .ok();
}

/// Abre um painel sem moldura no centro da tela. Sem `focus`, não tira o foco do
/// app em uso onde o sistema deixa escolher (o Wayland sempre foca a janela nova).
fn open<V: Render + 'static>(
    size: Size<Pixels>,
    focus: bool,
    cx: &mut App,
    build: impl FnOnce(&mut App) -> Entity<V>,
) -> Option<WindowHandle<V>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: None,
        focus,
        kind: WindowKind::PopUp,
        is_movable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(super::APP_ID.into()),
        ..Default::default()
    };
    cx.open_window(options, |_, cx| build(cx)).ok()
}
