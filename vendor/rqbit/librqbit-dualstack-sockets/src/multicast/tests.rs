#[test]
fn multicast_scope_classification() {
    use crate::addr::Ipv6AddrExt;
    assert!(
        "ff02::1"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()
            .is_link_local_mcast()
    );
    assert!(
        "ff05::1"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()
            .is_site_local_mcast()
    );
}
