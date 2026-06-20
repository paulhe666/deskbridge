#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Platform {
    Unknown = 0,
    Windows = 1,
    MacOS = 2,
}

impl Platform {
    pub fn local() -> Self {
        #[cfg(windows)]
        {
            Self::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOS
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            Self::Unknown
        }
    }

    pub fn from_byte(value: u8) -> Self {
        match value {
            1 => Self::Windows,
            2 => Self::MacOS,
            _ => Self::Unknown,
        }
    }

    pub fn as_byte(self) -> u8 {
        self as u8
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOS => "macos",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionProfile {
    WindowsToWindows,
    WindowsToMacOS,
    MacOSToWindows,
    MacOSToMacOS,
    Unknown,
}

impl ConnectionProfile {
    pub fn new(server: Platform, client: Platform) -> Self {
        match (server, client) {
            (Platform::Windows, Platform::Windows) => Self::WindowsToWindows,
            (Platform::Windows, Platform::MacOS) => Self::WindowsToMacOS,
            (Platform::MacOS, Platform::Windows) => Self::MacOSToWindows,
            (Platform::MacOS, Platform::MacOS) => Self::MacOSToMacOS,
            _ => Self::Unknown,
        }
    }

    pub fn local_client(server: Platform) -> Self {
        Self::new(server, Platform::local())
    }

    pub fn local_server(client: Platform) -> Self {
        Self::new(Platform::local(), client)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowsToWindows => "windows-to-windows",
            Self::WindowsToMacOS => "windows-to-macos",
            Self::MacOSToWindows => "macos-to-windows",
            Self::MacOSToMacOS => "macos-to-macos",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_same_platform(self) -> bool {
        matches!(self, Self::WindowsToWindows | Self::MacOSToMacOS)
    }
}
