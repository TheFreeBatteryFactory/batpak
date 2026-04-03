//! Compile-time assertions that key public types implement Send + Sync.
//! [SPEC:tests/type_assertions.rs]

#[test]
fn store_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<batpak::store::Store>();
}

#[test]
fn append_receipt_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<batpak::store::AppendReceipt>();
}

#[test]
fn store_config_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<batpak::store::StoreConfig>();
}

#[test]
fn notification_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<batpak::store::Notification>();
}

#[test]
fn coordinate_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<batpak::coordinate::Coordinate>();
}

#[test]
fn event_kind_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<batpak::event::EventKind>();
}

#[test]
fn receipt_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<batpak::guard::Receipt<()>>();
}
