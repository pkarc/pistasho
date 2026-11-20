//! Integration code printed by `pistasho init zsh|tmux`.

use crate::config::Config;

/// The zsh widget behind `Ctrl-Space`: a question on the prompt line, the best command left in
/// `BUFFER`. Never runs it. `pistasho ask --format zsh` prints `command NUL message NUL question`
/// (zsh keeps NUL bytes in `$(...)`, which is what lets a multiline command travel safely).
///
/// The widget remembers the last question and the command it put on the prompt. If `Ctrl-Space`
/// is pressed again while the prompt still holds exactly that command, it opens the other
/// options for the same question instead of asking a new one; any edit, or running the line,
/// makes the next `Ctrl-Space` a new question.
pub fn zsh(cfg: &Config) -> String {
    let key = cfg.keys.ask.replace('\'', "");
    format!(
        r#"# pistasho · zsh integration
# In ~/.zshrc:  eval "$(pistasho init zsh)"

_pistasho_ask() {{
  local out ret
  local -a parts
  # HISTFILE is a plain zsh variable; pass it along so history can be read
  if [[ -n "$BUFFER" && -n "$_PISTASHO_LAST_Q" && "$BUFFER" == "$_PISTASHO_LAST_CMD" ]]; then
    out="$(HISTFILE="$HISTFILE" command pistasho ask --alternatives --current "$BUFFER" --format zsh -- "$_PISTASHO_LAST_Q" </dev/tty)"
  else
    out="$(HISTFILE="$HISTFILE" command pistasho ask --prompt --format zsh --initial "$BUFFER" </dev/tty)"
  fi
  ret=$?
  if (( ret == 0 )) && [[ -n "$out" ]]; then
    parts=("${{(@0)out}}")
    BUFFER="${{parts[1]}}"
    CURSOR=${{#BUFFER}}
    _PISTASHO_LAST_CMD="$BUFFER"
    _PISTASHO_LAST_Q="${{parts[3]:-}}"
    zle reset-prompt
    [[ -n "${{parts[2]}}" ]] && zle -M "${{parts[2]}}"
  else
    zle reset-prompt
    (( ret == 1 )) && [[ -n "$out" ]] && zle -M "$out"
  fi
  return 0
}}
# forget the question when the line is accepted, so it cannot leak into the next prompt
_pistasho_forget() {{
  unset _PISTASHO_LAST_Q _PISTASHO_LAST_CMD
}}
autoload -Uz add-zle-hook-widget
add-zle-hook-widget zle-line-finish _pistasho_forget
zle -N _pistasho_ask
bindkey -M emacs '{key}' _pistasho_ask
bindkey -M viins '{key}' _pistasho_ask
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

/// tmux does not expand formats such as `#{pane_id}` inside the command of `display-popup`
/// (the process would receive the literal text), but `run-shell` does. So the binding goes
/// through `run-shell`, which resolves the pane and the client and then starts the popup with
/// plain values. The popup's own `TMUX_PANE` is the popup, not the pane we must write to.
pub fn tmux(cfg: &Config) -> String {
    tmux_binding(
        &cfg.keys.tmux,
        &popup_exe(std::env::current_exe().ok().as_deref()),
    )
}

fn tmux_binding(key: &str, exe: &str) -> String {
    format!(
        "# pistasho · tmux popup (add it to ~/.tmux.conf)\n\
         bind-key {key} run-shell -b \"tmux display-popup -c #{{client_name}} -t #{{pane_id}} -E -w 80% -h 60% \\\"{exe} ask --prompt --send-to #{{pane_id}}\\\"\"\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Regression: a real popup received the literal `#{pane_id}` and `tmux send-keys`
    /// failed, so nothing reached the prompt. Every format must sit in the `run-shell` part,
    /// where tmux expands it, never inside the popup's own command as a quoted literal.
    #[test]
    fn tmux_binding_expands_formats_in_run_shell() {
        let line = tmux_binding("g", "'/home/u/.cargo/bin/pistasho'")
            .lines()
            .find(|l| l.starts_with("bind-key"))
            .unwrap()
            .to_string();
        assert_eq!(
            line,
            "bind-key g run-shell -b \"tmux display-popup -c #{client_name} -t #{pane_id} -E -w 80% -h 60% \\\"'/home/u/.cargo/bin/pistasho' ask --prompt --send-to #{pane_id}\\\"\""
        );
        assert!(
            !line.contains("'#{pane_id}'"),
            "a quoted format is not expanded by display-popup"
        );
    }

    /// ADR 0001: Ctrl-Space is the only entry point in zsh.
    #[test]
    fn zsh_widget_is_the_question_one() {
        let out = zsh(&Config::default());
        assert!(out.contains("zle -N _pistasho_ask"));
        assert!(out.contains("bindkey -M emacs '^ ' _pistasho_ask"));
        assert!(out.contains("ask --prompt --format zsh"));
        // a second Ctrl-Space on the unedited answer opens the alternatives for the same question
        assert!(out.contains("ask --alternatives --current \"$BUFFER\""));
        assert!(out.contains("\"$BUFFER\" == \"$_PISTASHO_LAST_CMD\""));
        assert!(out.contains("add-zle-hook-widget zle-line-finish _pistasho_forget"));
        assert!(!out.contains("_pistasho_search"), "the Ctrl-G widget is gone");
        // the widget must pass HISTFILE along: it is a plain zsh variable, not exported
        assert!(out.contains("HISTFILE=\"$HISTFILE\""));
    }

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
