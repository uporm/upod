#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemOS {
    Linux,
    MacOS,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemArch {
    X86_64,
    Aarch64,
    Unknown,
}

impl SystemOS {
    pub fn current() -> Self {
        Self::from_os(std::env::consts::OS)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SystemOS::Linux => "linux",
            SystemOS::MacOS => "macos",
            SystemOS::Unknown => "unknown",
        }
    }

    fn from_os(os: &str) -> Self {
        match os {
            "linux" => SystemOS::Linux,
            "macos" => SystemOS::MacOS,
            _ => SystemOS::Unknown,
        }
    }
    pub fn is_linux() -> bool {
        Self::current() == SystemOS::Linux
    }

    pub fn is_macos() -> bool {
        Self::current() == SystemOS::MacOS
    }
}

impl SystemArch {
    pub fn current() -> Self {
        Self::from_arch(std::env::consts::ARCH)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SystemArch::X86_64 => "x86_64",
            SystemArch::Aarch64 => "aarch64",
            SystemArch::Unknown => "unknown",
        }
    }

    fn from_arch(arch: &str) -> Self {
        match arch {
            "x86_64" => SystemArch::X86_64,
            "aarch64" => SystemArch::Aarch64,
            _ => SystemArch::Unknown,
        }
    }

    pub fn is_x86_64() -> bool {
        Self::current() == SystemArch::X86_64
    }

    pub fn is_aarch64() -> bool {
        Self::current() == SystemArch::Aarch64
    }
}

