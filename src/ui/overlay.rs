//! Painéis flutuantes no centro da tela: o seletor de destinos, os avisos e as
//! perguntas de sim ou não.

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Div, Entity, FocusHandle, KeyDownEvent,
    Pixels, Render, SharedString, Size, Window, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, div, prelude::*, px, rgb, rgba, size,
};

use super::{button, theme};

const CARD: Pixels = px(150.);
const GAP: Pixels = px(12.);
const PADDING: Pixels = px(16.);
const HUD_WIDTH: Pixels = px(560.);
const HUD_HEIGHT: Pixels = px(96.);
const QUESTION_WIDTH: Pixels = px(520.);
const QUESTION_HEIGHT: Pixels = px(170.);

/// Quem fica com o teclado quando um painel abre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// O painel recebe o teclado, para o app saber dos modificadores.
    Take,
    /// O app em uso continua com o teclado, onde o sistema deixa escolher (o
    /// Wayland sempre foca a janela nova).
    Leave,
}

#[derive(Debug)]
pub struct SwitcherItem {
    pub icon: SharedString,
    pub name: SharedString,
    pub port: SharedString,
    pub is_local: bool,
}

#[derive(Debug)]
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

#[derive(Debug)]
pub struct HudView {
    pub message: SharedString,
}

impl Render for HudView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        panel().px_6().py_4().text_lg().child(self.message.clone())
    }
}

/// Uma pergunta de sim ou não, com o texto de cada botão.
#[derive(Debug, Clone)]
pub struct Question {
    pub message: SharedString,
    pub detail: SharedString,
    pub yes: SharedString,
    pub no: SharedString,
}

/// O que fazer com a resposta: `true` é sim. Recebe a janela da pergunta, que
/// continua aberta até quem respondeu fechá-la.
type Answer = Box<dyn FnOnce(bool, AnyWindowHandle, &mut App)>;

pub struct QuestionView {
    question: Question,
    /// Some até a primeira resposta: cliques depois dela não fazem nada.
    answer: Option<Answer>,
    focus: FocusHandle,
}

impl QuestionView {
    fn reply(&mut self, yes: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(answer) = self.answer.take() else {
            return;
        };
        let handle = window.window_handle();
        // Fora da atualização desta janela, porque a resposta costuma fechá-la.
        cx.defer(move |cx| answer(yes, handle, cx));
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "enter" => self.reply(true, window, cx),
            "escape" => self.reply(false, window, cx),
            _ => {}
        }
    }
}

impl Render for QuestionView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let question = &self.question;
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_between()
            .p(PADDING)
            .rounded_xl()
            .bg(rgba(0x1e1f22f0))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .text_color(rgb(theme::TEXT))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(div().text_lg().child(question.message.clone()))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(theme::MUTED))
                    .child(question.detail.clone()),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("question-no", question.no.clone())
                            .debug_selector(|| "QuestionNo".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reply(false, window, cx);
                            })),
                    )
                    .child(
                        button("question-yes", question.yes.clone())
                            .debug_selector(|| "QuestionYes".into())
                            .bg(rgb(theme::ACCENT))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reply(true, window, cx);
                            })),
                    ),
            )
    }
}

/// Fundo comum dos painéis: escuro, translúcido, ocupando a janela toda.
fn panel() -> Div {
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

pub fn open_switcher(
    items: Vec<SwitcherItem>,
    selected: usize,
    focus: Focus,
    cx: &mut App,
) -> Option<WindowHandle<SwitcherView>> {
    let count = items.len() as f32;
    let width = CARD * count + GAP * (count - 1.).max(0.) + PADDING * 2.;
    open(size(width, CARD + PADDING * 2.), focus, cx, |_, cx| {
        cx.new(|_| SwitcherView { items, selected })
    })
}

pub fn open_hud(view: HudView, cx: &mut App) -> Option<AnyWindowHandle> {
    open(size(HUD_WIDTH, HUD_HEIGHT), Focus::Leave, cx, |_, cx| {
        cx.new(|_| view)
    })
    .map(Into::into)
}

/// Abre a pergunta no centro da tela, com o teclado: Enter responde sim, Esc não.
pub fn open_question(
    question: Question,
    cx: &mut App,
    answer: impl FnOnce(bool, AnyWindowHandle, &mut App) + 'static,
) -> Option<AnyWindowHandle> {
    let answer: Answer = Box::new(answer);
    let opened = open(
        size(QUESTION_WIDTH, QUESTION_HEIGHT),
        Focus::Take,
        cx,
        |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                window.focus(&focus);
                QuestionView {
                    question,
                    answer: Some(answer),
                    focus,
                }
            })
        },
    )
    .map(Into::into);
    // A pergunta vem para a frente mesmo com outro app em uso.
    cx.activate(true);
    opened
}

pub fn close(window: AnyWindowHandle, cx: &mut App) {
    // Falha só se a janela já foi fechada, que é o estado desejado.
    window
        .update(cx, |_, window, _| window.remove_window())
        .ok();
}

/// Abre um painel sem moldura no centro da tela.
fn open<V: Render + 'static>(
    size: Size<Pixels>,
    focus: Focus,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Option<WindowHandle<V>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: None,
        focus: focus == Focus::Take,
        kind: WindowKind::PopUp,
        is_movable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(super::APP_ID.into()),
        ..Default::default()
    };
    cx.open_window(options, build).ok()
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    use super::*;

    fn question() -> Question {
        Question {
            message: "Encerrar?".into(),
            detail: "Detalhe".into(),
            yes: "Encerrar".into(),
            no: "Cancelar".into(),
        }
    }

    /// Abre a pergunta e guarda as respostas que ela der.
    fn ask(cx: &mut TestAppContext) -> (VisualTestContext, Rc<RefCell<Vec<bool>>>) {
        let answers = Rc::new(RefCell::new(Vec::new()));
        let recorded = answers.clone();
        let window = cx
            .update(|cx| {
                open_question(question(), cx, move |yes, _, _| {
                    recorded.borrow_mut().push(yes)
                })
            })
            .expect("janela da pergunta");
        (VisualTestContext::from_window(window, cx), answers)
    }

    #[gpui::test]
    fn clicking_yes_answers_once(cx: &mut TestAppContext) {
        let (mut window, answers) = ask(cx);
        let yes = window.debug_bounds("QuestionYes").expect("botão sim");
        window.simulate_click(yes.center(), Modifiers::none());
        window.simulate_click(yes.center(), Modifiers::none());
        window.run_until_parked();
        assert_eq!(*answers.borrow(), [true]);
    }

    #[gpui::test]
    fn escape_answers_no_and_enter_answers_yes(cx: &mut TestAppContext) {
        let (mut window, answers) = ask(cx);
        window.simulate_keystrokes("escape");
        window.run_until_parked();
        assert_eq!(*answers.borrow(), [false]);

        let (mut window, answers) = ask(cx);
        window.simulate_keystrokes("enter");
        window.run_until_parked();
        assert_eq!(*answers.borrow(), [true]);
    }
}
