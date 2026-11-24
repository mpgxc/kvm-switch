//! Bordas onde o app consegue ler a posição do cursor (macOS e Windows): uma
//! thread consulta o cursor e as telas e passa ao `EdgeWatch` do domínio. Cada
//! sistema só fornece a função que lê a posição.

use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use async_channel::Sender;

use crate::{
    domain::{EdgeWatch, Rect, Side, edge_at},
    ports::{Action, EdgeSensor},
};

/// De quanto em quanto tempo o cursor é consultado com o modo Bordas ligado.
const EDGE_POLL: Duration = Duration::from_millis(20);

/// Posição do cursor e retângulos das telas, no mesmo espaço global, com y para baixo.
pub type Sample = fn() -> Option<(f64, f64, Vec<Rect>)>;

/// Vigia o cursor numa thread própria. Só consulta a posição, sem ouvir eventos,
/// então não pede permissão (Monitoramento de Entrada no macOS, gancho no Windows).
pub struct PollingEdges {
    sides: Arc<Mutex<Vec<Side>>>,
}

impl PollingEdges {
    pub fn new(actions: Sender<Action>, sample: Sample) -> Self {
        let sides: Arc<Mutex<Vec<Side>>> = Arc::default();
        let watched = sides.clone();
        thread::spawn(move || {
            let mut watch = EdgeWatch::default();
            loop {
                thread::sleep(EDGE_POLL);
                let sides = watched.lock().expect("lock das bordas envenenado").clone();
                if sides.is_empty() {
                    // Ao religar, o cursor pode estar parado numa borda: começa desarmado.
                    watch = EdgeWatch::default();
                    continue;
                }
                let Some((x, y, screens)) = sample() else {
                    continue;
                };
                if let Some(side) = watch.observe(edge_at(x, y, &screens), &sides) {
                    // O receptor só some quando o app está encerrando.
                    actions.try_send(Action::CrossEdge(side)).ok();
                }
            }
        });
        Self { sides }
    }
}

impl EdgeSensor for PollingEdges {
    fn watch(&mut self, sides: &[Side]) -> Option<String> {
        *self.sides.lock().expect("lock das bordas envenenado") = sides.to_vec();
        None
    }
}
