//! Linux: monitor por `ddcutil`, periféricos pelo `hidraw` do receptor Logitech,
//! atalhos globais pelos atalhos personalizados do GNOME e bordas por uma
//! extensão do GNOME Shell.

mod clipboard;
mod edges;
mod gnome;
mod hidraw;
mod tray;

use std::{fs, path::PathBuf, sync::Mutex};

use anyhow::{Context as _, Result, bail};

pub use self::{
    edges::GnomeEdges, gnome::GnomeHotkeys, hidraw::LogiReceiver, tray::run as run_tray,
};
use super::{mccs::capability_inputs, run};
use crate::{domain::InputCode, ports::Monitor};
pub use clipboard::GnomeClipboard;

/// O primeiro monitor que responde por DDC/CI, pelo `ddcutil`.
#[derive(Debug)]
pub struct Ddcutil {
    /// Número do barramento I2C do monitor.
    bus: Mutex<Option<String>>,
    /// Onde o barramento fica guardado entre execuções: procurar o monitor leva
    /// ~1,5 s, e no Linux o app abre de novo a cada vez que fica sem janelas.
    cache: Option<PathBuf>,
}

impl Ddcutil {
    pub fn new() -> Self {
        let cache = dirs::cache_dir().map(|dir| dir.join("kvm-switch").join("ddc-bus"));
        let bus = cache
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .map(|bus| bus.trim().to_string())
            .filter(|bus| !bus.is_empty());
        Self {
            bus: Mutex::new(bus),
            cache,
        }
    }

    /// Roda o `ddcutil` direto no barramento do monitor: ~0,2 s por chamada.
    /// Sem `--bus` ele procura o monitor a cada vez.
    fn ddc(&self, args: &[&str]) -> Result<String> {
        let mut bus = self.bus.lock().expect("lock do barramento envenenado");
        if let Some(known) = bus.as_deref()
            && let Ok(output) = on_bus(known, args)
        {
            return Ok(output);
        }
        // Primeiro uso, ou o barramento mudou (cabo em outra porta): procura de novo.
        let found = detect_bus(&run("ddcutil", &["detect", "--brief"])?)?;
        let output = on_bus(&found, args);
        if let Some(cache) = &self.cache {
            // Falhar aqui só faz a próxima execução procurar o monitor de novo.
            if let Some(dir) = cache.parent() {
                fs::create_dir_all(dir).ok();
            }
            fs::write(cache, &found).ok();
        }
        *bus = Some(found);
        output
    }
}

impl Monitor for Ddcutil {
    fn inputs(&self) -> Result<Vec<InputCode>> {
        capability_inputs(&self.ddc(&["capabilities", "--brief"])?)
    }

    fn current_input(&self) -> Result<InputCode> {
        getvcp_input(&self.ddc(&["getvcp", "60", "--brief"])?)
    }

    fn set_input(&self, input: InputCode) -> Result<()> {
        // --noverify: o monitor já está trocando de entrada e não responde à releitura.
        let value = format!("0x{:02x}", input.0);
        self.ddc(&["setvcp", "60", &value, "--noverify"]).map(drop)
    }
}

fn on_bus(bus: &str, args: &[&str]) -> Result<String> {
    // --skip-ddc-checks: sem a sondagem inicial, que custa mais que o próprio comando.
    let mut all = vec!["--bus", bus, "--skip-ddc-checks"];
    all.extend_from_slice(args);
    run("ddcutil", &all)
}

/// Barramento I2C do primeiro monitor válido na saída de `ddcutil detect --brief`.
/// Telas sem DDC/CI (a interna de um notebook) aparecem como "Invalid display".
fn detect_bus(output: &str) -> Result<String> {
    let mut valid = false;
    for line in output.lines() {
        if !line.starts_with(char::is_whitespace) {
            valid = line.starts_with("Display ");
        } else if valid && let Some(device) = line.trim().strip_prefix("I2C bus:") {
            let number = device.trim().trim_start_matches("/dev/i2c-");
            return Ok(number.to_string());
        }
    }
    bail!("nenhum monitor respondeu por DDC/CI; confira o cabo e o DDC/CI no menu do monitor")
}

/// O `ddcutil getvcp 60 --brief` responde `VCP 60 SNC x0f`: o último campo é a entrada.
fn getvcp_input(output: &str) -> Result<InputCode> {
    output
        .lines()
        .find(|line| line.starts_with("VCP 60 "))
        .and_then(|line| line.split_whitespace().last())
        .and_then(|value| u8::from_str_radix(value.strip_prefix('x')?, 16).ok())
        .map(InputCode)
        .with_context(|| format!("resposta inesperada do ddcutil: {}", output.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Saídas reais do ddcutil 2.2.5 com um Dell P3225QE em DisplayPort.
    const DETECT: &str = "\
Display 1
   I2C bus:          /dev/i2c-5
   DRM connector:    card1-DP-3
   drm_connector_id: 0
   Monitor:          DEL:DELL P3225QE:4VVLB84

";

    const CAPABILITIES: &str = "Unparsed capabilities string: (prot(monitor)type(lcd)model(P3225QE)cmds(01 02 03 07 0C E3 F3)vcp(02 04 05 08 10 12 14(05 08 0B 0C) 16 18 1A 52 60( 1B 0F 11) 87 AA(00 01 04 02) AC AE B2 B6 C6 C8 C9 CA CC(02 03 04 06 09 0A 0D 0E) D6(01 04 05) DC(00 03 05) DF E0(03) E2(00 02 04 0E 12 14) EA(F800 F801) F1 F2 FE FD)mccs_ver(2.1)mswhql(1))\n";

    #[test]
    fn finds_the_bus_of_the_monitor() {
        assert_eq!(detect_bus(DETECT).expect("barramento"), "5");
    }

    #[test]
    fn a_display_without_ddc_is_skipped() {
        // Bloco no formato que o ddcutil usa para telas que não respondem.
        let internal = "\
Invalid display
   I2C bus:          /dev/i2c-2
   DRM connector:    card1-eDP-1
   Monitor:          BOE:0x0abc:
   DDC communication failed

";
        let both = format!("{internal}{DETECT}");
        assert_eq!(detect_bus(&both).expect("barramento"), "5");
        assert!(detect_bus(internal).is_err());
    }

    #[test]
    fn lists_the_inputs_from_the_capability_string() {
        let inputs = capability_inputs(CAPABILITIES).expect("entradas");
        assert_eq!(inputs, [InputCode(0x1b), InputCode(0x0f), InputCode(0x11)]);
    }

    #[test]
    fn reads_the_current_input() {
        assert_eq!(
            getvcp_input("VCP 60 SNC x0f\n").expect("entrada"),
            InputCode(0x0f)
        );
        assert!(getvcp_input("VCP 60 ERR\n").is_err());
        assert!(getvcp_input("").is_err());
    }
}
