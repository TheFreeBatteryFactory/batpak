# free-batteries

Event-sourced state machines over coordinate spaces.

```rust
use free_batteries::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::open_default()?;
    let coord = Coordinate::new("player:alice", "room:dungeon")?;
    let kind = EventKind::custom(0xF, 1);

    let receipt = store.append(&coord, kind, &serde_json::json!({"x": 10, "y": 20}))?;
    println!("Stored event {} at seq {}", receipt.event_id, receipt.sequence);

    for entry in store.stream("player:alice") {
        let stored = store.get(entry.event_id)?;
        println!("{}: {:?}", stored.event.event_kind(), stored.event.payload);
    }
    Ok(())
}
```

## License

MIT OR Apache-2.0
