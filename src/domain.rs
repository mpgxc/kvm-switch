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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub input: InputCode,
    pub name: String,
    pub icon: String,
    /// Canal para onde teclado e mouse vão junto com a tela. `None` = não mexe neles.
    pub channel: Option<Channel>,
    pub hotkey: Option<String>,
    /// Aparece no seletor. O atalho próprio e o botão Ir funcionam mesmo desligado.
    /// Arquivos gravados antes deste campo carregam com todos os destinos ativos.
    #[serde(default = "active")]
    pub enabled: bool,
}

fn active() -> bool {
    true
}

/// Configuração gravada. Campos ausentes num arquivo antigo assumem o valor de
/// [`Config::default`], que deixa o modo Monitor compartilhado ligado e as
/// bordas desligadas: quem atualiza o app não vê nada mudar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub targets: Vec<Target>,
    /// Entrada em que este computador está ligado.
    pub local_input: Option<InputCode>,
    pub switcher_hotkey: Option<String>,
    /// Modo Monitor compartilhado: atalhos, seletor e "Ir" trocam tela e periféricos.
    pub monitor_mode: bool,
    /// Modo Bordas: o cursor na borda manda só teclado e mouse para o PC do lado.
    pub edges: Edges,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            local_input: None,
            switcher_hotkey: None,
            monitor_mode: true,
            edges: Edges::default(),
        }
    }
}

/// Lado da tela onde fica um PC vizinho.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Left, Side::Right, Side::Top, Side::Bottom];

    /// Nome do lado, o mesmo usado no pedido `edge <lado>`.
    pub fn word(self) -> &'static str {
        match self {
            Side::Left => "esquerda",
            Side::Right => "direita",
            Side::Top => "cima",
            Side::Bottom => "baixo",
        }
    }

    pub fn from_word(word: &str) -> Option<Side> {
        Side::ALL.into_iter().find(|side| side.word() == word)
    }
}

/// O PC de um lado da tela. Sem canal, aquele lado não troca nada.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Neighbor {
    pub channel: Option<Channel>,
    /// Só para mostrar no quadro da configuração.
    pub name: String,
}

/// Os vizinhos de cada lado. A borda só troca teclado e mouse, nunca a tela.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Edges {
    pub enabled: bool,
    pub left: Neighbor,
    pub right: Neighbor,
    pub top: Neighbor,
    pub bottom: Neighbor,
}

impl Edges {
    pub fn neighbor(&self, side: Side) -> &Neighbor {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
            Side::Top => &self.top,
            Side::Bottom => &self.bottom,
        }
    }

    pub fn neighbor_mut(&mut self, side: Side) -> &mut Neighbor {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
            Side::Top => &mut self.top,
            Side::Bottom => &mut self.bottom,
        }
    }

    /// Canal para onde atravessar por `side`. `None` com as bordas desligadas
    /// ou sem vizinho naquele lado.
    pub fn channel(&self, side: Side) -> Option<Channel> {
        self.enabled.then(|| self.neighbor(side).channel).flatten()
    }

    /// Os lados que trocam alguma coisa agora.
    pub fn active_sides(&self) -> Vec<Side> {
        Side::ALL
            .into_iter()
            .filter(|&side| self.channel(side).is_some())
            .collect()
    }
}

/// Ícone de um destino até alguém escolher outro.
pub const DEFAULT_ICON: &str = "🖥";

impl Config {
    /// Cria um destino para cada entrada nova. Nunca remove nem altera os existentes.
    ///
    /// Cada destino novo recebe o primeiro canal Logitech ainda livre: na ordem em
    /// que o monitor lista as entradas, a primeira fica com o canal 1, a segunda
    /// com o 2 e a terceira com o 3. É um palpite que a janela deixa trocar.
    pub fn merge_inputs(&mut self, inputs: &[InputCode]) {
        for &input in inputs {
            if self.target(input).is_none() {
                let channel = Channel::ALL.into_iter().find(|channel| {
                    self.targets
                        .iter()
                        .all(|target| target.channel != Some(*channel))
                });
                self.targets.push(Target {
                    input,
                    name: input.label(),
                    icon: DEFAULT_ICON.into(),
                    channel,
                    hotkey: None,
                    enabled: true,
                });
            }
        }
    }

    /// Registra a entrada deste computador, se ainda não houver uma, e dá ao
    /// destino dela o ícone do sistema. Um ícone que o usuário já trocou fica.
    pub fn adopt_local(&mut self, input: InputCode, icon: &str) {
        if self.local_input.is_some() {
            return;
        }
        self.local_input = Some(input);
        let local = self.targets.iter_mut().find(|target| target.input == input);
        if let Some(target) = local
            && target.icon == DEFAULT_ICON
        {
            target.icon = icon.into();
        }
    }

    pub fn target(&self, input: InputCode) -> Option<&Target> {
        self.targets.iter().find(|target| target.input == input)
    }

    /// Destinos que o seletor mostra, os ativos na ordem da lista, e a seleção
    /// inicial: o destino deste computador, para o primeiro toque já cair no
    /// vizinho. Sem ele entre os ativos, parte do último, e o primeiro toque cai
    /// no primeiro. `None` quando nenhum destino está ativo.
    pub fn switcher(&self) -> Option<(Vec<&Target>, Cycle)> {
        let targets: Vec<&Target> = self
            .targets
            .iter()
            .filter(|target| target.enabled)
            .collect();
        let here = targets
            .iter()
            .position(|target| Some(target.input) == self.local_input);
        let start = here.unwrap_or_else(|| targets.len().saturating_sub(1));
        let cycle = Cycle::new(targets.len(), start)?;
        Some((targets, cycle))
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

    /// Manda teclado e mouse para o vizinho de `side`. Nunca mexe no monitor: o
    /// PC do lado tem a própria tela. `Ok(false)` quando aquele lado não troca nada.
    pub fn cross(&self, edges: &Edges, side: Side) -> Result<bool> {
        let Some(channel) = edges.channel(side) else {
            return Ok(false);
        };
        self.peripherals.switch_to(channel).with_context(|| {
            format!(
                "teclado e mouse não foram para o canal {}",
                channel.number()
            )
        })?;
        Ok(true)
    }
}

// A geometria do cursor só existe onde o app lê o cursor (macOS e Windows): no
// GNOME quem vê a borda é a extensão.

/// Retângulo de uma tela no espaço global, com y crescendo para baixo (como no
/// macOS e no GNOME).
#[cfg(any(target_os = "macos", target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Distância da borda, em pontos, em que o cursor conta como encostado: ele para
/// no último ponto dentro da tela, um antes do limite.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
const EDGE_TOLERANCE: f64 = 1.0;
/// Perto dos cantos nada dispara, para não brigar com os cantos ativos do sistema.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
const CORNER: f64 = 8.0;

/// O lado da área de trabalho em que o cursor está encostado, se estiver. Só
/// contam as bordas externas: entre dois monitores do mesmo PC o cursor passa.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub fn edge_at(x: f64, y: f64, screens: &[Rect]) -> Option<Side> {
    let screen = screens.iter().find(|screen| screen.contains(x, y))?;
    let (left, top) = (screen.x, screen.y);
    let (right, bottom) = (left + screen.width, top + screen.height);
    let near_corner_x = x < left + CORNER || x >= right - CORNER;
    let near_corner_y = y < top + CORNER || y >= bottom - CORNER;
    // O ponto logo depois da borda: se outra tela o contém, a borda é interna.
    let candidates = [
        (
            Side::Left,
            x < left + EDGE_TOLERANCE,
            (left - EDGE_TOLERANCE, y),
            near_corner_y,
        ),
        (
            Side::Right,
            x >= right - EDGE_TOLERANCE,
            (right + EDGE_TOLERANCE, y),
            near_corner_y,
        ),
        (
            Side::Top,
            y < top + EDGE_TOLERANCE,
            (x, top - EDGE_TOLERANCE),
            near_corner_x,
        ),
        (
            Side::Bottom,
            y >= bottom - EDGE_TOLERANCE,
            (x, bottom + EDGE_TOLERANCE),
            near_corner_x,
        ),
    ];
    candidates
        .into_iter()
        .find(|&(_, touching, (beyond_x, beyond_y), in_corner)| {
            touching
                && !in_corner
                && !screens
                    .iter()
                    .any(|other| other.contains(beyond_x, beyond_y))
        })
        .map(|(side, ..)| side)
}

/// Decide quando o cursor encostado numa borda atravessa. A borda só vale
/// depois que o cursor saiu dela: quando teclado e mouse voltam, o cursor ainda
/// está parado na borda por onde saiu e não pode mandá-los embora de novo.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
#[derive(Debug, Default)]
pub struct EdgeWatch {
    armed: bool,
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
impl EdgeWatch {
    /// `at` é a borda em que o cursor está agora; `sides`, os lados com vizinho.
    /// Devolve o lado por onde atravessar.
    pub fn observe(&mut self, at: Option<Side>, sides: &[Side]) -> Option<Side> {
        let Some(side) = at else {
            self.armed = true;
            return None;
        };
        if !self.armed || !sides.contains(&side) {
            return None;
        }
        self.armed = false;
        Some(side)
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
            enabled: true,
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
    fn new_targets_take_the_free_channels_in_order() {
        let hdmi = InputCode(0x11);
        let mut config = Config::default();
        config.merge_inputs(&[USB_C, DP]);
        // O usuário passou o DP para o canal 3: o HDMI fica com o 2, que sobrou.
        config.targets[1].channel = Some(Channel::Three);
        config.merge_inputs(&[USB_C, DP, hdmi, InputCode(0x12)]);
        let channels: Vec<Option<Channel>> = config.targets.iter().map(|t| t.channel).collect();
        assert_eq!(
            channels,
            [
                Some(Channel::One),
                Some(Channel::Three),
                Some(Channel::Two),
                None
            ]
        );
    }

    #[test]
    fn this_computer_gets_the_system_icon_once() {
        let mut config = Config::default();
        config.merge_inputs(&[USB_C, DP]);
        config.adopt_local(DP, "🐧");
        assert_eq!(config.local_input, Some(DP));
        assert_eq!(config.targets[1].icon, "🐧");
        assert_eq!(config.targets[0].icon, DEFAULT_ICON);

        // Com a entrada local já registrada, outra leitura do monitor não muda nada.
        config.adopt_local(USB_C, "🐧");
        assert_eq!(config.local_input, Some(DP));
        assert_eq!(config.targets[0].icon, DEFAULT_ICON);
    }

    #[test]
    fn an_icon_the_user_chose_is_kept() {
        let mut config = Config::default();
        config.merge_inputs(&[DP]);
        config.targets[0].icon = "🎮".into();
        config.adopt_local(DP, "🐧");
        assert_eq!(config.targets[0].icon, "🎮");
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

    const HDMI: InputCode = InputCode(0x11);

    /// USB-C, DP e HDMI, com o DP desligado do seletor.
    fn without_dp(local: Option<InputCode>) -> Config {
        let mut config = Config::default();
        config.merge_inputs(&[USB_C, DP, HDMI]);
        config.targets[1].enabled = false;
        config.local_input = local;
        config
    }

    fn inputs(targets: &[&Target]) -> Vec<InputCode> {
        targets.iter().map(|target| target.input).collect()
    }

    #[test]
    fn new_targets_start_active() {
        let mut config = Config::default();
        config.merge_inputs(&[USB_C, DP]);
        assert!(config.targets.iter().all(|target| target.enabled));
    }

    #[test]
    fn switcher_skips_inactive_targets_in_list_order() {
        let config = without_dp(Some(USB_C));
        let (targets, mut cycle) = config.switcher().expect("dois ativos");
        assert_eq!(inputs(&targets), [USB_C, HDMI]);
        // Parte deste computador: o primeiro toque pula o DP e cai no HDMI.
        assert_eq!(cycle.selected(), 0);
        cycle.advance();
        assert_eq!(targets[cycle.selected()].input, HDMI);
    }

    #[test]
    fn switcher_starts_at_this_computer_counted_among_active_ones() {
        let config = without_dp(Some(HDMI));
        let (targets, cycle) = config.switcher().expect("dois ativos");
        assert_eq!(targets[cycle.selected()].input, HDMI);
        assert_eq!(cycle.selected(), 1);
    }

    #[test]
    fn inactive_or_unknown_computer_starts_as_if_absent() {
        for local in [Some(DP), None] {
            let config = without_dp(local);
            let (targets, mut cycle) = config.switcher().expect("dois ativos");
            assert_eq!(targets[cycle.selected()].input, HDMI, "{local:?}");
            cycle.advance();
            assert_eq!(targets[cycle.selected()].input, USB_C, "{local:?}");
        }
    }

    #[test]
    fn switcher_does_not_open_without_active_targets() {
        let mut config = without_dp(Some(USB_C));
        for target in &mut config.targets {
            target.enabled = false;
        }
        assert_eq!(config.switcher(), None);
        assert_eq!(Config::default().switcher(), None);
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

    fn edges_with_right(channel: Channel) -> Edges {
        let mut edges = Edges {
            enabled: true,
            ..Edges::default()
        };
        edges.right.channel = Some(channel);
        edges
    }

    fn cross(fake: Fake, edges: &Edges, side: Side) -> (Result<bool>, Vec<String>) {
        let fake = Arc::new(fake);
        let crossed = Kvm::new(fake.clone(), fake.clone()).cross(edges, side);
        let calls = fake.calls.lock().expect("lock").clone();
        (crossed, calls)
    }

    #[test]
    fn crossing_an_edge_moves_only_the_peripherals() {
        // Um monitor que falharia prova que ele nem é consultado.
        let fake = Fake {
            monitor_fails: true,
            ..Fake::default()
        };
        let (crossed, calls) = cross(fake, &edges_with_right(Channel::Two), Side::Right);
        assert!(crossed.expect("travessia"));
        assert_eq!(calls, ["canal 2"]);
    }

    #[test]
    fn a_side_without_neighbor_or_disabled_edges_does_nothing() {
        let edges = edges_with_right(Channel::Two);
        let (crossed, calls) = cross(Fake::default(), &edges, Side::Left);
        assert!(!crossed.expect("travessia"));
        assert!(calls.is_empty(), "{calls:?}");

        let disabled = Edges {
            enabled: false,
            ..edges
        };
        let (crossed, calls) = cross(Fake::default(), &disabled, Side::Right);
        assert!(!crossed.expect("travessia"));
        assert!(calls.is_empty(), "{calls:?}");
        assert!(disabled.active_sides().is_empty());
    }

    #[test]
    fn file_from_before_the_modes_keeps_the_monitor_and_leaves_edges_off() {
        let config: Config =
            serde_json::from_str(r#"{"targets": [], "local_input": 27, "switcher_hotkey": null}"#)
                .expect("arquivo antigo");
        assert!(config.monitor_mode);
        assert_eq!(config.edges, Edges::default());
        assert!(!config.edges.enabled);
    }

    #[test]
    fn side_words_round_trip() {
        for side in Side::ALL {
            assert_eq!(Side::from_word(side.word()), Some(side));
        }
        assert_eq!(Side::from_word("left"), None);
    }

    const MAIN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };

    #[test]
    fn the_cursor_stops_one_point_before_each_outer_edge() {
        let screens = [MAIN];
        assert_eq!(edge_at(0.0, 500.0, &screens), Some(Side::Left));
        assert_eq!(edge_at(1919.0, 500.0, &screens), Some(Side::Right));
        assert_eq!(edge_at(900.0, 0.0, &screens), Some(Side::Top));
        assert_eq!(edge_at(900.0, 1079.0, &screens), Some(Side::Bottom));
        assert_eq!(edge_at(900.0, 500.0, &screens), None);
        assert_eq!(edge_at(1917.5, 500.0, &screens), None);
    }

    #[test]
    fn corners_do_not_trigger() {
        let screens = [MAIN];
        assert_eq!(edge_at(1919.0, 0.0, &screens), None);
        assert_eq!(edge_at(1919.0, 4.0, &screens), None);
        assert_eq!(edge_at(0.0, 1079.0, &screens), None);
        assert_eq!(edge_at(3.0, 0.0, &screens), None);
    }

    #[test]
    fn the_edge_between_two_screens_of_this_pc_is_not_a_crossing() {
        // Segundo monitor à direita, mais baixo e começando acima do principal.
        let side_by_side = [
            MAIN,
            Rect {
                x: 1920.0,
                y: -200.0,
                width: 2560.0,
                height: 1440.0,
            },
        ];
        assert_eq!(edge_at(1919.0, 500.0, &side_by_side), None);
        assert_eq!(edge_at(1920.0, 500.0, &side_by_side), None);
        assert_eq!(edge_at(4479.0, 500.0, &side_by_side), Some(Side::Right));
        assert_eq!(edge_at(0.0, 500.0, &side_by_side), Some(Side::Left));
        // Acima do principal não há tela; acima do segundo, a borda dele.
        assert_eq!(edge_at(900.0, 0.0, &side_by_side), Some(Side::Top));
        assert_eq!(edge_at(2500.0, -200.0, &side_by_side), Some(Side::Top));

        let stacked = [
            MAIN,
            Rect {
                x: 0.0,
                y: -1080.0,
                width: 1920.0,
                height: 1080.0,
            },
        ];
        assert_eq!(edge_at(900.0, 0.0, &stacked), None);
        assert_eq!(edge_at(900.0, -1080.0, &stacked), Some(Side::Top));
        assert_eq!(edge_at(900.0, 1079.0, &stacked), Some(Side::Bottom));
    }

    #[test]
    fn the_edge_only_fires_after_the_cursor_left_it() {
        let sides = [Side::Right];
        let mut watch = EdgeWatch::default();
        // Ao abrir o app o cursor já está na borda: não atravessa.
        assert_eq!(watch.observe(Some(Side::Right), &sides), None);
        assert_eq!(watch.observe(None, &sides), None);
        assert_eq!(watch.observe(Some(Side::Right), &sides), Some(Side::Right));
        // Teclado e mouse voltaram e o cursor continua parado na borda.
        assert_eq!(watch.observe(Some(Side::Right), &sides), None);
        assert_eq!(watch.observe(Some(Side::Right), &sides), None);
        assert_eq!(watch.observe(None, &sides), None);
        assert_eq!(watch.observe(Some(Side::Right), &sides), Some(Side::Right));
    }

    #[test]
    fn a_side_without_neighbor_never_fires() {
        let mut watch = EdgeWatch::default();
        watch.observe(None, &[Side::Right]);
        assert_eq!(watch.observe(Some(Side::Bottom), &[Side::Right]), None);
        assert_eq!(
            watch.observe(Some(Side::Right), &[Side::Right]),
            Some(Side::Right)
        );
    }
}
