use crate::caps::Capability;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapClass {
    FsRead,
    FsWrite,
    NetClient,
    NetServer,
    ProcessSpawn,
    EnvRead,
    EnvState,
    Clock,
    Time,
    Random,
    FfiUnsafe,
}

pub fn capability_of(class: CapClass, path: Option<&str>) -> Capability {
    let path = path.map(|p| p.to_string());
    match class {
        CapClass::FsRead => Capability::FsRead(path),
        CapClass::FsWrite => Capability::FsWrite(path),
        CapClass::NetClient => Capability::NetClient,
        CapClass::NetServer => Capability::NetServer,
        CapClass::ProcessSpawn => Capability::ProcessSpawn,
        CapClass::EnvRead => Capability::EnvRead,
        CapClass::EnvState => Capability::State(Some("env".to_string())),
        CapClass::Clock => Capability::Clock,
        CapClass::Time => Capability::Time,
        CapClass::Random => Capability::Random,
        CapClass::FfiUnsafe => Capability::FfiUnsafe,
    }
}

pub type ClassedArg = (CapClass, Option<usize>);

pub enum ExternCaps {
    Effect(&'static [ClassedArg]),
    OpenMode { path_arg: usize, mode_arg: usize },
    Benign,
    Unknown,
}

const FS_READ_0: &[ClassedArg] = &[(CapClass::FsRead, Some(0))];
const FS_WRITE_0: &[ClassedArg] = &[(CapClass::FsWrite, Some(0))];
const FS_WRITE_1: &[ClassedArg] = &[(CapClass::FsWrite, Some(1))];
const FS_WRITE_0_1: &[ClassedArg] = &[(CapClass::FsWrite, Some(0)), (CapClass::FsWrite, Some(1))];
const FS_RW_0: &[ClassedArg] = &[(CapClass::FsRead, Some(0)), (CapClass::FsWrite, Some(0))];
const NET_CLIENT: &[ClassedArg] = &[(CapClass::NetClient, None)];
const NET_SERVER: &[ClassedArg] = &[(CapClass::NetServer, None)];
const PROC_SPAWN: &[ClassedArg] = &[(CapClass::ProcessSpawn, None)];
const ENV_READ: &[ClassedArg] = &[(CapClass::EnvRead, None)];
const ENV_STATE: &[ClassedArg] = &[(CapClass::EnvState, None)];
const CLOCK: &[ClassedArg] = &[(CapClass::Clock, None)];
const TIME: &[ClassedArg] = &[(CapClass::Time, None)];
const RANDOM: &[ClassedArg] = &[(CapClass::Random, None)];
const FFI_UNSAFE: &[ClassedArg] = &[(CapClass::FfiUnsafe, None)];

const EFFECT_EXTERNS: &[(&str, &[ClassedArg])] = &[
    ("access", FS_READ_0),
    ("stat", FS_READ_0),
    ("opendir", FS_READ_0),
    ("c_opendir", FS_READ_0),
    ("realpath", FS_READ_0),
    ("c_chdir", FS_READ_0),
    ("jinn_is_dir", FS_READ_0),
    ("jinn_is_file", FS_READ_0),
    ("jinn_is_symlink", FS_READ_0),
    ("jinn_file_mtime", FS_READ_0),
    ("jinn_file_size", FS_READ_0),
    ("open", FS_RW_0),
    ("remove", FS_WRITE_0),
    ("c_remove", FS_WRITE_0),
    ("unlink", FS_WRITE_0),
    ("rename", FS_WRITE_0_1),
    ("c_rename", FS_WRITE_0_1),
    ("c_mkdir", FS_WRITE_0),
    ("c_rmdir", FS_WRITE_0),
    ("c_symlink", FS_WRITE_1),
    ("jinn_chmod", FS_WRITE_0),
    ("mmap", FFI_UNSAFE),
    ("munmap", FFI_UNSAFE),
    ("mprotect", FFI_UNSAFE),
    ("msync", FFI_UNSAFE),
    ("connect", NET_CLIENT),
    ("jinn_dns_resolve", NET_CLIENT),
    ("jinn_dns_resolve_all", NET_CLIENT),
    ("jinn_tls_connect", NET_CLIENT),
    ("jinn_sendto", NET_CLIENT),
    ("bind", NET_SERVER),
    ("listen_sock", NET_SERVER),
    ("accept", NET_SERVER),
    ("jinn_tls_listen", NET_SERVER),
    ("jinn_tls_accept", NET_SERVER),
    ("jinn_spawn_exec", PROC_SPAWN),
    ("jinn_spawn_capture", PROC_SPAWN),
    ("jinn_exec_capture", PROC_SPAWN),
    ("jinn_system", PROC_SPAWN),
    ("jinn_popen_read", PROC_SPAWN),
    ("getenv", ENV_READ),
    ("jinn_getenv_or_empty", ENV_READ),
    ("sysconf", ENV_READ),
    ("getpid", ENV_READ),
    ("jinn_hostname", ENV_READ),
    ("getcwd", ENV_READ),
    ("jinn_cwd", ENV_READ),
    ("setenv", ENV_STATE),
    ("unsetenv", ENV_STATE),
    ("time", CLOCK),
    ("__time_monotonic", TIME),
    ("__random_u64", RANDOM),
    ("jinn_random_bytes", RANDOM),
];

const BENIGN_EXTERNS: &[&str] = &[
    "acos",
    "asin",
    "atan",
    "atan2",
    "atof",
    "cbrt",
    "ceil",
    "closedir",
    "c_closedir",
    "c_readdir_name",
    "cos",
    "__cos",
    "cosh",
    "exit",
    "exp",
    "fabs",
    "fclose",
    "feof",
    "fflush",
    "fgets",
    "floor",
    "fmax",
    "fmin",
    "fmod",
    "fread",
    "free",
    "fseek",
    "fstat_size",
    "ftell",
    "fwrite",
    "htons",
    "hypot",
    "inet_pton",
    "isatty",
    "jinn_aes_cbc_decrypt",
    "jinn_aes_cbc_encrypt",
    "jinn_aes_gcm_decrypt",
    "jinn_aes_gcm_encrypt",
    "jinn_argon2id",
    "jinn_bits_to_f64",
    "jinn_bytes_to_hex",
    "jinn_chacha20_poly1305_decrypt",
    "jinn_chacha20_poly1305_encrypt",
    "jinn_close",
    "jinn_dirent_name",
    "jinn_event_loop_add_read",
    "jinn_event_loop_add_write",
    "jinn_event_loop_create",
    "jinn_event_loop_destroy",
    "jinn_event_loop_poll",
    "jinn_event_loop_rearm_read",
    "jinn_event_loop_rearm_write",
    "jinn_event_loop_remove",
    "jinn_event_wait_readable",
    "jinn_event_wait_writable",
    "jinn_evp_digest",
    "jinn_evp_hmac",
    "jinn_f64_to_bits",
    "jinn_fd_close",
    "jinn_fd_set_nonblock",
    "jinn_hex_to_bytes",
    "jinn_hmac_sha256",
    "jinn_io_waiter_create",
    "jinn_io_waiter_destroy",
    "jinn_ovector_get",
    "jinn_pbkdf2",
    "jinn_recv",
    "jinn_recvfrom",
    "jinn_scrypt",
    "jinn_send",
    "jinn_socket",
    "jinn_sort_f64",
    "jinn_terminal_disable_raw",
    "jinn_terminal_enable_raw",
    "jinn_terminal_size",
    "jinn_tls_close",
    "jinn_tls_last_error",
    "jinn_tls_listener_close",
    "jinn_tls_peer_cert_subject",
    "jinn_tls_protocol_version",
    "jinn_tls_recv",
    "jinn_tls_send",
    "__ln",
    "log",
    "log2",
    "malloc",
    "memcmp",
    "memcpy",
    "memset",
    "pcre2_code_free_8",
    "pcre2_compile_8",
    "pcre2_get_ovector_pointer_8",
    "pcre2_match_8",
    "pcre2_match_data_create_from_pattern_8",
    "pcre2_match_data_free_8",
    "pcre2_substring_number_from_name_8",
    "pow",
    "readdir",
    "round",
    "setsockopt",
    "shutdown",
    "sin",
    "sinh",
    "sqrt",
    "__sqrt",
    "strlen",
    "tan",
    "tanh",
    "trunc",
];

pub fn classify_extern(name: &str) -> ExternCaps {
    if name == "fopen" {
        return ExternCaps::OpenMode {
            path_arg: 0,
            mode_arg: 1,
        };
    }
    if let Some((_, caps)) = EFFECT_EXTERNS.iter().find(|(n, _)| *n == name) {
        return ExternCaps::Effect(caps);
    }
    if BENIGN_EXTERNS.contains(&name) {
        return ExternCaps::Benign;
    }
    ExternCaps::Unknown
}

pub struct ApertureSite {
    pub symbol: &'static str,
    pub caps: &'static [ClassedArg],
    pub open_mode: Option<(usize, usize)>,
}

pub const APERTURE_SITES: &[ApertureSite] = &[
    ApertureSite {
        symbol: "io.open",
        caps: &[],
        open_mode: Some((0, 1)),
    },
    ApertureSite {
        symbol: "io.read_file",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.read_bytes",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.read_lines",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.write_file",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.write_bytes",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.write_lines",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.append_file",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.copy_file",
        caps: &[(CapClass::FsRead, Some(0)), (CapClass::FsWrite, Some(1))],
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.move_file",
        caps: FS_WRITE_0_1,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.remove_file",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "io.file_exists",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.mkdir",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.mkdir_p",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.rmdir",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.list_dir",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.exists",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.is_readable",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.is_writable",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.remove",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.rename",
        caps: FS_WRITE_0_1,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.copy",
        caps: &[(CapClass::FsRead, Some(0)), (CapClass::FsWrite, Some(1))],
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.move",
        caps: FS_WRITE_0_1,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.cwd",
        caps: ENV_READ,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.chdir",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.absolute",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.symlink",
        caps: FS_WRITE_1,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.file_size",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.is_dir",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.is_file",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.is_symlink",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.modified",
        caps: FS_READ_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.chmod",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.touch",
        caps: FS_WRITE_0,
        open_mode: None,
    },
    ApertureSite {
        symbol: "fs.walk",
        caps: FS_READ_0,
        open_mode: None,
    },
];

pub fn aperture(symbol: &str) -> Option<&'static ApertureSite> {
    APERTURE_SITES.iter().find(|s| s.symbol == symbol)
}

pub fn open_mode_caps(path: Option<&str>, mode: Option<&str>, out: &mut impl FnMut(Capability)) {
    match mode {
        Some(m) if m.starts_with('r') && !m.contains('+') => {
            out(capability_of(CapClass::FsRead, path));
        }
        Some(_) => {
            out(capability_of(CapClass::FsWrite, path));
        }
        None => {
            out(capability_of(CapClass::FsRead, path));
            out(capability_of(CapClass::FsWrite, path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_have_no_duplicate_symbols() {
        let mut seen = std::collections::HashSet::new();
        for (n, _) in EFFECT_EXTERNS {
            assert!(seen.insert(*n), "duplicate effect extern {n}");
        }
        for n in BENIGN_EXTERNS {
            assert!(seen.insert(*n), "extern {n} classified twice");
        }
        let mut ap = std::collections::HashSet::new();
        for s in APERTURE_SITES {
            assert!(ap.insert(s.symbol), "duplicate aperture {}", s.symbol);
        }
    }

    #[test]
    fn effect_extern_classifies() {
        match classify_extern("connect") {
            ExternCaps::Effect(caps) => {
                assert_eq!(caps, NET_CLIENT);
            }
            _ => panic!("connect must be an effect extern"),
        }
    }

    #[test]
    fn unknown_extern_is_unknown() {
        assert!(matches!(
            classify_extern("my_ffi_thing"),
            ExternCaps::Unknown
        ));
    }

    #[test]
    fn benign_extern_is_benign() {
        assert!(matches!(classify_extern("memcpy"), ExternCaps::Benign));
    }

    #[test]
    fn fopen_is_mode_dependent() {
        let mut caps = Vec::new();
        open_mode_caps(Some("./f"), Some("r"), &mut |c| caps.push(c));
        assert_eq!(caps, vec![Capability::FsRead(Some("./f".into()))]);
        caps.clear();
        open_mode_caps(Some("./f"), Some("w"), &mut |c| caps.push(c));
        assert_eq!(caps, vec![Capability::FsWrite(Some("./f".into()))]);
        caps.clear();
        open_mode_caps(Some("./f"), None, &mut |c| caps.push(c));
        assert_eq!(caps.len(), 2);
    }

    #[test]
    fn aperture_lookup() {
        let s = aperture("io.write_file").unwrap();
        assert_eq!(s.caps, FS_WRITE_0);
        assert!(aperture("io.nope").is_none());
    }

    #[test]
    fn std_effect_externs_are_all_classified() {
        for name in [
            "fopen",
            "fwrite",
            "getenv",
            "jinn_random_bytes",
            "jinn_spawn_exec",
            "jinn_tls_connect",
            "time",
            "__time_monotonic",
            "setenv",
        ] {
            assert!(
                !matches!(classify_extern(name), ExternCaps::Unknown),
                "{name} must be classified"
            );
        }
    }
}
