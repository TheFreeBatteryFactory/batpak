use batpak::prelude::*;

#[allow(clippy::print_stdout)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(StoreConfig::new(dir.path()))?;

    let coord = Coordinate::new("player:fence", "room:hidden")?;
    let kind = EventKind::custom(0xF, 4);

    let fence = store.begin_visibility_fence()?;
    let ticket = fence.submit(&coord, kind, &serde_json::json!({"hidden": true}))?;
    let receipt = ticket.wait()?;

    println!(
        "durable before commit: visible_count={}",
        store.by_fact(kind).len()
    );
    assert_eq!(store.by_fact(kind).len(), 0);

    fence.commit()?;

    println!(
        "after commit event {} is visible and query count is {}",
        receipt.event_id,
        store.by_fact(kind).len()
    );

    store.close()?;
    Ok(())
}
