//! O menu do ícone na barra do sistema, igual no macOS e no Linux.
//!
//! O id de cada item é o pedido em texto de `request`: quem recebe o clique só
//! precisa de `request::parse` para saber o que fazer.

use anyhow::{Context as _, Result};
use tray_icon::{
    Icon,
    menu::{Menu, MenuItem, PredefinedMenuItem},
};

use crate::{domain::Config, ports::Action, request};

/// Uma linha do menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// `id` é o pedido que o clique faz.
    Item {
        id: String,
        label: String,
    },
    Separator,
}

fn item(action: Action, label: String) -> Entry {
    Entry::Item {
        id: request::arguments(action),
        label,
    }
}

/// As linhas do menu: abrir a configuração, um item por destino ativo e encerrar.
pub fn entries(config: &Config) -> Vec<Entry> {
    let mut entries = vec![
        item(Action::OpenSettings, "Abrir configuração".into()),
        Entry::Separator,
    ];
    for target in config.targets.iter().filter(|target| target.enabled) {
        let here = if Some(target.input) == config.local_input {
            " (aqui)"
        } else {
            ""
        };
        entries.push(item(
            Action::SwitchTo(target.input),
            format!("Ir para {} {}{here}", target.icon, target.name),
        ));
    }
    entries.push(Entry::Separator);
    entries.push(item(Action::Quit, "Encerrar".into()));
    entries
}

/// O menu pronto para o `tray-icon`. No macOS só pode ser criado na thread principal.
pub fn menu(config: &Config) -> Result<Menu> {
    let menu = Menu::new();
    for entry in entries(config) {
        match entry {
            Entry::Item { id, label } => menu.append(&MenuItem::with_id(id, label, true, None)),
            Entry::Separator => menu.append(&PredefinedMenuItem::separator()),
        }
        .context("não consegui montar o menu do ícone")?;
    }
    Ok(menu)
}

/// O ícone do monitor, o mesmo do destino 🖥 na janela.
pub fn icon() -> Result<Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "../assets/icons/1f5a5.png"
    )));
    let mut reader = decoder.read_info().context("ícone da barra inválido")?;
    let mut rgba = vec![
        0;
        reader
            .output_buffer_size()
            .context("ícone da barra grande demais")?
    ];
    let info = reader
        .next_frame(&mut rgba)
        .context("ícone da barra inválido")?;
    rgba.truncate(info.buffer_size());
    Icon::from_rgba(rgba, info.width, info.height).context("ícone da barra inválido")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{InputCode, Target};

    fn target(input: u8, name: &str, icon: &str, enabled: bool) -> Target {
        Target {
            input: InputCode(input),
            name: name.into(),
            icon: icon.into(),
            channel: None,
            hotkey: None,
            enabled,
        }
    }

    fn config() -> Config {
        Config {
            targets: vec![
                target(27, "Mac", "🍎", true),
                target(15, "PC", "🐧", true),
                target(17, "HDMI", "🖥", false),
            ],
            local_input: Some(InputCode(15)),
            switcher_hotkey: None,
        }
    }

    #[test]
    fn menu_lists_only_active_targets_and_marks_this_computer() {
        let labels: Vec<_> = entries(&config())
            .into_iter()
            .map(|entry| match entry {
                Entry::Item { label, .. } => label,
                Entry::Separator => "—".into(),
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Abrir configuração",
                "—",
                "Ir para 🍎 Mac",
                "Ir para 🐧 PC (aqui)",
                "—",
                "Encerrar"
            ]
        );
    }

    #[test]
    fn every_item_id_is_a_request_for_its_action() {
        let actions: Vec<_> = entries(&config())
            .into_iter()
            .filter_map(|entry| match entry {
                Entry::Item { id, .. } => Some(request::parse(&id).expect("pedido")),
                Entry::Separator => None,
            })
            .collect();
        assert_eq!(
            actions,
            [
                Action::OpenSettings,
                Action::SwitchTo(InputCode(27)),
                Action::SwitchTo(InputCode(15)),
                Action::Quit
            ]
        );
    }

    #[test]
    fn bar_icon_decodes() {
        assert!(icon().is_ok());
    }
}
