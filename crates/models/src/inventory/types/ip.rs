use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

// TODO: Rewrite this to not be generic. It's not necessary and `String` implements all of these
// traits, they're are definitely strings that are invalid IP addresses.
#[derive(Serialize, Deserialize, Debug, Clone, Hash, Eq, PartialEq)]
pub struct IPInfo<IP: Serialize + std::fmt::Debug + Clone> {
    pub subnet: IP,
    pub netmask: u8,
    pub gateway: Option<IP>,
    pub provides_dhcp: bool,
}

// TODO: Refactor this to be a 3 member enum for v4, v6, or both. Currently { v4: None, v6: None }
// is (incorrectly) valid and this is not a well written type.
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct IPNetwork {
    pub v4: Option<IPInfo<Ipv4Addr>>,
    pub v6: Option<IPInfo<Ipv6Addr>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    impl Arbitrary for IPInfo<Ipv4Addr> {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<Ipv4Addr>(),         // subnet
                (0..=32u8),                // netmask (valid for IPv4)
                any::<Option<Ipv4Addr>>(), // gateway
                any::<bool>(),             // provides_dhcp
            )
                .prop_map(|(subnet, netmask, gateway, provides_dhcp)| IPInfo {
                    subnet,
                    netmask,
                    gateway,
                    provides_dhcp,
                })
                .boxed()
        }
    }

    impl Arbitrary for IPInfo<Ipv6Addr> {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<Ipv6Addr>(),         // subnet
                (0..=128u8),               // netmask (valid for IPv6)
                any::<Option<Ipv6Addr>>(), // gateway
                any::<bool>(),             // provides_dhcp
            )
                .prop_map(|(subnet, netmask, gateway, provides_dhcp)| IPInfo {
                    subnet,
                    netmask,
                    gateway,
                    provides_dhcp,
                })
                .boxed()
        }
    }

    impl Arbitrary for IPNetwork {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            (
                any::<Option<IPInfo<Ipv4Addr>>>(),
                any::<Option<IPInfo<Ipv6Addr>>>(),
            )
                .prop_map(|(v4, v6)| IPNetwork { v4, v6 })
                .boxed()
        }
    }

    proptest! {
        #[test]
        fn test_ipinfo_ipv4_arbitrary(ip_info in any::<IPInfo<Ipv4Addr>>()) {
            // make sure the netmask is valid for IPv4
            assert!(ip_info.netmask <= 32);
        }

        #[test]
        fn test_ipinfo_ipv6_arbitrary(ip_info in any::<IPInfo<Ipv6Addr>>()) {
            // make sure the netmask is valid for IPv6
            assert!(ip_info.netmask <= 128);
        }

        // #[test]
        // fn test_ipnetwork_arbitrary(network in any::<IPNetwork>()) {
            // make sure at least one of v4 or v6 is present
            // assert!(network.v4.is_some() || network.v6.is_some());
        // }
    }
}
