//! FST waveforms converted through the shared trace session API.
use anyhow::{bail, ensure, Result};
use std::collections::{BTreeMap, BinaryHeap};
use std::path::Path;
use volna_trace::data::{Direction, SignalShape, WaveValue};
use vtr::{LogicStates, ScopeType, SignalKind, VarType, Writer};

/// Convert FST hierarchy, aliases, and recorded values into a fresh writer.
///
/// All distinct signal histories must fit in memory. Scope/variable types,
/// input/output/inout directions, components, and the time base are preserved.
/// Other directions become implicit; root variables use the session's `(top)`
/// scope. FST-specific attributes and enum tables are not exported by the session
/// API. Nonzero time offsets and dump-off regions are rejected by that API.
/// Nine-state declarations preserve the full FST logic alphabet. Use a writer
/// with `dedup: false` to retain repeated non-event samples.
pub fn convert_fst(input: impl AsRef<Path>, w: &mut Writer) -> Result<()> {
    let session = volna_trace::OpenSpec::Path(input.as_ref().to_owned()).open()?;
    ensure!(session.format() == Some("FST"), "expected an FST recording");
    w.set_timescale(vtr::Timescale::Exponent(session.info().timescale))?;
    let h = session.hierarchy();
    let mut scopes = Vec::new();
    for scope in h.scopes() {
        let kind = (0..=255)
            .map(ScopeType::from_code)
            .find(|k| k.name() == scope.kind)
            .unwrap_or(ScopeType::Generic);
        let parent = scope.parent.map(|p| scopes[p]);
        scopes.push(w.add_scope(parent, scope.name, kind, scope.component)?);
    }
    let mut signals = BTreeMap::new();
    for var in h.vars() {
        let kind = match var.shape {
            SignalShape::Bit | SignalShape::Event => SignalKind::bits(1, LogicStates::Nine),
            SignalShape::Vector { width } => SignalKind::bits(width, LogicStates::Nine),
            SignalShape::Real => SignalKind::Real,
            SignalShape::Text => SignalKind::VarLen,
        };
        let var_type = (0..=255)
            .map(VarType::from_code)
            .find(|k| k.name() == var.var_type)
            .unwrap_or(VarType::Wire);
        let direction = match var.direction {
            Direction::Input => vtr::Direction::Input,
            Direction::Output => vtr::Direction::Output,
            Direction::InOut => vtr::Direction::InOut,
            Direction::None => vtr::Direction::Implicit,
        };
        let parent = Some(scopes[var.scope]);
        match signals.entry(var.signal) {
            std::collections::btree_map::Entry::Occupied(e) => {
                w.add_alias(parent, var.name, var_type, direction, *e.get())?;
            }
            std::collections::btree_map::Entry::Vacant(e) => {
                let (_, signal) = w.add_var(parent, var.name, var_type, direction, kind)?;
                e.insert(signal);
            }
        }
    }
    let refs: Vec<_> = signals.keys().copied().collect();
    let histories: Vec<_> = session
        .load_signals(&refs)
        .into_iter()
        .map(|(s, h)| h.map(|h| (signals[&s], h)))
        .collect::<Result<_>>()?;
    let mut heap = BinaryHeap::new();
    for (i, (_, h)) in histories.iter().enumerate() {
        if !h.is_empty() {
            heap.push(std::cmp::Reverse((h.time(0), i, 0)));
        }
    }
    // Merge complete histories so the writer sees globally nondecreasing times.
    while let Some(std::cmp::Reverse((time, i, offset))) = heap.pop() {
        let (signal, history) = &histories[i];
        w.set_time(time)?;
        match history.value(Some(offset)) {
            WaveValue::Bits(bits) => w.emit_logic_str(*signal, bits.as_bytes())?,
            WaveValue::Real(value) => w.emit_real(*signal, value)?,
            WaveValue::Bytes(bytes) => w.emit_varlen(*signal, &bytes)?,
            WaveValue::Text(text) => w.emit_varlen(*signal, text.as_bytes())?,
            WaveValue::Unavailable => bail!("FST returned an unavailable recorded sample"),
        }
        if offset + 1 < history.len() {
            heap.push(std::cmp::Reverse((history.time(offset + 1), i, offset + 1)));
        }
    }
    w.set_time(session.info().time_range.1)?;
    Ok(())
}
