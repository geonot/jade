use crate::caps::Capability;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapClass {
    FsRead,
    FsWrite,
    NetClient,
    NetServer,
    ProcessSpawn,
    EnvRead,
    Clock,
    Time,
    Random,
    FfiUnsafe,
}

#[derive(Debug, Clone, Copy)]
pub struct CapSite {
    pub symbol: &'static str,
    pub class: CapClass,
    pub path_arg: Option<usize>,
}

pub const CAP_SITES: &[CapSite] = &[
    CapSite {
        symbol: "std.net.connect",
        class: CapClass::NetClient,
        path_arg: None,
    },
    CapSite {
        symbol: "std.net.get",
        class: CapClass::NetClient,
        path_arg: None,
    },
    CapSite {
        symbol: "std.net.listen",
        class: CapClass::NetServer,
        path_arg: None,
    },
    CapSite {
        symbol: "std.fs.read_file",
        class: CapClass::FsRead,
        path_arg: Some(0),
    },
    CapSite {
        symbol: "std.fs.read_dir",
        class: CapClass::FsRead,
        path_arg: Some(0),
    },
    CapSite {
        symbol: "std.fs.write_file",
        class: CapClass::FsWrite,
        path_arg: Some(0),
    },
    CapSite {
        symbol: "std.fs.remove",
        class: CapClass::FsWrite,
        path_arg: Some(0),
    },
    CapSite {
        symbol: "std.process.spawn",
        class: CapClass::ProcessSpawn,
        path_arg: None,
    },
    CapSite {
        symbol: "std.env.get",
        class: CapClass::EnvRead,
        path_arg: None,
    },
    CapSite {
        symbol: "std.time.now",
        class: CapClass::Clock,
        path_arg: None,
    },
    CapSite {
        symbol: "std.time.monotonic",
        class: CapClass::Time,
        path_arg: None,
    },
    CapSite {
        symbol: "std.random.next",
        class: CapClass::Random,
        path_arg: None,
    },
];

pub fn lookup(symbol: &str) -> Option<&'static CapSite> {
    CAP_SITES.iter().find(|s| s.symbol == symbol)
}

pub fn capability_of(site: &CapSite, path_literal: Option<&str>) -> Capability {
    let path = path_literal.map(|p| p.to_string());
    match site.class {
        CapClass::FsRead => Capability::FsRead(path),
        CapClass::FsWrite => Capability::FsWrite(path),
        CapClass::NetClient => Capability::NetClient,
        CapClass::NetServer => Capability::NetServer,
        CapClass::ProcessSpawn => Capability::ProcessSpawn,
        CapClass::EnvRead => Capability::EnvRead,
        CapClass::Clock => Capability::Clock,
        CapClass::Time => Capability::Time,
        CapClass::Random => Capability::Random,
        CapClass::FfiUnsafe => Capability::FfiUnsafe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_no_duplicate_symbols() {
        let mut seen = std::collections::HashSet::new();
        for s in CAP_SITES {
            assert!(seen.insert(s.symbol), "duplicate cap site {}", s.symbol);
        }
    }

    #[test]
    fn path_sites_carry_path() {
        let s = lookup("std.fs.write_file").unwrap();
        assert_eq!(s.path_arg, Some(0));
        let cap = capability_of(s, Some("./out"));
        assert_eq!(cap, Capability::FsWrite(Some("./out".into())));
        let unscoped = capability_of(s, None);
        assert_eq!(unscoped, Capability::FsWrite(None));
    }

    #[test]
    fn net_site_is_unscoped() {
        let s = lookup("std.net.connect").unwrap();
        assert_eq!(s.path_arg, None);
        assert_eq!(capability_of(s, None), Capability::NetClient);
    }

    #[test]
    fn unknown_symbol_is_none() {
        assert!(lookup("std.nope.nope").is_none());
    }
}
