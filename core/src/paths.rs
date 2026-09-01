use std::path::PathBuf;

/// 配置目录：Windows = %APPDATA%\ssh-tunnel；其余 = $XDG_CONFIG_HOME 或 ~/.config/ssh-tunnel
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("ssh-tunnel");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("ssh-tunnel");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config").join("ssh-tunnel");
        }
    }
    // 兜底：当前目录，仅在上面环境变量全缺失时才会走到
    PathBuf::from(".ssh-tunnel")
}
