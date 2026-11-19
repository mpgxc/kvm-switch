//! Periféricos Logitech pelo receptor (Bolt ou Unifying), falando HID++ 2.0
//! direto no `hidraw`. São os mesmos pacotes de `linux/logi-host`.
//!
//! O acesso ao `hidraw` vem da regra udev do Solaar ou de `linux/70-logi-bolt.rules`.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    os::fd::AsRawFd as _,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail};

use crate::{domain::Channel, ports::Peripherals};

/// Quantos periféricos um receptor pareia.
const SLOTS: u8 = 6;
/// Relatório longo do HID++: 20 bytes.
const LONG_REPORT: u8 = 0x11;
const REPORT_LEN: usize = 20;
/// Cabe o maior relatório que o receptor manda, o muito longo do HID++, de 64 bytes.
const READ_BUFFER_LEN: usize = 64;
/// Identifica as nossas perguntas entre as de outros programas (Solaar, por exemplo).
const SOFTWARE_ID: u8 = 0x0e;
/// Índice da feature IRoot, o mesmo em todo dispositivo. A função 0 dela,
/// `getFeature`, diz em que índice fica outra feature.
const IROOT: u8 = 0x00;
/// Identificador da feature CHANGE_HOST, a do Easy-Switch.
const CHANGE_HOST: [u8; 2] = [0x18, 0x14];
/// Função `setCurrentHost` de CHANGE_HOST, no nibble alto do byte que leva o `SOFTWARE_ID`.
const SET_CURRENT_HOST: u8 = 0x10;
/// Um periférico em repouso leva até ~1 s para acordar e responder.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct LogiReceiver;

impl Peripherals for LogiReceiver {
    fn switch_to(&self, channel: Channel) -> Result<()> {
        let receivers = receivers()?;
        if receivers.is_empty() {
            bail!("nenhum receptor Logitech nesta máquina");
        }
        // Um slot por thread: os que estão em repouso não atrasam os outros.
        let moved = thread::scope(|scope| {
            let slots: Vec<_> = receivers
                .iter()
                .flat_map(|receiver| (1..=SLOTS).map(move |slot| (receiver, slot)))
                .map(|(receiver, slot)| scope.spawn(move || switch_slot(receiver, slot, channel)))
                .collect();
            slots
                .into_iter()
                .map(|slot| slot.join().expect("a thread do slot entrou em pânico"))
                .collect::<Result<Vec<bool>>>()
        })?;
        if !moved.contains(&true) {
            bail!("nenhum periférico Logitech respondeu no receptor");
        }
        Ok(())
    }
}

/// Manda o periférico do slot para o canal. `false` se o slot está vazio, o
/// periférico está desligado ou ele não troca de canal.
fn switch_slot(receiver: &Path, slot: u8, channel: Channel) -> Result<bool> {
    // Um arquivo por thread: cada um recebe a sua cópia das respostas do receptor.
    let mut device = OpenOptions::new()
        .read(true)
        .write(true)
        .open(receiver)
        .with_context(|| {
            format!(
                "sem acesso a {}; instale o Solaar ou a regra linux/70-logi-bolt.rules",
                receiver.display()
            )
        })?;
    send(&mut device, &change_host_query(slot))?;
    let deadline = Instant::now() + REPLY_TIMEOUT;
    let mut report = [0u8; READ_BUFFER_LEN];
    let index = loop {
        let len = read_until(&device, deadline, &mut report)?;
        match feature_reply(&report[..len], slot) {
            Reply::Index(index) => break index,
            Reply::NoDevice => return Ok(false),
            Reply::Unrelated => {}
        }
    };
    // O periférico desconecta em seguida, sem responder.
    send(&mut device, &set_host_command(slot, index, channel))?;
    Ok(true)
}

/// IRoot.getFeature(CHANGE_HOST): o índice da feature muda de um dispositivo
/// para outro, então é perguntado a cada troca.
fn change_host_query(slot: u8) -> [u8; REPORT_LEN] {
    let [high, low] = CHANGE_HOST;
    long_report(&[slot, IROOT, SOFTWARE_ID, high, low])
}

/// CHANGE_HOST.setCurrentHost(host), no índice que o periférico informou.
fn set_host_command(slot: u8, index: u8, channel: Channel) -> [u8; REPORT_LEN] {
    long_report(&[
        slot,
        index,
        SET_CURRENT_HOST | SOFTWARE_ID,
        channel.host_index(),
    ])
}

fn long_report(payload: &[u8]) -> [u8; REPORT_LEN] {
    let mut report = [0u8; REPORT_LEN];
    report[0] = LONG_REPORT;
    report[1..=payload.len()].copy_from_slice(payload);
    report
}

fn send(device: &mut File, report: &[u8; REPORT_LEN]) -> Result<()> {
    device
        .write_all(report)
        .context("o receptor Logitech não aceitou o comando")
}

/// Lê o próximo relatório. Devolve 0 bytes se o prazo acabar antes.
fn read_until(device: &File, deadline: Instant, report: &mut [u8]) -> Result<usize> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Ok(0);
    }
    let mut poll = libc::pollfd {
        fd: device.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = i32::try_from(left.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: `poll` aponta para uma única `pollfd` válida durante toda a chamada.
    let ready = unsafe { libc::poll(&mut poll, 1, millis) };
    match ready {
        0 => Ok(0),
        1.. => {
            let mut device = device;
            device
                .read(report)
                .context("não consegui ler a resposta do receptor Logitech")
        }
        _ => Err(io::Error::last_os_error()).context("esperando o receptor Logitech"),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Reply {
    /// Índice da feature CHANGE_HOST neste periférico.
    Index(u8),
    /// Slot vazio, periférico desligado ou sem troca de canal. Também vale para
    /// o prazo esgotado, que chega como relatório vazio.
    NoDevice,
    /// Relatório de outro slot ou de outra conversa: continue lendo.
    Unrelated,
}

/// Interpreta um relatório lido depois de perguntar o índice de CHANGE_HOST ao slot.
fn feature_reply(report: &[u8], slot: u8) -> Reply {
    match report {
        [] => Reply::NoDevice,
        [LONG_REPORT, from, IROOT, SOFTWARE_ID, index, ..] if *from == slot => match index {
            0 => Reply::NoDevice,
            index => Reply::Index(*index),
        },
        // Erro do HID++ 1.0 (slot vazio ou periférico desligado) ou do 2.0 à nossa pergunta.
        [_, from, 0x8f | 0xff, IROOT, SOFTWARE_ID, ..] if *from == slot => Reply::NoDevice,
        _ => Reply::Unrelated,
    }
}

/// Os `hidraw` que falam HID++ com um receptor Logitech.
fn receivers() -> Result<Vec<PathBuf>> {
    let entries = fs::read_dir("/sys/class/hidraw").context("este sistema não tem hidraw")?;
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let device = entry.path().join("device");
        let (Ok(uevent), Ok(descriptor)) = (
            fs::read_to_string(device.join("uevent")),
            fs::read(device.join("report_descriptor")),
        ) else {
            continue;
        };
        if is_hidpp_receiver(&uevent, &descriptor) {
            found.push(Path::new("/dev").join(entry.file_name()));
        }
    }
    Ok(found)
}

/// O receptor expõe várias interfaces HID; a do HID++ é a que usa a página de
/// uso do fabricante (0xFF00).
fn is_hidpp_receiver(uevent: &str, descriptor: &[u8]) -> bool {
    let uevent = uevent.to_ascii_uppercase();
    uevent.contains("HID_ID=0003:0000046D:")
        && uevent.contains("RECEIVER")
        && descriptor.starts_with(&[0x06, 0x00, 0xff])
}

#[cfg(test)]
mod tests {
    use super::*;

    // Respostas reais de um receptor Bolt com MX Master 3S (slot 1), um teclado
    // desligado (slot 2), MX Mechanical Mini (slot 3) e o slot 4 vazio.
    const MOUSE: [u8; 20] = [
        0x11, 0x01, 0x00, 0x0e, 0x0a, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    const KEYBOARD: [u8; 20] = [
        0x11, 0x03, 0x00, 0x0e, 0x09, 0x00, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    const POWERED_OFF: [u8; 7] = [0x10, 0x02, 0x8f, 0x00, 0x0e, 0x04, 0x00];
    const EMPTY_SLOT: [u8; 7] = [0x10, 0x04, 0x8f, 0x00, 0x0e, 0x09, 0x00];

    #[test]
    fn the_packets_are_the_ones_logi_host_writes() {
        // 0x11, slot, índice da feature, função | 0x0e, parâmetros, zeros até 20 bytes.
        assert_eq!(
            change_host_query(3),
            [
                0x11, 0x03, 0x00, 0x0e, 0x18, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        assert_eq!(
            set_host_command(3, 0x09, Channel::Two),
            [
                0x11, 0x03, 0x09, 0x1e, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn reads_the_feature_index_of_each_device() {
        assert_eq!(feature_reply(&MOUSE, 1), Reply::Index(0x0a));
        assert_eq!(feature_reply(&KEYBOARD, 3), Reply::Index(0x09));
    }

    #[test]
    fn powered_off_empty_and_silent_slots_have_no_device() {
        assert_eq!(feature_reply(&POWERED_OFF, 2), Reply::NoDevice);
        assert_eq!(feature_reply(&EMPTY_SLOT, 4), Reply::NoDevice);
        assert_eq!(feature_reply(&[], 1), Reply::NoDevice);
    }

    #[test]
    fn replies_to_other_slots_are_skipped() {
        // As threads leem o mesmo receptor: a resposta do mouse chega também a quem perguntou ao teclado.
        assert_eq!(feature_reply(&MOUSE, 3), Reply::Unrelated);
        assert_eq!(feature_reply(&EMPTY_SLOT, 1), Reply::Unrelated);
    }

    #[test]
    fn only_the_hidpp_interface_of_the_receiver_is_used() {
        // uevent real da interface HID++ de um receptor Bolt.
        let uevent =
            "DRIVER=hid-generic\nHID_ID=0003:0000046D:0000C548\nHID_NAME=Logitech USB Receiver\n";
        assert!(is_hidpp_receiver(uevent, &[0x06, 0x00, 0xff, 0x09, 0x01]));
        // A interface de teclado do mesmo receptor.
        assert!(!is_hidpp_receiver(uevent, &[0x05, 0x01, 0x09, 0x06]));
        // Um headset Logitech também usa a página 0xFF00, mas não é receptor.
        let headset = "HID_ID=0003:0000046D:00000ADF\nHID_NAME=G435 Wireless Gaming Headset\n";
        assert!(!is_hidpp_receiver(headset, &[0x06, 0x00, 0xff]));
    }
}
