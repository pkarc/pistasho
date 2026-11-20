//! Código de integración que imprime `pistasho init zsh|tmux`.

use crate::config::Config;

pub fn zsh(cfg: &Config) -> String {
    let key = cfg.keys.search.replace('\'', "");
    format!(
        r#"# pistasho · integración con zsh
# En ~/.zshrc:  eval "$(pistasho init zsh)"

_pistasho_search() {{
  local out
  out="$(command pistasho search --query "$LBUFFER" </dev/tty)"
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

pub fn tmux(cfg: &Config) -> String {
    let key = &cfg.keys.tmux;
    format!(
        "# pistasho · popup de tmux (agrégalo a ~/.tmux.conf)\n\
         bind-key {key} display-popup -E -w 80% -h 60% \"pistasho search --send-to '#{{pane_id}}'\"\n"
    )
}
