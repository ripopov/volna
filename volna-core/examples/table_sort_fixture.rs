//! Reproducible browser sorting fixture with unique labels in reverse source order.
use vtr::{TxStatus, Value, Writer};

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("output .vtr path");
    let count: u64 = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "1000000".into())
        .parse()?;
    let mut writer = Writer::create(path)?;
    let stream = writer.add_scope(None, "sort", vtr::ScopeType::Generic, "")?;
    let generator = writer.add_generator(Some(stream), "rows", "TRANSACTION")?;
    let label = writer.intern("vtr.label");
    for i in 0..count {
        let tx = writer.begin_tx(generator, i)?;
        writer.tx_attr(tx, label, &Value::Text(format!("{:020}", count - i)))?;
        writer.end_tx(tx, i + 1, TxStatus::Ok)?;
    }
    writer.close()?;
    Ok(())
}
