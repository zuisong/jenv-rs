# Installing jenv-rs

One file and one data directory. That is the whole product.

## The one rule

**Put `jenv` on your `PATH`.** That is the entire installation.

The binary is also its own shim. `jenv rehash` hard-links it into a `shims/`
directory under the name of every command in every registered JDK, and the
hard link reads its own file name to decide what to be. So there is no second
file to install, no version to keep in step, and no way for a shim to end up
being a different build from the `jenv` that manages it.

Where you put it is your business: `/usr/local/bin`, `~/.local/bin`, a
Homebrew prefix, a hand-rolled `~/opt/jenv`.

```sh
install -m 755 jenv ~/.local/bin/jenv
```

## What it creates for itself

A data directory, `~/.jenv` unless you say otherwise, holding nothing but
state:

```
~/.jenv/
  shims/java           hard links to that same binary, one per tool
  versions/<name>      a link to each registered JDK
  version              the version used when no directory has its own
```

`jenv init -` puts `~/.jenv/shims` in front of your `PATH`, so `java` resolves
to a shim rather than to whatever the system installed.

## 1. Get the binaries

### From a release

One file per platform, from
[releases](https://github.com/zuisong/jenv-rs/releases). Each archive contains a
single `jenv` (or `jenv.exe`).

| Platform | Asset |
|---|---|
| Apple Silicon Mac | `jenv-aarch64-apple-darwin.tar.gz` |
| Intel Mac | `jenv-x86_64-apple-darwin.tar.gz` |
| Linux x86_64 | `jenv-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `jenv-aarch64-unknown-linux-gnu.tar.gz` |
| Windows | `jenv-x86_64-pc-windows-msvc.zip` |

`universal-apple-darwin` runs on either Mac. Windows also publishes
`aarch64-pc-windows-msvc` for ARM machines.

### From source

Needs a current Rust toolchain; there is no minimum-version pin.

```sh
git clone https://github.com/zuisong/jenv-rs
cd jenv-rs
cargo build --release
```

One binary comes out, in `target/release/`.

## 2. Get it on PATH

### macOS and Linux

Add the directory to `PATH` in your shell profile — `~/.zshrc`, `~/.bashrc`,
or `~/.config/fish/config.fish`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

If you put them in `/usr/local/bin` and it is already on `PATH`, there is
nothing to do.

### Windows (PowerShell)

```powershell
$env:PATH = "$HOME\.local\bin;$env:PATH"
```

To make it permanent, put it in your PowerShell profile. Create the file if it
is not there yet:

```powershell
if (-not (Test-Path $PROFILE)) { New-Item -ItemType File -Path $PROFILE -Force }
notepad $PROFILE
```

### If you want the data somewhere else

Optional. Skip this section unless you have a reason — `~/.jenv` is fine for
almost everyone, and the variable does not exist unless you create it.

```sh
export JENV_ROOT="$HOME/.local/share/jenv"    # macOS and Linux
```

```powershell
$env:JENV_ROOT = "$HOME\.local\share\jenv"    # Windows
```

Put it in the same profile as the `PATH` line. The shell needs it too, not just
the first command that creates the directory.

## 3. Initialize your shell

`jenv init -` prints shell code that prepends the shims directory to `PATH`,
registers tab completion, and installs a prompt hook that keeps `JAVA_HOME` in
sync with the selected version. It has to be run by the shell itself, so it
prints rather than writing to your profile.

Name your shell explicitly if you want to be sure which one gets the code.

**bash** — `~/.bashrc`

```sh
eval "$(jenv init - bash)"
```

**zsh** — `~/.zshrc`. Put it after `compinit`, or the completion half will
error with `command not found: compdef`.

```sh
autoload -U compinit && compinit
eval "$(jenv init - zsh)"
```

**fish** — `~/.config/fish/config.fish`. Fish cannot eval a command
substitution, so pipe it instead.

```fish
jenv init - fish | source
```

**PowerShell** — `$PROFILE`

```powershell
Invoke-Expression (& jenv init - powershell | Out-String)
```

Open a new shell and check:

```sh
jenv doctor
```

## 4. Register a JDK

jenv-rs does not download JDKs. Point it at one you already have:

```sh
jenv add /Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home
```

That registers every name the JDK answers to — for Temurin 21.0.2,
`temurin64-21.0.2`, `21.0.2`, `21.0` and `21`. List them with:

```sh
jenv versions
```

One at a time, as in jenv. On a machine with a dozen JDKs on it, a loop is
the practical answer:

```sh
for home in ~/jdks/*/Contents/Home; do jenv add "$home"; done
```

Give a JDK a name of your own by passing it first:

```sh
jenv add work-jdk ~/jdks/temurin-21.jdk/Contents/Home
```

`jenv add` rehashes on its own. After installing a JDK by other means, run
`jenv rehash` to pick up new executables.

## Everyday use

```sh
jenv local 21                 # this directory and below
jenv global 21                # the fallback
jenv shell 21                 # this shell session only
jenv version                  # which version, and why
jenv versions                 # everything registered
jenv exec java -jar app.jar   # run a tool against the selected version
```

`jenv local 21` writes `.java-version` into the current directory. Commit it
and everyone on the project gets the same JDK.

## Keeping `JAVA_HOME` correct

`jenv init -` installs a prompt hook that re-exports `JAVA_HOME` and
`JDK_HOME` before every prompt, so tools that read the variable instead of
going through a shim — an IDE, a Gradle daemon — see the right JDK. It costs
one subprocess per prompt, the same as rbenv and direnv.

To opt out and leave those variables alone:

```sh
eval "$(jenv init - bash --no-export)"
```

## Upgrading

Replace the one binary where it already is, then rehash. `shims/` records
which generation it holds, and `rehash` rewrites the hard links when that
changes, so there is nothing to clean up by hand.

```sh
install -m 755 jenv ~/.local/bin/jenv
jenv rehash --force
```

```powershell
install -m 755 jenv.exe "$HOME\.local\bin\jenv.exe" -Force
jenv rehash --force
```

On Windows a running `jenv` cannot be overwritten, so close any shell using
it first or the copy fails with a sharing violation. A stale shim is not fatal
in the meantime: it is the same file, and the old copy still works.

## Uninstalling

```sh
rm -rf "${JENV_ROOT:-$HOME/.jenv}"
rm ~/.local/bin/jenv
```

Then remove the `jenv init` line and the `PATH` entry from your shell profile.
Without `~/.jenv/shims` on `PATH` the shims are unreachable, so deleting that
directory is enough to stop jenv-rs affecting anything.

## Windows notes

- **Symlinks.** `versions/<name>` is normally a symlink, which needs Developer
  Mode or an elevated prompt. Without it jenv-rs writes a text file holding
  the JDK path instead; `jenv versions` then shows files rather than
  directories, and everything still works. Turn on *Settings → Privacy &
  security → For developers → Developer Mode* to get real symlinks.
- **Hard links.** The shims are hard links to `jenv.exe`, which need no
  special privilege on any filesystem that supports them. `jenv rehash` will
  fail on a network share or a FAT volume — keep the data directory on a local
  NTFS volume.
- **CTRL_C.** The shim opts out of the default console handler so that
  interrupting a JVM does not kill the shim before the tool can shut down.
