//! Общая грамматика component-v3 IDs для host и worker.

use anyhow::{Context, Result, bail};

/// Сторона, выделившая ID: `h` для host, `m` для module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireDirection {
    Host,
    Module,
}

/// Разобранный ID без проверки текущего состояния transport/invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireId {
    pub direction: WireDirection,
    pub generation: u64,
    pub sequence: u64,
}

/// Разбирает ровно три сегмента с каноническими десятичными `u64`.
///
/// Ноль допустим в грамматике. Ожидаемое направление, generation и допустимость
/// sequence zero проверяет вызывающая сторона по фазе протокола.
pub fn parse_wire_id(raw: &str) -> Result<WireId> {
    let mut parts = raw.split(':');
    let direction = match parts.next() {
        Some("h") => WireDirection::Host,
        Some("m") => WireDirection::Module,
        _ => bail!("wire id {raw:?} has an unknown direction"),
    };
    let generation_raw = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("wire id {raw:?} is missing generation"))?;
    let sequence_raw = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("wire id {raw:?} is missing sequence"))?;
    if parts.next().is_some() {
        bail!("wire id {raw:?} has extra segments");
    }
    let generation = parse_canonical_number(generation_raw, raw, "generation")?;
    let sequence = parse_canonical_number(sequence_raw, raw, "sequence")?;
    Ok(WireId {
        direction,
        generation,
        sequence,
    })
}

fn parse_canonical_number(value: &str, id: &str, label: &str) -> Result<u64> {
    let parsed = value
        .parse::<u64>()
        .with_context(|| format!("wire id {id:?} has invalid {label}"))?;
    if parsed.to_string() != value {
        bail!("wire id {id:?} has non-canonical {label}");
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directional_ids_are_exact_and_canonical() {
        for (raw, expected) in [
            (
                "h:7:42",
                WireId {
                    direction: WireDirection::Host,
                    generation: 7,
                    sequence: 42,
                },
            ),
            (
                "m:7:9",
                WireId {
                    direction: WireDirection::Module,
                    generation: 7,
                    sequence: 9,
                },
            ),
            (
                "h:0:0",
                WireId {
                    direction: WireDirection::Host,
                    generation: 0,
                    sequence: 0,
                },
            ),
            (
                "m:18446744073709551615:18446744073709551615",
                WireId {
                    direction: WireDirection::Module,
                    generation: u64::MAX,
                    sequence: u64::MAX,
                },
            ),
        ] {
            assert_eq!(parse_wire_id(raw).expect("valid wire id"), expected);
        }
        for invalid in [
            "",
            "7:1",
            "x:7:1",
            "H:7:1",
            "h:07:1",
            "m:7:01",
            "h:+7:1",
            "m:7:-1",
            "h: 7:1",
            "m:7:1 ",
            "h::1",
            "m:7:",
            "h:7",
            "m:7:1:extra",
            "h:18446744073709551616:1",
            "m:7:18446744073709551616",
        ] {
            parse_wire_id(invalid).expect_err("invalid wire id must fail");
        }
        assert!(
            parse_wire_id("h:invalid:1:extra")
                .expect_err("invalid structure must fail before number parsing")
                .to_string()
                .contains("extra segments")
        );
    }
}
