# venv

管理 `~/.venvs` 下的具名 Python 环境，后端可选 pixi、uv、python-venv（`python -m venv` + pip），在当前 shell 里切换。

```
$ venv new ml --backend uv --python 3.12
→ 创建 ml（uv，Python 3.12）
✔ 已创建 ~/.venvs/ml
✔ 已切换到 ml
  路径    ~/.venvs/ml/.venv
  Python  3.12.13
$ venv add numpy
✔ 已向 ml 加入 numpy
$ venv list
* ml  uv    3.12.13
$ venv off
✔ 已退出 ml
```

每个环境是 `$VENVS_HOME/<名字>/` 下的一个目录。目录里的声明文件（`pixi.toml`、`pyproject.toml` 或 `requirements.txt`）是唯一的真相，可以放进 dotfiles 仓库；环境本体（`.pixi/`、`.venv/`）随时可以删掉，用 `venv sync` 重建。

## 安装

pixi（推荐）：

```sh
pixi global install -c https://chenghongxu.github.io/packages/conda -c conda-forge venv
```

或者从 [Releases](https://github.com/ChengHongxu/venv/releases) 下载静态二进制（`venv-linux-amd64`、`venv-linux-arm64`、`venv-darwin-arm64`，附 `SHA256SUMS`），或者从源码：`cargo install --locked --git https://github.com/ChengHongxu/venv`。

pixi、uv、python 都是外部命令，用到哪个后端装哪个。

## shell 集成

子进程改不了父 shell 的环境变量，所以 `switch`、`off`、`new` 要靠一个同名 shell 函数。运行一次：

```sh
venv init        # 自动识别 zsh / bash，也可以写 venv init zsh
```

它在 `~/.zshrc`（或 `$ZDOTDIR/.zshrc`、`~/.bashrc`）末尾加上：

```sh
# venv
eval "$(venv hook zsh)"
```

然后重开终端。已经有这一行就不改；启动文件是软链接时改它指向的文件，链接保持不变。没加载集成就运行 `venv switch` 会提示该做什么，不改任何文件。

## 命令

| 命令 | 作用 |
| --- | --- |
| `venv new <名字> [--backend pixi\|uv\|venv] [--python 3.12] [--no-switch]` | 创建环境并激活 |
| `venv switch [名字]`（别名 `use`） | 激活环境；不带名字时列出所有环境，`*` 标出当前环境 |
| `venv off` | 退出当前环境 |
| `venv list [--json]` | 名字、后端、Python 版本、是否当前环境。不启动 python，版本从 `conda-meta/` 或 `pyvenv.cfg` 读 |
| `venv rm <名字> [--yes] [--force]` | 删除环境（要确认）。正在用的环境要先 `off`；`pixi.toml` 是软链接时要加 `--force` |
| `venv add [-e 名字] [--pypi] <包...>` | 给当前环境（或 `-e` 指定的环境）加包，在任何目录下都生效。`--pypi` 只用于 pixi |
| `venv remove [-e 名字] [--pypi] <包...>` | 删包 |
| `venv sync [名字...]` | 按声明文件安装或更新环境，不带名字时处理全部。适合 dotfiles 部署时预装 |
| `venv run -e <名字> -- <命令...>` | 在环境里运行命令，不需要激活 |
| `venv path [名字]` | 打印环境前缀 |
| `venv which [-e 名字] <命令>` | 打印环境里某个命令的路径 |
| `venv init [zsh\|bash]` / `venv hook <zsh\|bash>` | 写入启动文件 / 打印集成代码 |
| `venv config` | 配置文件路径，以及每一项的当前值和来源 |

激活后导出 `VENV_NAME=<名字>`，给提示符用：

```toml
# ~/.config/starship.toml
[env_var.VENV_NAME]
format = '[🐍 $env_value]($style) '
style = 'bold yellow'
```

## 配置

`~/.config/venv/config.toml`（或 `$XDG_CONFIG_HOME/venv/config.toml`），所有项都可以不写。写错的键会报错，不会被悄悄忽略。

```toml
home = "~/.venvs"              # 环境变量 VENVS_HOME 优先
default_backend = "pixi"       # pixi | uv | venv
default_python = "3.14"
auto_activate = "default"      # 新 shell 里没有激活任何环境时激活它；默认不写，不自动激活

[pixi]
channels = ["conda-forge"]     # venv new 时传给 pixi init
exe = "pixi"                   # 可以写绝对路径；默认在 PATH 和 ~/.pixi/bin 里找

[uv]
exe = "uv"                     # 默认在 PATH、~/.local/bin、~/.cargo/bin 里找

[venv]
python = "/usr/bin/python3.12" # python-venv 后端没指定 --python 时用的解释器
```

## 后端的差异

| | pixi | uv | python-venv |
| --- | --- | --- | --- |
| 声明文件 | `pixi.toml` + `pixi.lock` | `pyproject.toml`（`[tool.uv] package = false`）+ `uv.lock` + `.python-version` | `requirements.txt` + `.python-version` |
| 环境本体 | `.pixi/envs/default` | `.venv/` | `.venv/` |
| Python 来源 | conda-forge 的 `python` 包 | uv 管理的 Python（`python-preference = "only-managed"`，没有就下载） | PATH 里的 `python3.X`，或配置的 `venv.python` |
| 加包 | `pixi add`；`--pypi` 写进 `pypi-dependencies` | `uv add` | `pip install`，再把这一行写进 `requirements.txt` |
| 锁版本 | 是 | 是 | 否 |
| 激活 | `pixi shell-hook --json` 给出的变量（CONDA_PREFIX 等），结果缓存 | `VIRTUAL_ENV` + PATH | `VIRTUAL_ENV` + PATH |

几点说明：

- 后端按目录里的文件识别，顺序是 `pixi.toml`、`pyproject.toml`、`requirements.txt`。只有 `pyvenv.cfg` 的目录（裸 `python -m venv`）会被忽略。
- python-venv 的 `requirements.txt` 只记直接依赖：`add`/`remove` 只改命令行上那几个包对应的行，注释、`-r`、`-e` 和手写的版本约束都保留。`pip uninstall` 不会删掉被删包的依赖。要锁版本请用 uv。
- python-venv 在 Debian/Ubuntu 上需要 `python3-venv`（`ensurepip`），没有时会提示。
- uv 和 python-venv 的 `.venv/` 不存在时，`switch` 和 `run` 会先按声明文件建好。pixi 环境没装时 pixi 自己会装。
- 激活不用 `bin/activate`，也不用 `pixi shell-hook` 生成的脚本（它会定义一个 `pixi()` 函数）。venv 自己算出要改的变量，记下原值，`off` 时精确还原（不只是 PATH）。

### pixi.toml 是软链接

在 dotfiles 里，`~/.venvs/default/pixi.toml` 可以是指向仓库的软链接。venv 总是把真实路径传给 pixi，所以 `pixi.lock` 和 `.pixi/` 在仓库那边（记得把 `.pixi/` 加进仓库的 `.gitignore`），`venv add` 改的也是仓库里的文件。`venv list` 会标出链接目标；`venv rm` 不加 `--force` 会拒绝，加了会删掉 `~/.venvs/default` 和仓库那边的 `.pixi/`，仓库里的 `pixi.toml`、`pixi.lock` 不动。

## 从 vswitch 迁移

| vswitch（venv.zsh） | venv |
| --- | --- |
| `vswitch <名字>` | `venv switch <名字>`（或 `venv use`） |
| `vswitch`（不带参数） | `venv switch` 或 `venv list` |
| `vdeactivate` | `venv off` |
| `vnew <名字> [3.12]` | `venv new <名字> [--python 3.12]` |
| `vadd [--pypi] <包>` | `venv add [--pypi] <包>` |
| `vrm [--pypi] <包>` | `venv remove [--pypi] <包>` |
| `$VSWITCH_ENV` | `$VENV_NAME` |
| `.zshrc` 末尾的 `[[ -z $VSWITCH_ENV ]] && vswitch default` | 配置 `auto_activate = "default"` |
| `.zshrc` 里的 `[[ -n $VSWITCH_ENV && -n $CONDA_PREFIX ]] && path=(...)` | 不需要，见下文 |
| `run_onchange` 脚本里的 `pixi install` 循环 | `venv sync` |
| starship `[env_var.VSWITCH_ENV]` | `[env_var.VENV_NAME]` |

### 嵌套 shell 和 tmux

激活状态记在导出的 `_VENV_STATE` 里，子 shell 和 tmux 新窗格会继承。hook 在启动时检查它：环境还在，就把环境的 bin 挪回 PATH 最前（rc 文件或 macOS 的 path_helper 可能把别的目录放到了前面）；环境已经删了，就退出它。所以 `.zshrc` 里那一行可以删掉。

hook 只在交互 shell 里运行（它在 `.zshrc` 里）。非交互的 `zsh -c` 和脚本继承的 PATH 里环境的 bin 本来就在最前，但如果 `.zshenv` 又往前加了目录（比如 `~/.pixi/bin`），它就不在最前了。在意这种情况的话，可以在 `.zshenv` 里保留一行：

```zsh
[[ -n $VENV_NAME ]] && path=("${VIRTUAL_ENV:-$CONDA_PREFIX}/bin" $path)
```

## 启动速度

每开一个 shell 会执行两次 venv：`venv hook` 输出函数定义，函数再调用 `venv --emit zsh startup` 处理继承的状态或 `auto_activate`。pixi 的激活结果缓存在 `~/.cache/venv/pixi/<名字>.json`，key 是 `pixi.toml` 和 `pixi.lock` 的内容、manifest 的真实路径和 pixi 可执行文件的 mtime，任何一个变了就重新运行 `pixi shell-hook`。`add`、`remove`、`sync`、`new` 之后会顺手更新缓存。

测量方法（[hyperfine](https://github.com/sharkdp/hyperfine)，`pixi exec hyperfine` 不用安装）。在配置了 `auto_activate` 的 shell 外面运行：

```sh
hyperfine -N -w 5 \
  "zsh -f -c true" \
  "zsh -f -c 'eval \"\$(venv hook zsh)\"'"
```

两者之差就是 venv 带来的开销（集成加上自动激活）。在一台 x86_64 Linux 上，`auto_activate` 指向一个 pixi 环境、缓存命中时的结果：

| | 平均 |
| --- | --- |
| `zsh -f -c true` | 0.8 ms |
| `zsh -f -c 'eval "$(venv hook zsh)"'` | 2.9 ms |
| `bash --norc -c 'eval "$(venv hook bash)"'` | 3.0 ms |
| 其中 `venv hook zsh` | 0.5 ms |
| 其中 `venv --emit zsh startup` | 0.7 ms |

也就是每个 shell 多 2 ms 左右，目标是 20 ms 以内。这是直接运行二进制测的；通过 `pixi global install` 装的 `venv` 在 `venv hook` 这一步还要经过 pixi 的 trampoline，之后的调用都直接走真实的二进制。缓存没命中时（第一次、或 lock 变了）要运行一次 `pixi shell-hook`，几十毫秒到几秒，取决于环境要不要安装。

## 实现说明

- `pixi global install` 装出来的 `~/.pixi/bin/venv` 是一个 trampoline，它启动真实二进制前会改写 `CONDA_PREFIX`、`CONDA_SHLVL` 和 PATH。所以 `venv hook` 把真实二进制的路径写进 shell 函数，之后都直接调用它。在脚本里直接运行 `venv run`、`venv sync` 时会经过 trampoline，venv 会去掉它加的这几个变量；如果你原本就设置了 `CONDA_PREFIX`，这种情况下原值拿不回来。
- `--emit` 模式下 stdout 只有要 eval 的代码；提示、clap 的帮助和报错、pixi/uv/pip 的输出都在 stderr。出错时 stdout 为空，当前环境不变。
- 所有后端命令都在"去掉当前激活"的环境里运行，所以从 A 切到 B 时 A 的变量不会混进 B，uv 也不会抱怨 `VIRTUAL_ENV` 不匹配。

## 开发

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

- 单元测试：后端识别、激活/还原算法（PATH 有空格、被 `typeset -U` 去重、被 path_helper 重排、用户后来加了条目）、pixi 输出的分类、缓存 key、引号转义（交给真实的 zsh 和 bash 执行）、`requirements.txt` 的修改、配置。
- `tests/shell.rs`：用 `zsh -f -c` 和 `bash --norc -c` 跑 `eval "$(venv hook ...)"`，检查 switch 后的变量，以及 off 之后完整环境和激活前逐字节一致；还有嵌套 shell、自动激活、缓存命中。环境是在临时目录里伪造的，pixi 用一个桩脚本代替，不需要网络。
- `tests/backends.rs`：真实的 pixi、uv、python 各跑一遍 new、add、switch、off、rm，以及 `pixi.toml` 是软链接的情况。本机缺哪个工具就跳过哪个；CI 设置了 `VENV_REQUIRE_BACKENDS=1`，缺了就失败。它们用临时的 HOME 和 VENVS_HOME，但共用 pixi 和 uv 真实的下载缓存。

## 发布

1. 在 `Cargo.toml` 和 `recipe/recipe.yaml` 里改版本号，提交
2. `git tag vX.Y.Z && git push origin vX.Y.Z`
3. GitHub Actions 检查三处版本号一致，构建三个静态二进制和 `SHA256SUMS` 发到 Releases，再用 rattler-build 打 conda 包，最后通知 [ChengHongxu/packages](https://github.com/ChengHongxu/packages) 更新 channel。最后一步需要仓库 secret `CHANNEL_DISPATCH_TOKEN`

## License

MIT
