//! Periféricos Logitech pelo receptor Bolt, falando HID++ 2.0 com o `hidapitester`.
//! São os mesmos comandos de `macos/kvm-switch.sh`.

use anyhow::{Result, bail};

use super::run;
use crate::{domain::Channel, ports::Peripherals};

/// Vendor:product USB do receptor Logi Bolt.
const RECEIVER: &str = "046D:C548";
/// Quantos periféricos um receptor Bolt pareia.
const SLOTS: u8 = 6;

pub struct LogiBolt;

impl Peripherals for LogiBolt {
    fn switch_to(&self, channel: Channel) -> Result<()> {
        let mut moved = 0;
        for slot in 1..=SLOTS {
            // IRoot.getFeature(0x1814 CHANGE_HOST): o índice da feature muda de um
            // dispositivo para outro, então é perguntado a cada troca.
            // A 1ª leitura espera o dispositivo acordar; a 2ª cobre uma resposta
            // atrasada de outro slot que chegue na frente.
            let probe = hidpp(
                &format!("{slot},0x00,0x0e,0x18,0x14"),
                &[
                    "--timeout",
                    "1000",
                    "--read-input",
                    "--timeout",
                    "100",
                    "--read-input",
                ],
            )?;
            if let Some(index) = change_host_index(&probe, slot) {
                // setCurrentHost(host)
                hidpp(
                    &format!("{slot},0x{index:02x},0x1e,{}", channel.host_index()),
                    &[],
                )?;
                moved += 1;
            }
        }
        if moved == 0 {
            bail!("nenhum periférico Logitech neste receptor Bolt");
        }
        Ok(())
    }
}

/// Envia um relatório longo HID++ (0x11, software id 0xe) ao receptor.
fn hidpp(payload: &str, reads: &[&str]) -> Result<String> {
    let report = format!("0x11,{payload}");
    let mut args = vec![
        "--vidpid",
        RECEIVER,
        "--usagePage",
        "0xFF00",
        "--usage",
        "2",
        "--open",
        "--length",
        "20",
        "--send-output",
        &report,
    ];
    args.extend_from_slice(reads);
    run("hidapitester", &args)
}

/// Índice da feature CHANGE_HOST na resposta do `hidapitester`, se o slot tiver
/// um dispositivo que a suporte.
fn change_host_index(output: &str, slot: u8) -> Option<u8> {
    output
        .lines()
        // Só vale o que vem depois de "Reading": antes disso o programa ecoa o pacote enviado.
        .skip_while(|line| !line.contains("Reading"))
        .find_map(|line| {
            let bytes: Vec<u8> = line
                .split_whitespace()
                .map(|byte| u8::from_str_radix(byte, 16))
                .collect::<Result<_, _>>()
                .ok()?;
            match bytes.as_slice() {
                [0x11, from, 0x00, 0x0e, index, ..] if *from == slot && *index != 0 => Some(*index),
                _ => None,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Saídas reais do hidapitester 0.7 com MX Mechanical Mini (slot 1) e MX Master 3S (slot 2).
    const ANSWERED: &str = "\
Opening device, vid/pid:0x046D/0xC548, usagePage/usage: FF00/2
Device opened
Writing output report of 20-bytes...wrote 20 bytes:
 11 01 00 0E 18 14 00 00 00 00 00 00 00 00 00 00 00 00 00 00
Reading up to 20-byte input report, 500 msec timeout...read 20 bytes:
 11 01 00 0E 09 00 02 00 00 00 00 00 00 00 00 00 00 00 00 00
Reading up to 20-byte input report, 500 msec timeout...read 0 bytes:
 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
Closing device
";

    const EMPTY_SLOT: &str = "\
Opening device, vid/pid:0x046D/0xC548, usagePage/usage: FF00/2
Device opened
Writing output report of 20-bytes...wrote 20 bytes:
 11 03 00 0E 18 14 00 00 00 00 00 00 00 00 00 00 00 00 00 00
Reading up to 20-byte input report, 500 msec timeout...read 7 bytes:
 10 03 8F 00 0E 09 00 00 00 00 00 00 00 00 00 00 00 00 00 00
Reading up to 20-byte input report, 500 msec timeout...read 0 bytes:
 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
Closing device
";

    #[test]
    fn reads_the_feature_index_from_the_reply() {
        assert_eq!(change_host_index(ANSWERED, 1), Some(0x09));
    }

    #[test]
    fn the_echoed_request_is_not_a_reply() {
        // Sem resposta, a única linha "11 01 00 0E .." é o eco do pedido, cujo byte 5 é 0x18.
        let no_reply = ANSWERED.replace("11 01 00 0E 09", "00 00 00 00 00");
        assert_eq!(change_host_index(&no_reply, 1), None);
    }

    #[test]
    fn empty_slot_and_other_slots_replies_are_ignored() {
        assert_eq!(change_host_index(EMPTY_SLOT, 3), None);
        assert_eq!(change_host_index(ANSWERED, 2), None);
    }
}
