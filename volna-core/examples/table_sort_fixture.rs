//! Reproducible browser sorting fixture with unique labels in reverse source order.
use vtr::{Recorder, SourceOptions, Status, Value, Writer};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("output .vtr path");
    let count: u64 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "1000000".into())
        .parse()?;
    let mut writer = Writer::create(path)?;
    let stream = writer.scope("sort", None)?;
    let generator =
        writer.transactions::<()>(SourceOptions::new("rows", Some(stream)), "TRANSACTION")?;
    let label = writer.intern("vtr.label")?;
    for i in 0..count {
        let tx = writer.begin_at(generator, i, &(), None)?;
        writer.attribute(tx.owner(), label, Value::Text(format!("{:020}", count - i)))?;
        writer.end_at(tx, i + 1, Status::Ok)?;
    }
    writer.close()?;
    Ok(())
}
