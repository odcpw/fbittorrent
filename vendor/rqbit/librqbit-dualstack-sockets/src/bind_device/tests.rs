#[test]
fn bind_device_rejects_unknown_interfaces() {
    assert!(super::BindDevice::new_from_name("rqbit-missing-interface").is_err());
}
