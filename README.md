# jenv-rs

[jenv](https://github.com/jenv/jenv) reimplemented in Rust, with native Windows
support and **one file** to install.

It is a drop-in replacement for the `jenv` CLI: same commands, same
`$JENV_ROOT` layout, same `.java-version` resolution. The differences are the
ones a rewrite buys — and one deliberate omission, which is the plugins.

## Why it is faster

A jenv shim is a `#!/usr/bin/env bash` script that `exec`s `jenv exec`, which is
another bash script, which calls `jenv-version-name`, `jenv-options`,
`jenv-which`, `jenv-hooks` and `jenv-whence` — roughly a dozen bash interpreters
before `java` starts.

`jenv` is its own shim: `jenv rehash` hard-links the executable into `shims/`
under every name in every JDK's `bin/`, and the hard link reads its own file
name to decide what to be. One process, straight to the tool. Measured against
jenv 0.6.0 on the same machine, same JDK, release build, 200 invocations:

| | 200 calls | per call | overhead above a bare run |
|---|---|---|---|
| no version manager | 2.963s | 14.8 ms | — |
| jenv 0.6.0 | 19.136s | 95.7 ms | 80.9 ms |
| jenv-rs | 3.935s | 19.7 ms | 4.9 ms |

**16x less dispatch overhead.** For `mvn` or `gradle`, which fork hundreds of
processes, that is the difference you feel.

## What is new

- **Windows.** `jenv add` / `local` / `global` / `exec` / `rehash` / `init` all
  work, `jenv init - powershell` is a first-class target, and the shim is a
  `.exe`. Version registrations fall back to a text file when the machine
  cannot create symlinks without an elevated token.
- **Better JDK detection.** `jenv add` reads the JDK's `release` file first and
  only falls back to parsing `java -version`. It also reports `aarch64` as
  64-bit, which the shell version gets wrong.
- **`JAVA_HOME` works.** jenv unsets it and leaves it to a plugin to put it
  back. jenv-rs keeps it in sync with the selected version by itself, which is
  the single most common thing people want from a Java version manager and the
  main reason the original has an `export` plugin.
- **Shell completion is generated** by `clap_complete` from the same
  definition the parser uses, so it cannot drift from the CLI.
- **Built-in operating notes.** `jenv skill` prints a reference written for an
  automated agent: the version resolution order, what the path-returning
  commands actually return, and what is not supported. It is compiled into the
  binary, so it is present wherever `jenv` is and cannot drift from the code.
  The output is a conforming [Agent Skills](https://agentskills.io) `SKILL.md`,
  so an agent can install it directly:

  ```sh
  mkdir -p ~/.claude/skills/jenv-rs
  jenv skill > ~/.claude/skills/jenv-rs/SKILL.md
  ```

  The directory has to be named `jenv-rs`; a skills directory is only accepted
  when its name matches the `name` in the frontmatter.

## `JAVA_HOME` follows the version

jenv's `export` plugin exists because `jenv init` *unsets* `JAVA_HOME`, and
everything that reads that variable rather than going through a shim — an IDE,
a Gradle daemon, a CI step — then has no idea which JDK you meant. The plugin's
job is to put it back, and to keep it right as you move between projects.

jenv-rs does that itself, with no plugin to install. `jenv init -` installs a
prompt hook that re-evaluates `jenv export-hook` before every prompt:

```sh
$ cat ~/work/legacy-service/.java-version
8
$ cd ~/work/legacy-service
$ echo $JAVA_HOME
/Users/you/.jenv/versions/temurin64-8.0.412.08.1
```

It is the path the version is registered under rather than the JDK's real
location, which is what you want pinned into an IDE's config: it stays stable
when you re-register the same version, and every project agrees on it.

- `JAVA_HOME` follows the selected version
- `JDK_HOME` is set as well, and only for a real JDK — one that has `javac`
- `JENV_FORCEJAVAHOME` / `JENV_FORCEJDKHOME` are set to `true`, which is what
  jenv's export plugin sets and what some launchers look for
- with no version configured, or the system version, all four are cleared
- when the environment already agrees, the hook prints nothing and `eval` is a
  no-op

Pass `--no-export` to `jenv init -` for jenv's original behaviour of leaving
those variables alone.

This costs one subprocess per prompt, the same as rbenv, pyenv and direnv. It
cannot be done without a hook, because only the shell can notice that the
working directory changed.

## What is the same

- `jenv add <path>` registers a JDK under every name it answers to
  (`temurin64-21.0.2.13`, `21.0.2.13`, `21.0`, `21`). This is the thing
  proto and mise do not do, and the reason to use jenv at all.
- Version resolution order: `JENV_VERSION` → `.java-version` (then legacy
  `.jenv-version`) walking up from `$JENV_DIR` and `$PWD` → the global file,
  which is still accepted under its `global` and `default` legacy names.
- `jenv javahome`, `jenv local`, `jenv global`, `jenv shell`, `jenv rehash`,
  `jenv exec` and the shim directory layout behave as they always have.

## Not supported: jenv plugins

jenv's eleven plugins are not supported, and their hook scripts are ignored.
What they do, for the record, is 239 lines of bash that reduces to two rules:
shim these tool names if they are on `PATH`, and move `$JENV_OPTIONS` into
`<TOOL>_OPTS` before running one of them. That is data, not a contract, and a
data-shaped feature does not need a shell interpreter to run — especially not
on a platform that does not ship one. A version manager that needs bash
installed on Windows is a version manager that does not work on Windows.

`jenv hooks` is gone with them; there is nothing left for it to list.

## Install

```sh
install -m 755 target/release/jenv ~/.local/bin/jenv
```

Then add the usual line to your shell profile:

```sh
eval "$(jenv init -)"
```

Full instructions, including Windows, are in [INSTALL.md](INSTALL.md).

## Layout

```
~/.jenv/                  or $JENV_ROOT
  version                 the global version
  versions/<name>         a symlink to a JDK home, or on Windows a file holding the path
  shims/<name>            a hard link to jenv, the same executable
```

The binary itself goes anywhere on `PATH`; `$JENV_ROOT` is only this data
directory. `shims/.jenv-shim-version` records which generation the shims were
made from. Bumping it and running `jenv rehash` rewrites all of them, which
costs one byte read instead of diffing every shim.

See [INSTALL.md](INSTALL.md) for macOS, Linux and Windows.

## Limitations

- jenv plugins are not supported. A tool that needs jenv-rs to shim it, such
  as `mvn` or `gradle`, has to be on `PATH` already; jenv-rs will not go looking
  for it the way the gradle plugin's hook did.
- `jenv` does not install JDKs. Neither does this; `jenv add` registers one that
  is already on the machine.
- On Windows, registering a version without symlink privileges stores a text
  file, so `dir versions` shows files rather than directories.

## Development

```sh
cargo test                        # unit + end-to-end against a fake JDK
cargo check --target x86_64-pc-windows-msvc
```
