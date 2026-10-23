//! Provision a deterministic multi-page recording for headless hierarchy transfer checks.

use vtr::{Direction, ScopeType, SignalKind, SourceOptions, VarType, Writer};

fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .expect("hierarchy_fixture OUTPUT.vtr");
    let mut writer = Writer::create(path)?;
    let root = writer.scope_with(SourceOptions::new("顶层.λ", None), ScopeType::Module, "TOP")?;
    let signal = writer.signal(
        SourceOptions::new("net", Some(root)),
        SignalKind::Bits {
            width: 1,
            states: 2,
        },
        VarType::Wire,
        Direction::Input,
    )?;
    for i in 0..65_537 {
        let scope = writer.scope_with(
            SourceOptions::new(format!("cell{i}"), Some(root)),
            ScopeType::Module,
            "NAND2_X1",
        )?;
        writer.alias(
            signal,
            SourceOptions::new("\\pin.λ", Some(scope)),
            VarType::Wire,
            Direction::Output,
        )?;
    }
    writer.close()?;
    Ok(())
}
