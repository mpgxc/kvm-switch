//! Bordas no GNOME: no Wayland o app não vê o cursor, quem vê é a extensão do
//! GNOME Shell em `linux/gnome-extension/`, que roda `kvm-switch edge <lado>`.
//! Aqui o app só liga a extensão e conta na janela quando ela ainda não funciona.

use std::path::Path;

use anyhow::Result;

use super::gnome::quoted;
use crate::{adapters::run, domain::Side, ports::EdgeSensor};

/// Identificador da extensão, o nome da pasta dela e o que o `gnome-extensions` usa.
const UUID: &str = "kvm-switch@mpgxc.github.io";

/// Liga a extensão das bordas quando a configuração passa a ter algum lado ativo.
#[derive(Debug, Default)]
pub struct GnomeEdges {
    /// Aviso da última conferência, enquanto há lados ativos. O app chama `watch`
    /// a cada mudança na configuração; conferir de novo custaria dois processos.
    aviso: Option<Option<String>>,
}

impl EdgeSensor for GnomeEdges {
    fn watch(&mut self, sides: &[Side]) -> Option<String> {
        if sides.is_empty() {
            // A extensão fica ligada: sem vizinho, o `kvm-switch edge` não faz nada.
            self.aviso = None;
            return None;
        }
        self.aviso.get_or_insert_with(ligar).clone()
    }
}

/// Liga a extensão e diz o que falta para ela funcionar; `None` se já está ativa.
fn ligar() -> Option<String> {
    if run("gnome-extensions", &["enable", UUID]).is_err() && instalada() {
        // Recém-instalada: o Shell só conhece as extensões que achou ao iniciar a
        // sessão e recusa o `enable`. Na lista do GNOME ela liga no próximo login.
        return Some(match ligar_no_proximo_login() {
            Ok(()) => SAIR_E_ENTRAR.to_string(),
            Err(error) => format!("{error:#}"),
        });
    }
    match run("gnome-extensions", &["info", UUID]) {
        Ok(info) => aviso(&info),
        Err(error) => Some(format!(
            "as bordas precisam da extensão {UUID} no GNOME Shell 45 ou mais novo; \
             instale com linux/install.sh --app ou pelo .deb ({error:#})"
        )),
    }
}

const SAIR_E_ENTRAR: &str = "Saia e entre na sessão para ativar as bordas";

/// Põe a extensão em `enabled-extensions`, a lista que o Shell liga ao iniciar.
fn ligar_no_proximo_login() -> Result<()> {
    let mut ligadas = quoted(&run(
        "gsettings",
        &["get", "org.gnome.shell", "enabled-extensions"],
    )?);
    if !ligadas.iter().any(|uuid| uuid == UUID) {
        ligadas.push(UUID.to_string());
        let lista = format!("['{}']", ligadas.join("', '"));
        run(
            "gsettings",
            &["set", "org.gnome.shell", "enabled-extensions", &lista],
        )?;
    }
    Ok(())
}

/// A pasta da extensão existe, do `install.sh --app` (usuário) ou do `.deb` (sistema).
fn instalada() -> bool {
    let usuario = dirs::data_dir().map(|dir| dir.join("gnome-shell/extensions").join(UUID));
    let sistema = Path::new("/usr/share/gnome-shell/extensions").join(UUID);
    usuario.is_some_and(|dir| dir.is_dir()) || sistema.is_dir()
}

/// Aviso para a janela a partir da saída de `gnome-extensions info`; `None` se ativa.
fn aviso(info: &str) -> Option<String> {
    let estado = info
        .lines()
        .find_map(|line| line.trim().strip_prefix("State:"))
        .map(str::trim);
    match estado {
        Some("ACTIVE") => None,
        Some("OUT OF DATE") => Some(
            "a extensão das bordas não aceita esta versão do GNOME Shell (precisa do 45 ou mais novo)"
                .to_string(),
        ),
        Some("ERROR") => Some(
            "a extensão das bordas falhou ao carregar; veja `journalctl --user -b /usr/bin/gnome-shell`"
                .to_string(),
        ),
        // Com `enable` aceito e ainda desligada: as extensões estão todas desligadas.
        Some("INACTIVE") => Some(
            "a extensão das bordas está desligada; ligue as extensões no app Extensões do GNOME"
                .to_string(),
        ),
        Some(estado) => Some(format!("{SAIR_E_ENTRAR} (extensão em {estado})")),
        None => Some(format!(
            "resposta inesperada do gnome-extensions: {}",
            info.trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Saídas reais do `gnome-extensions info` do GNOME Shell 46.0 (Ubuntu 24.04,
    // `gnome-shell --headless`) com esta extensão instalada em ~/.local.
    const INFO_ACTIVE: &str = "\
kvm-switch@mpgxc.github.io
  Name: KVM Switch: bordas
  Description: Empurrar o cursor contra a borda externa da tela roda `kvm-switch edge <lado>`, que manda teclado e mouse Logitech para o PC vizinho daquele lado.
  Path: /root/.local/share/gnome-shell/extensions/kvm-switch@mpgxc.github.io
  URL: https://github.com/mpgxc/kvm-switch
  Enabled: Yes
  State: ACTIVE
";

    // Depois de `gnome-extensions disable`.
    const INFO_INACTIVE: &str = "\
kvm-switch@mpgxc.github.io
  Name: KVM Switch: bordas
  Description: Empurrar o cursor contra a borda externa da tela roda `kvm-switch edge <lado>`, que manda teclado e mouse Logitech para o PC vizinho daquele lado.
  Path: /root/.local/share/gnome-shell/extensions/kvm-switch@mpgxc.github.io
  URL: https://github.com/mpgxc/kvm-switch
  Enabled: No
  State: INACTIVE
";

    // Uma cópia com `shell-version` ["45"], num Shell 46.
    const INFO_OUT_OF_DATE: &str = "\
velha@teste
  Name: KVM Switch: bordas
  Description: Empurrar o cursor contra a borda externa da tela roda `kvm-switch edge <lado>`, que manda teclado e mouse Logitech para o PC vizinho daquele lado.
  Path: /root/.local/share/gnome-shell/extensions/velha@teste
  URL: https://github.com/mpgxc/kvm-switch
  Enabled: No
  State: OUT OF DATE
";

    #[test]
    fn an_active_extension_needs_no_notice() {
        assert_eq!(aviso(INFO_ACTIVE), None);
    }

    #[test]
    fn an_extension_that_is_not_running_gets_a_notice() {
        let inactive = aviso(INFO_INACTIVE).expect("aviso");
        assert!(inactive.contains("desligada"), "{inactive}");
        let old = aviso(INFO_OUT_OF_DATE).expect("aviso");
        assert!(old.contains("45"), "{old}");
        let loading = aviso(&INFO_ACTIVE.replace("ACTIVE", "INITIALIZED")).expect("aviso");
        assert!(loading.starts_with(SAIR_E_ENTRAR), "{loading}");
        assert!(aviso("").is_some());
    }

    #[test]
    fn the_answer_is_cached_while_sides_stay_active() {
        let mut edges = GnomeEdges {
            aviso: Some(Some("guardado".into())),
        };
        assert_eq!(edges.watch(&[Side::Left]).as_deref(), Some("guardado"));
        assert_eq!(edges.watch(&[]), None);
        assert!(edges.aviso.is_none());
    }
}
