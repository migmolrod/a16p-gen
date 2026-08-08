use std::path::PathBuf;

/// $XDG_CONFIG_HOME, falling back to $HOME/.config per the XDG Base
/// Directory spec. Linux-only tool, so a plain env lookup is enough --
/// no need for a `dirs`-style cross-platform crate.
pub fn config_home() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config")
}

pub fn default_config_path() -> PathBuf {
    config_home().join("a16p-gen").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_config_home_env_var_takes_precedence() {
        // SAFETY: single-threaded test, no other test in this crate touches env vars.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-test-config");
        }
        assert_eq!(config_home(), PathBuf::from("/tmp/xdg-test-config"));
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn default_config_path_is_under_a16p_gen() {
        let p = default_config_path();
        assert!(p.ends_with("a16p-gen/config.toml"));
    }
}
