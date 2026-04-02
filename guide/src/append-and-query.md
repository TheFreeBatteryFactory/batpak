# Append and query

Use `Store::append`, `Store::get`, `Store::query`, `Store::stream`, `Store::by_scope`, and `Store::by_fact` for the basic event-log workflow. The storage boundary returns `StoredEvent<serde_json::Value>`, which carries both the `Coordinate` and the decoded event payload/header.
