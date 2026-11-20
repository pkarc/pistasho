//! Integration code printed by `pistasho init zsh|tmux`.

use crate::config::Config;

pub fn zsh(cfg: &Config) -> String {
    let key = cfg.keys.search.replace('\'', "");
    format!(
        r#"# pistasho · zsh integration
# In ~/.zshrc:  eval "$(pistasho init zsh)"

_pistasho_search() {{
  local out
  out="$(HISTFILE="$HISTFILE" command pistasho search --query "$LBUFFER" </dev/tty)"
  local ret=$?
  if (( ret == 0 )) && [[ -n "$out" ]]; then
    BUFFER="$out"
    CURSOR=${{#BUFFER}}
  fi
  zle reset-prompt
  return 0
}}
zle -N _pistasho_search
bindkey -M emacs '{key}' _pistasho_search
bindkey -M viins '{key}' _pistasho_search
"#
    )
}

/// Command used inside the tmux popup. The popup runs with the tmux server's `PATH`, which often
/// lacks `~/.cargo/bin`, so we embed the absolute path of this binary. Falls back to the bare
/// name when the path has characters that would need escaping inside tmux's double quotes.
fn popup_exe(exe: Option<&std::path::Path>) -> String {
    match exe.and_then(|p| p.to_str()) {
        Some(p) if !p.contains(['"', '\'', '\\', '$', '#']) => format!("'{p}'"),
        _ => "pistasho".to_string(),
    }
}

pub fn tmux(cfg: &Config) -> String {
    let key = &cfg.keys.tmux;
    let exe = popup_exe(std::env::current_exe().ok().as_deref());
    format!(
        "# pistasho · tmux popup (add it to ~/.tmux.conf)\n\
         bind-key {key} display-popup -E -w 80% -h 60% \"{exe} search --send-to '#{{pane_id}}'\"\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn popup_uses_absolute_path() {
        assert_eq!(
            popup_exe(Some(Path::new("/home/u/.cargo/bin/pistasho"))),
            "'/home/u/.cargo/bin/pistasho'"
        );
    }

    #[test]
    fn popup_falls_back_on_awkward_paths() {
        assert_eq!(popup_exe(None), "pistasho");
        assert_eq!(popup_exe(Some(Path::new("/tmp/it's/pistasho"))), "pistasho");
        assert_eq!(popup_exe(Some(Path::new("/tmp/$x/pistasho"))), "pistasho");
    }
}
