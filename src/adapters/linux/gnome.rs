//! Atalhos globais pelos atalhos personalizados do GNOME.
//!
//! No Wayland um app não ouve o teclado fora da própria janela. O portal
//! GlobalShortcuts do XDG existe, mas pede uma confirmação do sistema a cada
//! atalho novo e deixa a troca de tecla para as Configurações do GNOME, o que
//! não combina com gravar o atalho na janela do app. Aqui cada atalho vira um
//! atalho personalizado do GNOME que roda este programa com o pedido (veja
//! `instance`). Eles ficam em Configurações → Teclado → Atalhos personalizados.

use std::env;

use anyhow::{Context as _, Result, bail};

use crate::{
    adapters::run,
    ports::{Action, Hotkeys},
    request,
};

const SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const BINDING_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const DIR: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
/// Começo do nome dos atalhos criados por este app dentro de `DIR`.
const OURS: &str = "kvm-switch-";

#[derive(Debug)]
pub struct GnomeHotkeys {
    /// Este executável, já entre aspas para a linha de comando do atalho.
    program: String,
}

impl GnomeHotkeys {
    pub fn new() -> Result<Self> {
        let program = env::current_exe().context("não descobri o caminho deste programa")?;
        Ok(Self {
            program: shell_quote(&program.to_string_lossy()),
        })
    }

    fn write(&self, bindings: &[(String, Action)]) -> Result<Vec<String>> {
        let current = quoted(&run("gsettings", &["get", SCHEMA, "custom-keybindings"]).context(
            "sem GNOME, crie os atalhos no seu ambiente gráfico rodando `kvm-switch switch <entrada>`",
        )?);
        let ours = format!("{DIR}{OURS}");
        let others: Vec<&String> = current
            .iter()
            .filter(|path| !path.starts_with(&ours))
            .collect();
        // Atalhos de outros programas: dois com a mesma tecla e o GNOME só atende um.
        let mut taken = Vec::new();
        if !bindings.is_empty() {
            for path in &others {
                taken.push((
                    normalized(&get(path, "binding")?),
                    get(path, "name").unwrap_or_default(),
                ));
            }
        }

        let mut errors = Vec::new();
        let mut paths: Vec<String> = others.into_iter().cloned().collect();
        let mut index = 0;
        for (shortcut, action) in bindings {
            let accelerator = match accelerator(shortcut) {
                Ok(accelerator) => accelerator,
                Err(error) => {
                    errors.push(format!("{shortcut}: {error}"));
                    continue;
                }
            };
            let wanted = normalized(&accelerator);
            if let Some((_, name)) = taken.iter().find(|(other, _)| *other == wanted) {
                errors.push(format!("{shortcut}: já é o atalho \"{name}\" do GNOME"));
                continue;
            }
            let path = format!("{DIR}{OURS}{index}/");
            index += 1;
            set(&path, "name", &name(*action))?;
            let command = format!("{} {}", self.program, request::arguments(*action));
            set(&path, "command", &command)?;
            set(&path, "binding", &accelerator)?;
            paths.push(path);
        }

        if paths != current {
            let list = match paths.as_slice() {
                [] => "@as []".to_string(),
                paths => format!("['{}']", paths.join("', '")),
            };
            run("gsettings", &["set", SCHEMA, "custom-keybindings", &list])?;
        }
        // Quem saiu da lista já não vale; apagar o resto só evita sobras no dconf.
        for stale in current.iter().filter(|path| !paths.contains(path)) {
            let entry = format!("{BINDING_SCHEMA}:{stale}");
            run("gsettings", &["reset-recursively", &entry]).ok();
        }
        Ok(errors)
    }
}

impl Hotkeys for GnomeHotkeys {
    fn rebind(&mut self, bindings: &[(String, Action)]) -> Vec<String> {
        self.write(bindings)
            .unwrap_or_else(|error| vec![format!("{error:#}")])
    }

    fn modifiers_held(&self, _: &str) -> Option<bool> {
        None
    }
}

/// Um texto lido do atalho em `path`, sem as aspas com que o `gsettings` o imprime.
fn get(path: &str, key: &str) -> Result<String> {
    let value = run(
        "gsettings",
        &["get", &format!("{BINDING_SCHEMA}:{path}"), key],
    )?;
    let value = value.trim();
    let inner = value
        .strip_prefix(['\'', '"'])
        .and_then(|rest| rest.strip_suffix(['\'', '"']));
    Ok(inner.unwrap_or(value).to_string())
}

fn set(path: &str, key: &str, value: &str) -> Result<()> {
    // Texto no formato GVariant: entre aspas duplas, com `\` e `"` escapados.
    let value = format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""));
    run(
        "gsettings",
        &["set", &format!("{BINDING_SCHEMA}:{path}"), key, &value],
    )
    .map(drop)
}

/// Os textos numa lista impressa pelo `gsettings get`, como `['/a/', '/b/']`.
/// A lista vazia sai como `@as []`.
pub(super) fn quoted(value: &str) -> Vec<String> {
    value
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// Nome que aparece nas Configurações do GNOME.
fn name(action: Action) -> String {
    match action {
        Action::SwitchTo(input) => format!("KVM Switch: {}", input.label()),
        Action::SwitcherNext => "KVM Switch: seletor".into(),
        Action::SwitcherPrevious => "KVM Switch: seletor, voltar".into(),
        Action::OpenSettings | Action::Start => "KVM Switch".into(),
        Action::Quit => "KVM Switch: encerrar".into(),
        Action::CrossEdge(side) => format!("KVM Switch: borda {}", side.word()),
    }
}

/// Atalho no formato do GTK, como `<Super><Alt>1`, a partir do formato da porta.
fn accelerator(shortcut: &str) -> Result<String> {
    let mut parts: Vec<&str> = shortcut.split('+').collect();
    // Só a tecla `+` deixa o texto terminando em "++".
    let key = if shortcut.ends_with("++") {
        parts.truncate(parts.len() - 2);
        "+"
    } else {
        parts.pop().unwrap_or_default()
    };
    let mut accelerator = String::new();
    for modifier in parts {
        accelerator.push_str(match modifier {
            "ctrl" => "<Control>",
            "alt" => "<Alt>",
            "shift" => "<Shift>",
            "cmd" | "super" => "<Super>",
            other => bail!("modificador desconhecido: {other}"),
        });
    }
    accelerator.push_str(&key_name(key)?);
    Ok(accelerator)
}

/// Nome da tecla para o GTK, a partir do nome que o GPUI dá a ela.
fn key_name(key: &str) -> Result<String> {
    let name = match key {
        "tab" => "Tab",
        "space" => "space",
        "enter" => "Return",
        "escape" => "Escape",
        "backspace" => "BackSpace",
        "delete" => "Delete",
        "insert" => "Insert",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "-" => "minus",
        "=" => "equal",
        "+" => "plus",
        "[" => "bracketleft",
        "]" => "bracketright",
        ";" => "semicolon",
        "'" => "apostrophe",
        "`" => "grave",
        "," => "comma",
        "." => "period",
        "/" => "slash",
        "\\" => "backslash",
        _ => {
            let mut letters = key.chars();
            return match (letters.next(), letters.as_str()) {
                (Some(only), "") if only.is_ascii_alphanumeric() => Ok(key.to_string()),
                // f1, f2, …: o GTK quer a letra maiúscula.
                (Some('f'), number) if number.parse::<u8>().is_ok() => Ok(format!("F{number}")),
                _ => bail!("o GNOME não tem atalho para a tecla {key}"),
            };
        }
    };
    Ok(name.to_string())
}

/// Modificadores em ordem e tudo em minúsculas, para comparar dois atalhos do
/// GTK que o usuário pode ter escrito de formas diferentes.
fn normalized(accelerator: &str) -> String {
    let mut modifiers = Vec::new();
    let mut rest = accelerator;
    while let Some((modifier, after)) = rest
        .strip_prefix('<')
        .and_then(|inside| inside.split_once('>'))
    {
        modifiers.push(match modifier.to_ascii_lowercase().as_str() {
            "primary" | "ctrl" | "control" => "control".to_string(),
            "mod1" | "alt" => "alt".to_string(),
            "mod4" | "super" => "super".to_string(),
            other => other.to_string(),
        });
        rest = after;
    }
    modifiers.sort();
    format!("{}+{}", modifiers.join("+"), rest.to_lowercase())
}

/// Entre aspas simples, como o shell (e o GNOME, que lê o comando do mesmo jeito) espera.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_the_recorder_can_produce_is_a_valid_accelerator() {
        for (key, name) in [
            ("1", "1"),
            ("a", "a"),
            ("tab", "Tab"),
            ("space", "space"),
            ("enter", "Return"),
            ("f5", "F5"),
            ("f12", "F12"),
            ("-", "minus"),
            ("[", "bracketleft"),
            ("up", "Up"),
            ("pagedown", "Page_Down"),
            ("home", "Home"),
            ("+", "plus"),
        ] {
            let shortcut = format!("ctrl+alt+shift+super+{key}");
            assert_eq!(
                accelerator(&shortcut).expect(&shortcut),
                format!("<Control><Alt><Shift><Super>{name}")
            );
        }
        assert_eq!(accelerator("cmd+alt+1").expect("cmd"), "<Super><Alt>1");
    }

    #[test]
    fn keys_gnome_cannot_bind_are_refused() {
        assert!(accelerator("super+alt+dead_acute").is_err());
        assert!(accelerator("hyper+1").is_err());
    }

    #[test]
    fn the_same_shortcut_written_differently_is_a_conflict() {
        assert_eq!(normalized("<Super><Alt>1"), normalized("<Alt><Super>1"));
        assert_eq!(
            normalized("<Primary><Alt>Tab"),
            normalized("<Control><alt>tab")
        );
        assert_ne!(normalized("<Super><Alt>1"), normalized("<Super><Alt>2"));
        assert_ne!(normalized("<Super>1"), normalized("<Super><Alt>1"));
    }

    #[test]
    fn reads_gsettings_replies() {
        // Respostas reais do gsettings.
        assert_eq!(
            quoted("['/org/gnome/a/', '/org/gnome/b/']\n"),
            ["/org/gnome/a/", "/org/gnome/b/"]
        );
        assert_eq!(quoted("@as []\n"), Vec::<String>::new());
        assert_eq!(quoted("'<Super><Alt>1'\n"), ["<Super><Alt>1"]);
    }

    #[test]
    fn a_program_path_with_spaces_or_quotes_stays_one_argument() {
        assert_eq!(
            shell_quote("/home/eu/Vibe Coding/kvm-switch"),
            "'/home/eu/Vibe Coding/kvm-switch'"
        );
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
    }
}
