---
name: jenv-rs
description: >-
  Operating reference for jenv-rs, the Rust reimplementation of jenv, the Java
  version manager. Use when driving the `jenv` CLI from a script, agent or CI:
  registering a JDK with `jenv add`, pinning a version with `jenv local`,
  `jenv global` or `jenv shell`, resolving which version is active and why,
  setting up a shell with `jenv init`, or wiring JAVA_HOME. Covers the version
  resolution order, why `jenv prefix` and `jenv javahome` report the
  registration path rather than the JDK's real location, the 127 exit status
  for a command no version provides, and the fact that jenv plugins are not
  supported.
license: MIT (see LICENSE at the repository root)
---

# Operating jenv-rs

[`jenv`](https://github.com/jenv/jenv) reimplemented in Rust: same commands,
same `$JENV_ROOT` layout, same `.java-version` resolution. This document covers
the behaviour; `jenv --help` covers the flags.

## The one thing that is not like a normal CLI

**The `jenv` binary is its own shim.** `jenv rehash` hard-links the executable
into `$JENV_ROOT/shims/` once per executable found in any registered JDK's
`bin/`. A hard link reads its own file name at startup to decide what it is:
named `java` it stands in for `java`, named `jenv` it is the CLI.

Consequences worth internalising:

- There is one file to install. Never write a wrapper script that calls
  `jenv exec <tool>` in place of a shim — that reintroduces the process
  spawning this project exists to remove.
- Anything you create as a hard link to `jenv` becomes a live shim for that
  name. A link named `mvn` will make `jenv` try to find an `mvn`.
- `shims/.jenv-shim-version` records the shim generation. Bump it and rehash
  rewrites every shim.

## Data layout

```
$JENV_ROOT/                  default: ~/.jenv
  version                   the global version (legacy names: global, default)
  versions/<name>           registration: a symlink to a JDK home, or on
                            Windows a text file holding that path
  shims/<name>              hard link to the jenv binary
  shims/.jenv-shim-version  shim generation marker
  options                   extra arguments, equivalent to JENV_OPTIONS
```

The binary itself lives anywhere on `PATH`; `$JENV_ROOT` holds only state.

## How a version is selected

First match wins, in this order:

1. `$JENV_VERSION`
2. `.java-version`, walking up from `$JENV_DIR`, then from `$PWD`
3. the legacy `.jenv-version`, same walk
4. the global file (`version`, then `global`, then `default`)
5. `system` — the JDK on `PATH`

`jenv version` prints the name and its origin in one line. `jenv
version-origin` prints the file responsible. Prefer these over
reimplementing the walk: the `JENV_DIR`/`PWD` two-step and the legacy filename
chain are easy to get subtly wrong.

`system` is a version you *select*, not one that is registered. It is valid
everywhere a version name is accepted, including `jenv global system`.

## Registering a JDK

`jenv add <path>` registers a JDK under **every name it answers to**. For
Temurin 21.0.2 that is `temurin64-21.0.2`, `21.0.2`, `21.0` and `21` — four
entries, all equivalent. This multi-alias behaviour is the reason to use jenv
at all, and it is the thing `proto` and `mise` do not do.

`jenv add <name> <path>` registers only `<name>`.

Prefer `jenv add` over creating entries under `versions/` yourself. `add`
reads the JDK's `release` file first and only falls back to parsing
`java -version`, and it rehashes on its own. A directory with a `release` file
but no `bin/java` is rejected.

## Paths: registration, not reality

`jenv prefix`, `jenv javahome`, `jenv which` and the exported `JAVA_HOME` all
report the **registration path** — `$JENV_ROOT/versions/<name>` — not the JDK's
real location on disk.

This is deliberate and matches upstream jenv. The value gets pinned into IDE
configurations and exported as `JAVA_HOME`, and it has to keep naming the same
thing when the same version is re-registered. If you need the physical path,
resolve the registration path yourself.

## Commands

| Command | Effect |
|---|---|
| `jenv add [<name>] <path>` | register a JDK; rehashes |
| `jenv global [<version>]` | set or show the fallback version |
| `jenv local [<version>]` | set or show this directory's version, via `.java-version` |
| `jenv shell [<version>]` | set or show this session's version; prints a line for the shell to `eval` |
| `jenv version` | active version and where it came from |
| `jenv version-name` | just the name |
| `jenv version-origin` | just the file |
| `jenv version-file` | the file that determines the active version |
| `jenv versions` | list; `--bare` for names only, `--verbose` for resolved paths |
| `jenv prefix [<version>]` | registration path of a version, default the active one |
| `jenv javahome` | registration path, or an error under `system` |
| `jenv which <cmd>` | path that will be executed |
| `jenv whence <cmd>` | every version providing a command |
| `jenv exec <cmd> [args…]` | run a tool with the selected version's `bin/` first on `PATH` |
| `jenv rehash [--force]` | rebuild shims; `--force` clears a lock left by a crashed run |
| `jenv remove <version>…` | unregister, then rehash |
| `jenv shims [--short]` | list the shims |
| `jenv options [--verbose]` | the extra arguments that will be prepended |
| `jenv doctor` | diagnose the installation |
| `jenv root` | the `$JENV_ROOT` in effect |
| `jenv init - <shell>` | shell setup code: `PATH`, completion, prompt hook |
| `jenv completions <shell>` | completion script on stdout |
| `jenv complete <field>` | dynamic completion data, used by the generated scripts |
| `jenv info` | environment and configuration summary |
| `jenv commands` | the command list |
| `jenv skill` | this document |

`--unset` on `global`, `local` and `shell` clears rather than sets.

## Shell integration

`eval "$(jenv init -)"` is all that is required. It must be run by the shell
itself, which is why it prints rather than editing a profile. Pass the shell
explicitly (`bash`, `zsh`, `fish`, `powershell`) if auto-detection guesses
wrong. `--no-rehash` skips the initial rehash; `--no-export` disables the
`JAVA_HOME` hook.

`jenv init -` also installs a prompt hook that keeps `JAVA_HOME` and
`JDK_HOME` pointing at the selected version, because the shims alone do not
help a tool that reads the variable instead of resolving through `PATH`. It
costs one subprocess per prompt, prints nothing when the environment already
agrees, and is the built-in equivalent of jenv's `export` plugin.

## Exit status

- `0` — success
- `1` — the operation failed (version not installed, nothing removed, …)
- `127` — `jenv which` / `jenv exec` for a command no version provides. This is
  what a shell reports for a missing executable, so `if jenv which foo` works.

## Not supported

**jenv plugins, and their hooks, are ignored.** `jenv hooks` was removed with
them; there is nothing left for it to list.

The upstream plugin contract is a shell-level one: `jenv-rehash` sources a
script and then calls a function that script defined. A compiled binary cannot
be redefined from the outside, so the contract cannot survive the port. What
those plugins do reduces to two rules — shim these tool names if they are on
`PATH`, and move `$JENV_OPTIONS` into `<TOOL>_OPTS` before running one — which
is data, not a contract.

The practical effect: a tool that upstream's plugins shim (`mvn`, `gradle`,
`sbt`, `lein`, …) has to be on `PATH` already. jenv-rs will not go looking for
it.

`jenv` also does not download JDKs. Neither does this; `jenv add` registers a
JDK that is already on the machine.
