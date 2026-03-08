#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEnv {
    Linux,
    MacOS,
    Windows,
    Unknown,
}

impl SystemEnv {
    pub fn current() -> Self {
        Self::from_os(std::env::consts::OS)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SystemEnv::Linux => "linux",
            SystemEnv::MacOS => "macos",
            SystemEnv::Windows => "windows",
            SystemEnv::Unknown => "unknown",
        }
    }

    fn from_os(os: &str) -> Self {
        match os {
            "linux" => SystemEnv::Linux,
            "macos" => SystemEnv::MacOS,
            "windows" => SystemEnv::Windows,
            _ => SystemEnv::Unknown,
        }
    }
}

pub struct SystemEnvUtils;

impl SystemEnvUtils {
    pub fn current() -> SystemEnv {
        SystemEnv::current()
    }

    pub fn is_linux() -> bool {
        Self::current() == SystemEnv::Linux
    }

    pub fn is_macos() -> bool {
        Self::current() == SystemEnv::MacOS
    }

    pub fn is_windows() -> bool {
        Self::current() == SystemEnv::Windows
    }
}

#[cfg(test)]
mod tests {
    use super::SystemEnv;

    #[test]
    fn current_matches_compile_target_os() {
        assert_eq!(SystemEnv::current(), SystemEnv::from_os(std::env::consts::OS));
    }

    #[test]
    fn parse_known_os_names() {
        assert_eq!(SystemEnv::from_os("linux"), SystemEnv::Linux);
        assert_eq!(SystemEnv::from_os("macos"), SystemEnv::MacOS);
        assert_eq!(SystemEnv::from_os("windows"), SystemEnv::Windows);
    }

    #[test]
    fn parse_unknown_os_name() {
        assert_eq!(SystemEnv::from_os("freebsd"), SystemEnv::Unknown);
    }
}
