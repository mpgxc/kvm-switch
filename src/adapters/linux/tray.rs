//! `kvm-switch tray`: o ícone na barra do GNOME, num processo pequeno à parte.
//!
//! O GPUI encerra o app com a última janela, então o ícone não pode morar nele.
//! Este processo nunca abre o GPUI: cada clique roda `kvm-switch <pedido>`, como
//! um atalho do GNOME, e o pedido segue pelo mesmo caminho (socket do app aberto
//! ou troca sem janela).

use std::{
    env,
    fs::{self, File},
    os::fd::AsRawFd as _,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, Result, bail};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent, menu::MenuEvent};

use crate::{config, domain::Config, ports::Action, request, tray};

/// De quanto em quanto tempo o `config.json` é conferido, para o menu acompanhar a janela.
const CONFIG_POLL: Duration = Duration::from_secs(2);

enum Click {
    /// O pedido do item de menu, como em `request`.
    Item(String),
    /// Clique duplo no ícone (no GNOME o simples abre o menu).
    Icon,
}

/// Roda o ícone até "Encerrar".
pub fn run() -> Result<()> {
    // Mantido aberto até o fim: o lock é do arquivo aberto.
    let _lock = single_instance()?;
    let config_path = config::default_path()?;
    let (config, mut seen) = read(&config_path);
    let icon = TrayIconBuilder::new()
        .with_id("kvm-switch")
        .with_title("KVM Switch")
        .with_tooltip("KVM Switch")
        .with_icon(tray::icon()?)
        .with_menu(Box::new(tray::menu(&config)?))
        .build()
        .context("não consegui criar o ícone na barra")?;

    let (clicks, received) = mpsc::channel();
    let menu_clicks = clicks.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        menu_clicks.send(Click::Item(event.id.0)).ok();
    }));
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            clicks.send(Click::Icon).ok();
        }
    }));

    loop {
        match received.recv_timeout(CONFIG_POLL) {
            Ok(Click::Icon) => ask(Action::OpenSettings),
            Ok(Click::Item(id)) => match request::parse(&id) {
                Ok(Action::Quit) => {
                    // Fecha também o app, se estiver aberto, e espera para não deixar zumbi.
                    if let Ok(mut child) = command(Action::Quit).spawn() {
                        child.wait().ok();
                    }
                    return Ok(());
                }
                Ok(action) => ask(action),
                Err(error) => eprintln!("kvm-switch tray: {error:#}"),
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let modified = modified(&config_path);
                if modified != seen {
                    let (config, now) = read(&config_path);
                    seen = now;
                    icon.set_menu(Some(Box::new(tray::menu(&config)?)));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// A configuração e a data dela. Arquivo ausente ou inválido vira um menu sem destinos:
/// quem avisa do erro é a janela.
fn read(path: &Path) -> (Config, Option<SystemTime>) {
    let modified = modified(path);
    let config = if path.exists() {
        config::load(path).unwrap_or_default()
    } else {
        Config::default()
    };
    (config, modified)
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// `kvm-switch <pedido>`, este mesmo programa.
fn command(action: Action) -> Command {
    let program = env::current_exe().map_or_else(
        |_| PathBuf::from("kvm-switch"),
        // Reinstalado com o ícone aberto, o Linux mostra o caminho antigo com
        // " (deleted)"; o programa novo está no mesmo lugar.
        |path| match path
            .to_str()
            .and_then(|path| path.strip_suffix(" (deleted)"))
        {
            Some(installed) => PathBuf::from(installed),
            None => path,
        },
    );
    let mut command = Command::new(program);
    command.args(request::arguments(action).split_whitespace());
    command
}

/// Faz o pedido sem travar o menu; uma thread espera o processo, para não sobrar zumbi.
fn ask(action: Action) {
    match command(action).spawn() {
        Ok(mut child) => {
            thread::spawn(move || child.wait());
        }
        Err(error) => eprintln!("kvm-switch tray: não consegui rodar o pedido: {error}"),
    }
}

/// Um ícone por sessão: um segundo `kvm-switch tray` sai com erro.
fn single_instance() -> Result<File> {
    let dir = dirs::runtime_dir().context("esta sessão não tem XDG_RUNTIME_DIR")?;
    let path = dir.join("kvm-switch-tray.lock");
    let file =
        File::create(&path).with_context(|| format!("não consegui criar {}", path.display()))?;
    // SAFETY: o descritor pertence a `file`, que segue aberto durante a chamada.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    if !locked {
        bail!("o ícone do kvm-switch já está na barra");
    }
    Ok(file)
}
