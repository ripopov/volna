//! Minimal waveform round trip.
//!
//!     cargo run -p vtr --example waveform -- /tmp/waveform.vtr

use vtr::{Direction, Reader, ScopeType, SignalKind, VarType, Writer};

fn main() -> vtr::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "waveform.vtr".into());
    let mut writer = Writer::create(&path)?;
    writer.set_timescale(-9)?;
    let top = writer.add_scope(None, "top", ScopeType::Module, "counter")?;
    let (_, count) = writer.add_var(
        Some(top),
        "count",
        VarType::Reg,
        Direction::Output,
        SignalKind::Bits {
            width: 8,
            states: 4,
        },
    )?;
    writer.set_time(0)?;
    writer.emit_u64(count, 0)?;
    writer.set_time(10)?;
    writer.emit_u64(count, 42)?;
    writer.close()?;

    let reader = Reader::open(&path)?;
    let count = reader
        .find_signal("top.count", '.')
        .expect("declared signal");
    assert_eq!(reader.value_at(count, 10)?.borrow().as_u64(), Some(42));
    assert_eq!(reader.changes(count, 0, 10)?.len(), 2);
    println!(
        "top.count at 10 ns: {}",
        reader.value_at(count, 10)?.to_ascii()
    );
    Ok(())
}
