# venv shell integration for bash (3.2 and later).
# `venv init` adds this line to ~/.bashrc:
#   eval "$(venv hook bash)"
#
# A child process cannot change its parent's environment, so the venv
# executable prints shell code on stdout and this function evals it. Messages
# for humans go to stderr.

# The real executable. Calling it directly skips the pixi global trampoline,
# which would overwrite CONDA_PREFIX before venv could record it.
__venv_bin=@VENV_BIN@
__venv() {
  if [ -x "$__venv_bin" ]; then "$__venv_bin" "$@"; else command venv "$@"; fi
}

venv() {
  local __venv_code
  case "${1:-}" in
    switch|use|off|new)
      __venv_code="$(__venv --emit bash "$@")" || return
      eval "$__venv_code"
      ;;
    add|remove)
      __venv "$@" || return
      hash -r
      ;;
    *)
      __venv "$@"
      ;;
  esac
}

_venv() {
  local cur prev sub i
  cur="${COMP_WORDS[COMP_CWORD]}"
  prev="${COMP_WORDS[COMP_CWORD-1]}"
  COMPREPLY=()
  if [ "$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=($(compgen -W "new switch use off list rm add remove sync run path which init hook config" -- "$cur"))
    return
  fi
  sub="${COMP_WORDS[1]}"
  if [ "$sub" = run ]; then
    for ((i = 2; i < COMP_CWORD; i++)); do
      if [ "${COMP_WORDS[i]}" = "--" ]; then
        COMPREPLY=($(compgen -c -- "$cur"))
        return
      fi
    done
  fi
  case "$prev" in
    -e|--env)
      COMPREPLY=($(compgen -W "$(__venv list --names 2>/dev/null)" -- "$cur"))
      return
      ;;
    --backend)
      COMPREPLY=($(compgen -W "pixi uv venv" -- "$cur"))
      return
      ;;
  esac
  case "$sub" in
    switch|use|path|sync|rm)
      COMPREPLY=($(compgen -W "$(__venv list --names 2>/dev/null)" -- "$cur")) ;;
    init|hook)
      COMPREPLY=($(compgen -W "zsh bash" -- "$cur")) ;;
    new)
      COMPREPLY=($(compgen -W "--backend --python --no-switch" -- "$cur")) ;;
    add|remove)
      COMPREPLY=($(compgen -W "-e --pypi" -- "$cur")) ;;
    run|which)
      COMPREPLY=($(compgen -W "-e" -- "$cur")) ;;
  esac
}
type complete >/dev/null 2>&1 && complete -F _venv venv

# Startup: keep an inherited environment's bin first on PATH (nested shells,
# tmux panes), or activate `auto_activate` from the config.
__venv_code="$(__venv --emit bash startup)" && eval "$__venv_code"
unset __venv_code
