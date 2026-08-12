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

/// Expand a leading `~` (bare, or `~/...`) to `$HOME`. Template
/// input/output paths in `Config::templates` use this so they can be
/// written matugen-style (`~/.config/waybar/colors.css`) instead of
/// requiring absolute paths. Anything else (relative paths, `~user/...`)
/// is returned unchanged -- only the current user's home is resolved.
pub fn expand_tilde(path: &str) -> PathBuf {
    let Ok(home) = std::env::var("HOME") else {
        return PathBuf::from(path);
    };
    if let Some(rest) = path.strip_prefix("~/") {
        PathBuf::from(home).join(rest)
    } else if path == "~" {
        PathBuf::from(home)
    } else {
        PathBuf::from(path)
    }
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

    #[test]
    fn expand_tilde_replaces_leading_slash_and_bare_forms() {
        // Both HOME-dependent cases live in one test -- separate #[test]
        // fns run in different threads by default, and two tests each
        // mutating the same env var concurrently races.
        unsafe {
            std::env::set_var("HOME", "/home/testuser");
        }
        assert_eq!(
            expand_tilde("~/.config/waybar/colors.css"),
            PathBuf::from("/home/testuser/.config/waybar/colors.css")
        );
        assert_eq!(expand_tilde("~"), PathBuf::from("/home/testuser"));
        unsafe {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn expand_tilde_leaves_other_paths_unchanged() {
        assert_eq!(
            expand_tilde("/absolute/path.toml"),
            PathBuf::from("/absolute/path.toml")
        );
        assert_eq!(
            expand_tilde("relative/path.toml"),
            PathBuf::from("relative/path.toml")
        );
    }
}
