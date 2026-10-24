# venv shell integration for zsh.
# `venv init` adds this line to ~/.zshrc:
#   eval "$(venv hook zsh)"
#
# A child process cannot change its parent's environment, so the venv
# executable prints shell code on stdout and this function evals it. Messages
# for humans go to stderr.

# The real executable. Calling it directly skips the pixi global trampoline,
# which would overwrite CONDA_PREFIX before venv could record it.
__venv_bin=@VENV_BIN@
__venv() {
  if [[ -x $__venv_bin ]]; then "$__venv_bin" "$@"; else command venv "$@"; fi
}

venv() {
  local __venv_code
  case "${1:-}" in
    switch|use|off|new)
      __venv_code="$(__venv --emit zsh "$@")" || return
      eval "$__venv_code"
      ;;
    add|remove)
      __venv "$@" || return
      rehash
      ;;
    *)
      __venv "$@"
      ;;
  esac
}

_venv() {
  local -a cmds envs
  cmds=(
    'new:创建环境并激活'
    'switch:激活环境'
    'use:激活环境（switch 的别名）'
    'off:退出当前环境'
    'list:列出所有环境'
    'rm:删除环境'
    'add:给环境加包'
    'remove:从环境删包'
    'sync:按声明文件安装或更新环境'
    'run:在环境里运行命令'
    'path:打印环境前缀'
    'which:打印环境里某个命令的路径'
    'init:把 shell 集成写进启动文件'
    'hook:打印 shell 集成代码'
    'config:查看配置'
  )
  if (( CURRENT == 2 )); then
    _describe -t commands 'venv 子命令' cmds
    return
  fi
  envs=(${(f)"$(__venv list --names 2>/dev/null)"})
  if [[ ${words[CURRENT-1]} == (-e|--env) ]]; then
    compadd -a envs
    return
  fi
  case ${words[2]} in
    switch|use|path|sync) compadd -a envs ;;
    rm) _arguments '--yes[不确认]' '--force[pixi.toml 是软链接时也删除]' '*:环境:($envs)' ;;
    new) _arguments '--backend[后端]:后端:(pixi uv venv)' '--python[python 版本]:版本:' '--no-switch[创建后不激活]' '1:名字:' ;;
    add|remove) _arguments '-e[环境]:环境:($envs)' '--pypi[PyPI 包（仅 pixi）]' '*:包:' ;;
    run) _arguments '-e[环境]:环境:($envs)' '*::命令:_normal' ;;
    which) _arguments '-e[环境]:环境:($envs)' '1:命令:_command_names' ;;
    list) _arguments '--json[输出 JSON]' ;;
    init|hook) compadd zsh bash ;;
  esac
}
(( $+functions[compdef] )) && compdef _venv venv

# Startup: keep an inherited environment's bin first on PATH (nested shells,
# tmux panes, macOS path_helper), or activate `auto_activate` from the config.
__venv_code="$(__venv --emit zsh startup)" && eval "$__venv_code"
unset __venv_code
