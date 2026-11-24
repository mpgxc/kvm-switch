//! Linux e macOS: o app atende num socket Unix do usuário. Quem consegue criar o
//! socket é o app; as outras execuções escrevem nele.

use std::{
    fs,
    io::{BufRead as _, BufReader, Write as _},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result, bail};

/// Quem conecta e não escreve não pode travar os próximos pedidos: a leitura
/// desiste depois deste tempo.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

/// O socket do app aberto.
pub struct Listener(UnixListener);

/// Vira o app. Erro se outro app desta sessão já atende.
pub fn claim() -> Result<Listener> {
    claim_at(&path()?)
}

/// Manda uma linha ao app aberto. `Ok(false)`: não há app aberto.
pub fn send(line: &str) -> Result<bool> {
    send_to(&path()?, line)
}

impl Listener {
    /// Atende numa thread. Cada linha vai para `on_line`; `false` para de atender.
    pub fn serve(self, mut on_line: impl FnMut(&str) -> bool + Send + 'static) {
        thread::spawn(move || {
            for stream in self.0.incoming().flatten() {
                stream.set_read_timeout(Some(REQUEST_TIMEOUT)).ok();
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok() && !on_line(&line) {
                    break;
                }
            }
        });
    }
}

fn claim_at(path: &Path) -> Result<Listener> {
    let listener = UnixListener::bind(path).or_else(|_| {
        // O caminho já existe: ou outro app abriu neste instante, ou sobrou de um que fechou.
        if UnixStream::connect(path).is_ok() {
            bail!("o kvm-switch já está aberto");
        }
        fs::remove_file(path).ok();
        UnixListener::bind(path).with_context(|| format!("não consegui criar {}", path.display()))
    })?;
    Ok(Listener(listener))
}

fn send_to(path: &Path, line: &str) -> Result<bool> {
    let Ok(mut running) = UnixStream::connect(path) else {
        return Ok(false);
    };
    writeln!(running, "{line}").context("o app aberto não recebeu o pedido")?;
    Ok(true)
}

/// No Linux, a pasta da sessão; no macOS, que não tem uma, a de cache do usuário.
fn path() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    let dir = dirs::runtime_dir().context("esta sessão não tem XDG_RUNTIME_DIR")?;
    #[cfg(target_os = "macos")]
    let dir = {
        let dir = dirs::cache_dir()
            .context("este sistema não informou a pasta de cache")?
            .join("kvm-switch");
        fs::create_dir_all(&dir)
            .with_context(|| format!("não consegui criar {}", dir.display()))?;
        dir
    };
    Ok(dir.join("kvm-switch.sock"))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn one_app_holds_the_socket_and_receives_the_lines() {
        let dir =
            std::env::temp_dir().join(format!("kvm-switch-test-{}-socket", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).expect("criar pasta");
        let path = dir.join("app.sock");

        assert!(!send_to(&path, "open").expect("enviar"), "sem app aberto");
        let listener = claim_at(&path).expect("primeiro app");
        assert!(claim_at(&path).is_err(), "o segundo não vira app");
        let (lines, received) = mpsc::channel();
        // A segunda tentativa de virar app conecta para conferir: chega como linha vazia.
        listener.serve(move |line| {
            line.trim().is_empty() || lines.send(line.trim().to_string()).is_ok()
        });
        assert!(send_to(&path, "switch 27").expect("enviar"));
        assert_eq!(
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("linha"),
            "switch 27"
        );
        fs::remove_dir_all(&dir).ok();
    }
}
