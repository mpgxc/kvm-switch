//! Regras do KVM que não dependem de sistema operacional.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::ports::{Monitor, Peripherals};

/// Valor do código VCP 0x60 (fonte de entrada) do DDC/CI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InputCode(pub u8);

impl InputCode {
    /// Nome usual da entrada: tabela MCCS, mais o 0x1B que a Dell usa para USB-C.
    pub fn label(self) -> String {
        match self.0 {
            0x0f => "DP".into(),
            0x10 => "DP 2".into(),
            0x11 => "HDMI".into(),
            0x12 => "HDMI 2".into(),
            0x1b => "USB-C".into(),
            other => format!("0x{other:02X}"),
        }
    }
}

/// Canal Easy-Switch dos periféricos Logitech.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum Channel {
    One,
    Two,
    Three,
}

impl Channel {
    pub const ALL: [Channel; 3] = [Channel::One, Channel::Two, Channel::Three];

    /// Número impresso no botão do periférico (1 a 3).
    pub fn number(self) -> u8 {
        self.host_index() + 1
    }

    /// Índice de host do HID++, que conta a partir de zero.
    pub fn host_index(self) -> u8 {
        match self {
            Channel::One => 0,
            Channel::Two => 1,
            Channel::Three => 2,
        }
    }
}

impl From<Channel> for u8 {
    fn from(channel: Channel) -> u8 {
        channel.number()
    }
}

impl TryFrom<u8> for Channel {
    type Error = String;

    fn try_from(number: u8) -> Result<Self, String> {
        Channel::ALL
            .into_iter()
            .find(|channel| channel.number() == number)
            .ok_or_else(|| format!("canal {number} não existe (use 1, 2 ou 3)"))
    }
}

/// Um computador ligado a uma entrada do monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub input: InputCode,
    pub name: String,
    pub icon: String,
    /// Canal para onde teclado e mouse vão junto com a tela. `None` = não mexe neles.
    pub channel: Option<Channel>,
    pub hotkey: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub targets: Vec<Target>,
    /// Entrada em que este computador está ligado.
    pub local_input: Option<InputCode>,
    pub switcher_hotkey: Option<String>,
}

impl Config {
    /// Cria um destino para cada entrada nova. Nunca remove nem altera os existentes.
    pub fn merge_inputs(&mut self, inputs: &[InputCode]) {
        for &input in inputs {
            if self.target(input).is_none() {
                self.targets.push(Target {
                    input,
                    name: input.label(),
                    icon: "🖥".into(),
                    channel: None,
                    hotkey: None,
                });
            }
        }
    }

    pub fn target(&self, input: InputCode) -> Option<&Target> {
        self.targets.iter().find(|target| target.input == input)
    }
}

/// Seleção do seletor de destinos, que dá a volta nas pontas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cycle {
    len: usize,
    selected: usize,
}

impl Cycle {
    /// `None` quando não há destinos.
    pub fn new(len: usize, selected: usize) -> Option<Self> {
        if len == 0 {
            return None;
        }
        Some(Self {
            len,
            selected: selected % len,
        })
    }

    pub fn advance(&mut self) {
        self.selected = (self.selected + 1) % self.len;
    }

    pub fn back(&mut self) {
        self.selected = (self.selected + self.len - 1) % self.len;
    }

    pub fn selected(&self) -> usize {
        self.selected
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Switched,
    /// O destino pedido é este computador e a tela já está nele.
    AlreadyHere,
}

/// O que o monitor informou na detecção.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub inputs: Vec<InputCode>,
    /// `None` quando o monitor não respondeu a entrada atual.
    pub current: Option<InputCode>,
}

/// Caso de uso da troca: monitor e periféricos juntos.
#[derive(Clone)]
pub struct Kvm {
    monitor: Arc<dyn Monitor>,
    peripherals: Arc<dyn Peripherals>,
}

impl Kvm {
    pub fn new(monitor: Arc<dyn Monitor>, peripherals: Arc<dyn Peripherals>) -> Self {
        Self {
            monitor,
            peripherals,
        }
    }

    pub fn detect(&self) -> Result<Detected> {
        let inputs = self.monitor.inputs()?;
        let current = self.monitor.current_input().ok();
        Ok(Detected { inputs, current })
    }

    /// Leva tela e periféricos para `target`. `local` é a entrada deste computador.
    pub fn switch(&self, target: &Target, local: Option<InputCode>) -> Result<Outcome> {
        let label = target.input.label();
        if local == Some(target.input) {
            // O atalho só chega em quem está com o teclado: os periféricos já estão aqui.
            // Resta trazer a tela de volta se ela ficou em outra entrada.
            if self.monitor.current_input().ok() == Some(target.input) {
                return Ok(Outcome::AlreadyHere);
            }
            self.set_input(target.input, &label)?;
            return Ok(Outcome::Switched);
        }

        // Periféricos só saem depois do monitor: se ele falhar, teclado e mouse ficam aqui.
        self.set_input(target.input, &label)?;
        if let Some(channel) = target.channel {
            self.peripherals
                .switch_to(channel)
                .with_context(|| format!("o monitor foi para {label}, mas os periféricos não"))?;
        }
        Ok(Outcome::Switched)
    }

    fn set_input(&self, input: InputCode, label: &str) -> Result<()> {
        self.monitor
            .set_input(input)
            .with_context(|| format!("o monitor não trocou para {label}"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use anyhow::bail;

    use super::*;

    const USB_C: InputCode = InputCode(0x1b);
    const DP: InputCode = InputCode(0x0f);

    /// Monitor e periféricos de mentira que anotam as chamadas na ordem.
    #[derive(Default)]
    struct Fake {
        current: Option<InputCode>,
        monitor_fails: bool,
        calls: Mutex<Vec<String>>,
    }

    impl Monitor for Fake {
        fn inputs(&self) -> Result<Vec<InputCode>> {
            Ok(vec![USB_C, DP])
        }

        fn current_input(&self) -> Result<InputCode> {
            self.current.context("sem resposta")
        }

        fn set_input(&self, input: InputCode) -> Result<()> {
            if self.monitor_fails {
                bail!("sem monitor");
            }
            self.calls
                .lock()
                .expect("lock")
                .push(format!("monitor {}", input.label()));
            Ok(())
        }
    }

    impl Peripherals for Fake {
        fn switch_to(&self, channel: Channel) -> Result<()> {
            self.calls
                .lock()
                .expect("lock")
                .push(format!("canal {}", channel.number()));
            Ok(())
        }
    }

    fn switch(
        fake: Fake,
        target: &Target,
        local: Option<InputCode>,
    ) -> (Result<Outcome>, Vec<String>) {
        let fake = Arc::new(fake);
        let outcome = Kvm::new(fake.clone(), fake.clone()).switch(target, local);
        let calls = fake.calls.lock().expect("lock").clone();
        (outcome, calls)
    }

    fn target(input: InputCode, channel: Option<Channel>) -> Target {
        Target {
            input,
            name: "x".into(),
            icon: "🖥".into(),
            channel,
            hotkey: None,
        }
    }

    #[test]
    fn remote_target_moves_monitor_then_peripherals() {
        let fake = Fake {
            current: Some(USB_C),
            ..Fake::default()
        };
        let (outcome, calls) = switch(fake, &target(DP, Some(Channel::Two)), Some(USB_C));
        assert_eq!(outcome.expect("troca"), Outcome::Switched);
        assert_eq!(calls, ["monitor DP", "canal 2"]);
    }

    #[test]
    fn peripherals_stay_when_monitor_fails() {
        let fake = Fake {
            monitor_fails: true,
            ..Fake::default()
        };
        let (outcome, calls) = switch(fake, &target(DP, Some(Channel::Two)), Some(USB_C));
        assert!(outcome.is_err());
        assert!(calls.is_empty(), "nada deve ser enviado: {calls:?}");
    }

    #[test]
    fn target_without_channel_leaves_peripherals_alone() {
        let (_, calls) = switch(Fake::default(), &target(DP, None), Some(USB_C));
        assert_eq!(calls, ["monitor DP"]);
    }

    #[test]
    fn local_target_already_on_screen_does_nothing() {
        let fake = Fake {
            current: Some(USB_C),
            ..Fake::default()
        };
        let (outcome, calls) = switch(fake, &target(USB_C, Some(Channel::One)), Some(USB_C));
        assert_eq!(outcome.expect("troca"), Outcome::AlreadyHere);
        assert!(calls.is_empty(), "{calls:?}");
    }

    #[test]
    fn local_target_brings_only_the_screen_back() {
        let fake = Fake {
            current: Some(DP),
            ..Fake::default()
        };
        let (outcome, calls) = switch(fake, &target(USB_C, Some(Channel::One)), Some(USB_C));
        assert_eq!(outcome.expect("troca"), Outcome::Switched);
        assert_eq!(calls, ["monitor USB-C"]);
    }

    #[test]
    fn merge_adds_new_inputs_and_keeps_edited_targets() {
        let mut config = Config::default();
        config.merge_inputs(&[USB_C]);
        config.targets[0].name = "Mac".into();
        config.merge_inputs(&[USB_C, DP]);
        let names: Vec<&str> = config.targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Mac", "DP"]);
    }

    #[test]
    fn cycle_wraps_in_both_directions() {
        let mut cycle = Cycle::new(3, 2).expect("três destinos");
        cycle.advance();
        assert_eq!(cycle.selected(), 0);
        cycle.back();
        cycle.back();
        assert_eq!(cycle.selected(), 1);
        assert_eq!(Cycle::new(0, 0), None);
    }

    #[test]
    fn channel_is_stored_as_its_button_number() {
        let json = serde_json::to_string(&Channel::Two).expect("json");
        assert_eq!(json, "2");
        assert_eq!(
            serde_json::from_str::<Channel>("3").expect("canal"),
            Channel::Three
        );
        assert!(serde_json::from_str::<Channel>("4").is_err());
    }
}
