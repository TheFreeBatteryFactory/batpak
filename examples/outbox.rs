use batpak::prelude::*;

#[allow(clippy::print_stdout)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(StoreConfig::new(dir.path()))?;
    let kind = EventKind::custom(0xF, 3);

    let mut outbox = store.outbox();
    outbox.stage(
        Coordinate::new("player:outbox", "room:batch")?,
        kind,
        &serde_json::json!({"n": 1}),
    )?;
    outbox.stage(
        Coordinate::new("player:outbox", "room:batch")?,
        kind,
        &serde_json::json!({"n": 2}),
    )?;
    outbox.stage(
        Coordinate::new("player:outbox", "room:batch")?,
        kind,
        &serde_json::json!({"n": 3}),
    )?;

    let receipts = outbox.flush()?;
    println!(
        "flushed {} staged events through one batch path",
        receipts.len()
    );

    store.close()?;
    Ok(())
}
