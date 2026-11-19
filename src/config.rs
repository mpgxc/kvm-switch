//! Configuração gravada em disco, em JSON.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};

use crate::domain::Config;

pub fn default_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("este sistema não informou a pasta de configuração")?;
    Ok(dir.join("kvm-switch").join("config.json"))
}

/// Arquivo ausente vira configuração vazia.
///
/// # Errors
///
/// Arquivo ilegível ou inválido. O inválido é renomeado para `*.invalido`, para
/// que o próximo salvamento não apague o que havia nele.
pub fn load(path: &Path) -> Result<Config> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("não consegui ler {}", path.display()));
        }
    };
    serde_json::from_str(&text).or_else(|error| {
        let kept = path.with_extension("json.invalido");
        fs::rename(path, &kept).ok();
        Err(error).with_context(|| format!("configuração inválida, guardada em {}", kept.display()))
    })
}

pub fn save(path: &Path, config: &Config) -> Result<()> {
    let dir = path.parent().context("caminho de configuração sem pasta")?;
    fs::create_dir_all(dir).with_context(|| format!("não consegui criar {}", dir.display()))?;
    // Grava ao lado e renomeia: uma queda no meio não deixa o arquivo pela metade.
    let partial = path.with_extension("json.parcial");
    fs::write(&partial, serde_json::to_string_pretty(config)?)
        .and_then(|()| fs::rename(&partial, path))
        .with_context(|| format!("não consegui gravar {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Channel, InputCode, Target};

    fn temp_file(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kvm-switch-test-{}-{name}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        dir.join("config.json")
    }

    #[test]
    fn saved_config_loads_back_identical() {
        let path = temp_file("roundtrip");
        let config = Config {
            targets: vec![Target {
                input: InputCode(0x0f),
                name: "Linux".into(),
                icon: "🐧".into(),
                channel: Some(Channel::Two),
                hotkey: Some("cmd+alt+2".into()),
            }],
            local_input: Some(InputCode(0x1b)),
            switcher_hotkey: Some("cmd+alt+tab".into()),
        };
        save(&path, &config).expect("salvar");
        assert_eq!(load(&path).expect("carregar"), config);
    }

    #[test]
    fn missing_file_is_an_empty_config() {
        assert_eq!(
            load(&temp_file("missing")).expect("carregar"),
            Config::default()
        );
    }

    #[test]
    fn invalid_file_is_kept_aside_instead_of_overwritten() {
        let path = temp_file("invalid");
        fs::create_dir_all(path.parent().expect("pasta")).expect("criar pasta");
        fs::write(&path, "{ isto não é json").expect("escrever");
        assert!(load(&path).is_err());
        let kept =
            fs::read_to_string(path.with_extension("json.invalido")).expect("cópia guardada");
        assert_eq!(kept, "{ isto não é json");
    }
}
