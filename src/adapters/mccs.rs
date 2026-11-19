//! Capability string do MCCS, o texto em que o monitor lista o que aceita por DDC/CI.

use anyhow::{Context as _, Result};

use crate::domain::InputCode;

/// Valores aceitos pelo VCP 0x60 numa capability string, como em
/// `VCP(02 04 60( 1B 0F 11) 87)`.
pub fn capability_inputs(capabilities: &str) -> Result<Vec<InputCode>> {
    let values = capabilities
        .match_indices("60(")
        .find(|(at, _)| capabilities[..*at].ends_with([' ', '(']))
        .and_then(|(at, _)| capabilities[at + 3..].split(')').next())
        .context("o monitor não informou as entradas (VCP 60)")?;
    values
        .split_whitespace()
        .map(|value| {
            let code = u8::from_str_radix(value, 16)
                .with_context(|| format!("entrada inesperada na capability string: {value}"))?;
            Ok(InputCode(code))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_string_without_input_list_is_an_error() {
        // "60" aparece só como valor de outra feature, não como a feature 0x60.
        assert!(capability_inputs("(VCP(02 14(05 60) 87))").is_err());
    }
}
