# Projections

Implement `EventSourced`, then call `Store::project`. Use `Freshness::Consistent` when you need exact replay and a cache backend when replay cost matters.
