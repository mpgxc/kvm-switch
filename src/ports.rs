//! Portas: o que o app precisa de cada sistema operacional.
//!
//! O núcleo (`domain`, `app`, `ui`) só conhece estas interfaces. Cada sistema
//! fornece as implementações em `adapters`.

use anyhow::Result;

use crate::domain::{Channel, Config, InputCode};

/// Controle do monitor por DDC/CI.
pub trait Monitor: Send + Sync {
    /// Entradas que o monitor diz ter.
    fn inputs(&self) -> Result<Vec<InputCode>>;
    /// Entrada que o monitor está exibindo agora.
    fn current_input(&self) -> Result<InputCode>;
    fn set_input(&self, input: InputCode) -> Result<()>;
}

/// Teclado e mouse com vários canais, conectados a esta máquina.
pub trait Peripherals: Send + Sync {
    /// Manda todos os periféricos para o canal. Erro se nenhum for encontrado.
    fn switch_to(&self, channel: Channel) -> Result<()>;
}

/// O que chega de fora das janelas: um atalho global, um item do ícone na barra
/// ou um pedido de abertura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SwitchTo(InputCode),
    SwitcherNext,
    SwitcherPrevious,
    /// Mostrar a janela de configuração. No Linux é o que chega quando o
    /// programa é aberto de novo com o app já rodando.
    OpenSettings,
    /// Encerrar o app.
    Quit,
}

/// Ícone na barra do sistema, no mesmo processo do app. Os itens clicados são
/// entregues como [`Action`] pelo canal que o adapter recebe ao ser criado.
pub trait StatusMenu {
    /// Monta o menu de novo a partir da configuração.
    fn show(&mut self, config: &Config);
}

/// Atalhos globais. Cada atalho pressionado é entregue como [`Action`] pelo
/// canal que o adapter recebe ao ser criado.
///
/// Formato do atalho: modificadores `ctrl`, `alt`, `shift`, `cmd` (ou `super`)
/// e uma tecla, unidos por `+`. Exemplo: `cmd+alt+1`.
pub trait Hotkeys {
    /// Substitui todos os atalhos registrados. Devolve uma mensagem por atalho recusado.
    fn rebind(&mut self, bindings: &[(String, Action)]) -> Vec<String>;
    /// Diz se os modificadores do atalho estão pressionados agora. `None` quando
    /// o sistema só mostra o teclado à janela em foco, caso do Wayland: aí quem
    /// responde é a janela do seletor.
    fn modifiers_held(&self, shortcut: &str) -> Option<bool>;
}
