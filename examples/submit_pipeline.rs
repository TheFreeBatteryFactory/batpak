use batpak::prelude::*;

#[allow(clippy::print_stdout)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(StoreConfig::new(dir.path()))?;

    let coord = Coordinate::new("player:submit", "room:pipeline")?;
    let kind = EventKind::custom(0xF, 2);

    let first = store.submit(&coord, kind, &serde_json::json!({"n": 1}))?;
    let second = store.submit(&coord, kind, &serde_json::json!({"n": 2}))?;
    let third = store.submit(&coord, kind, &serde_json::json!({"n": 3}))?;

    let receipts = [first.wait()?, second.wait()?, third.wait()?];
    println!(
        "queued {} appends and committed through the blocking wait path",
        receipts.len()
    );

    store.close()?;
    Ok(())
}
