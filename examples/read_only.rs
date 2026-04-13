use batpak::prelude::*;

#[allow(clippy::print_stdout)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let config = StoreConfig::new(dir.path())
        .with_enable_checkpoint(true)
        .with_enable_mmap_index(true);

    let store = Store::open(config.clone())?;
    let coord = Coordinate::new("player:readonly", "room:archive")?;
    let kind = EventKind::custom(0xF, 5);
    store.append(&coord, kind, &serde_json::json!({"n": 1}))?;
    store.close()?;

    let read_only = Store::<batpak::store::ReadOnly>::open_read_only(config)?;
    let stream = read_only.stream("player:readonly");
    println!("read-only reopen recovered {} event(s)", stream.len());

    Ok(())
}
