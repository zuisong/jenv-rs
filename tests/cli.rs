//! End-to-end tests that drive the real `jenv` binary against a fake JDK.
//!
//! Every test gets its own `$JENV_ROOT` and its own working directory, and
//! the child process is given both explicitly, so the suite runs in parallel
//! without a lock and without leaking `JENV_VERSION` or `PATH` between tests.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const JENV: &str = env!("CARGO_BIN_EXE_jenv");

struct Sandbox {
    dir: PathBuf,
    root: PathBuf,
    jdk: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "jenv-rs-it-{}-{}-{name}",
            std::process::id(),
            next_id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        // `/tmp` is a symlink to `/private/tmp` on macOS, and jenv reports
        // resolved paths. Canonicalizing here keeps the two comparable.
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);

        let jdk = dir.join("jdk-21");
        std::fs::create_dir_all(jdk.join("bin")).unwrap();
        // The canonical root is only known once it exists.
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        let root = dir.join("root");
        let jdk = dir.join("jdk-21");
        std::fs::write(
            jdk.join("release"),
            "JAVA_VERSION=\"21.0.2\"\n\
             IMPLEMENTOR=\"Eclipse Adoptium\"\n\
             IMPLEMENTOR_VERSION=\"Eclipse Temurin-21.0.2+13\"\n\
             OS_ARCH=\"aarch64\"\n",
        )
        .unwrap();
        write_script(&jdk.join("bin").join("java"), "echo \"java $*\"");

        // The binaries go in a directory that has nothing to do with
        // $JENV_ROOT and are reached only through PATH, which is the whole
        // point: jenv-rs imposes no install layout.
        // A single file, reachable only through PATH. `rehash` hard-links
        // this same binary into `shims/`, so there is nothing else to place.
        std::fs::create_dir_all(root.join("bin")).unwrap();
        copy(JENV, &root.join("bin").join(binary_name("jenv")));

        let sandbox = Self { dir, root, jdk };
        sandbox.project();
        sandbox
    }

    fn project(&self) -> PathBuf {
        let dir = self.dir.join("project");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn shims(&self) -> PathBuf {
        self.root.join("shims")
    }

    /// A `java` on the sandbox's own `PATH` that no jenv version provides, so
    /// "the system JDK" is something a test can name. Its home is `$JENV_ROOT`,
    /// because it sits in `$JENV_ROOT/bin`.
    fn install_system_java(&self) {
        write_script(&self.root.join("bin").join("java"), "echo system java");
    }

    /// Where `name` is registered, as `jenv prefix` and `jenv which` report it.
    fn registered(&self, name: &str) -> PathBuf {
        self.root.join("versions").join(name)
    }

    fn shim_names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.shims())
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// `PATH` for a child: the installed binaries first, then enough of the
    /// system to run them. `env_clear()` means a shim that cannot find `jenv`
    /// fails here rather than silently picking up the developer's own.
    fn bin_on_path(&self) -> String {
        format!(
            "{}{}{}",
            self.root.join("bin").display(),
            jenv::proc::SEPARATOR,
            env_path()
        )
    }

    fn command(&self, cwd: &Path) -> Invocation {
        let mut command = Command::new(self.root.join("bin").join(binary_name("jenv")));
        command
            .current_dir(cwd)
            .env_clear()
            .env("PATH", self.bin_on_path())
            .env("JENV_ROOT", &self.root)
            .env("JENV_SHELL", "bash")
            // A `.java-version` anywhere above the sandbox would silently win,
            // so the walk is pinned to the sandbox.
            .env("JENV_DIR", cwd);
        Invocation { command }
    }

    fn run(&self, args: &[&str]) -> Run {
        self.finish(self.command(&self.project()).args(args).output())
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Run {
        self.finish(self.command(cwd).args(args).output())
    }

    fn finish(&self, output: Output) -> Run {
        Run {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Run an arbitrary installed program (a shim) with the sandbox wired up.
    fn spawn(&self, program: &Path, args: &[&str]) -> Output {
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(self.project())
            .env_clear()
            .env("PATH", self.bin_on_path())
            .env("JENV_ROOT", &self.root)
            .env("JENV_SHELL", "bash")
            .env("JENV_DIR", self.project());
        // A shim is a hard link to the sandbox binary, so it is as exposed to
        // ETXTBSY as the binary itself.
        output_of(command).unwrap()
    }

    /// Run `jenv <args>` and hand back stdout as text.
    fn spawn_text(&self, args: &[&str]) -> String {
        let output = self.command(&self.project()).args(args).output();
        assert!(
            output.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Same, with a pre-existing environment, so the hook's "already correct,
    /// say nothing" path can be told apart from "needs changing".
    fn spawn_text_with(&self, preset: &[(&str, &str)], args: &[&str]) -> String {
        let mut command = self.command(&self.project());
        for (name, value) in preset {
            command = command.env(name, value);
        }
        let output = command.args(args).output();
        assert!(
            output.status.success(),
            "stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Run {
    fn lines(&self) -> Vec<&str> {
        self.stdout.lines().collect()
    }

    #[track_caller]
    fn succeeds(self) -> Self {
        assert!(self.ok, "expected success, stderr:\n{}", self.stderr);
        self
    }

    #[track_caller]
    fn fails(self) -> Self {
        assert!(!self.ok, "expected failure, stdout:\n{}", self.stdout);
        self
    }

    #[track_caller]
    fn stdout_is(self, expected: &str) -> Self {
        assert_eq!(self.stdout.trim_end(), expected, "stderr:\n{}", self.stderr);
        self
    }

    #[track_caller]
    fn contains(self, needle: &str) -> Self {
        assert!(
            self.stdout.contains(needle) || self.stderr.contains(needle),
            "expected {needle:?} in output, got:\nstdout: {}\nstderr: {}",
            self.stdout,
            self.stderr
        );
        self
    }
}

fn binary_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// A deliberately minimal PATH.
///
/// The developer's own PATH usually contains their real jenv shims, and a test
/// that resolves `java` through those is testing the wrong thing entirely.
fn env_path() -> String {
    if cfg!(windows) {
        ["C:\\Windows\\System32", "C:\\Windows"].join(";")
    } else {
        "/usr/bin:/bin".to_string()
    }
}

fn next_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// A `jenv` invocation being assembled.
///
/// `output` is the only way to run one, so the busy-executable retry cannot be
/// bypassed by a test that builds its own command chain.
struct Invocation {
    command: Command,
}

impl Invocation {
    fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.command.args(args);
        self
    }

    fn env(
        mut self,
        name: impl AsRef<std::ffi::OsStr>,
        value: impl AsRef<std::ffi::OsStr>,
    ) -> Self {
        self.command.env(name, value);
        self
    }

    fn output(self) -> Output {
        output_of(self.command).unwrap()
    }
}

/// `ETXTBSY`: exec refused because the file is open for writing somewhere.
const TEXT_FILE_BUSY: i32 = 26;

/// Run a child, retrying briefly while the kernel calls the executable busy.
///
/// Every sandbox copies the freshly built binary into place and then runs it.
/// On a Linux CI runner the temp directory sits on an overlayfs upper layer,
/// and exec'ing a file written a moment earlier can come back as `ETXTBSY`
/// while the write is still visible. It is a property of the filesystem, not
/// of anything under test, and it does not reproduce on a normal disk.
///
/// Retrying is safe here because `ETXTBSY` is raised by `exec` itself: the
/// binary never starts, so nothing it would have done is being skipped, and no
/// assertion can be masked by it. Matching the errno rather than
/// `io::ErrorKind::ExecutableFileBusy` keeps this building on stable, where
/// that variant is not available.
fn output_of(mut command: Command) -> std::io::Result<Output> {
    for _ in 0..40 {
        match command.output() {
            Err(e) if e.raw_os_error() == Some(TEXT_FILE_BUSY) => back_off(),
            other => return other,
        }
    }
    unreachable!("the loop only exits by returning")
}

fn back_off() {
    std::thread::sleep(std::time::Duration::from_millis(25));
}

fn copy(from: &str, to: &Path) {
    std::fs::copy(from, to).unwrap_or_else(|e| panic!("copy {from} -> {to:?}: {e}"));
}

fn write_script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

// ---------------------------------------------------------------- add

#[test]
fn add_registers_a_jdk_under_every_alias_it_answers_to() {
    let sandbox = Sandbox::new("add-aliases");
    let jdk = sandbox.jdk.display().to_string();

    let run = sandbox.run(&["add", &jdk]).succeeds();
    assert!(
        run.stdout.contains("temurin64-21.0.2 added"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("21 added"), "{}", run.stdout);
    assert!(run.stdout.contains("21.0 added"), "{}", run.stdout);

    let listed = sandbox.run(&["versions", "--bare"]).succeeds();
    for alias in ["temurin64-21.0.2", "21.0.2", "21.0", "21"] {
        assert!(
            listed.lines().contains(&alias),
            "missing {alias} in {:?}",
            listed.lines()
        );
    }
}

#[test]
fn add_falls_back_to_the_version_banner_without_a_release_file() {
    let sandbox = Sandbox::new("add-banner");
    // A JRE-shaped install: no release file, so the banner is all there is.
    let jre = sandbox.dir.join("jre");
    std::fs::create_dir_all(jre.join("bin")).unwrap();
    write_script(
        &jre.join("bin").join("java"),
        "echo 'openjdk version \"17.0.9\" 2023-10-17' >&2\n\
         echo 'Zulu17.46.19-CA' >&2\n\
         echo 'OpenJDK 64-Bit Server VM (build 17.0.9+9-LTS, mixed mode)' >&2\n\
         exit 0",
    );

    sandbox
        .run(&["add", &jre.display().to_string()])
        .succeeds()
        .contains("zulu64-17.0.9 added");
}

#[test]
fn add_with_an_explicit_alias_registers_only_that_name() {
    let sandbox = Sandbox::new("add-custom");
    sandbox
        .run(&["add", "my-jdk", &sandbox.jdk.display().to_string()])
        .succeeds()
        .stdout_is("my-jdk added");

    let listed = sandbox.run(&["versions", "--bare"]).succeeds();
    assert_eq!(listed.lines(), vec!["my-jdk"]);
}

#[test]
fn add_rejects_a_directory_that_is_not_a_jdk() {
    let sandbox = Sandbox::new("add-invalid");
    let empty = sandbox.dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();

    sandbox
        .run(&["add", &empty.display().to_string()])
        .fails()
        .contains("is not a valid path to java installation");
}

#[test]
fn add_rejects_a_release_file_with_no_java_binary() {
    let sandbox = Sandbox::new("add-release-only");
    // A `release` file describes a JDK, it does not make one. Registering this
    // would produce a version that `jenv local` accepts and that then fails on
    // every `java` invocation.
    let fake = sandbox.dir.join("release-only");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(
        fake.join("release"),
        "JAVA_VERSION=\"21.0.2\"\nIMPLEMENTOR=\"Eclipse Adoptium\"\nOS_ARCH=\"x86_64\"\n",
    )
    .unwrap();

    sandbox
        .run(&["add", &fake.display().to_string()])
        .fails()
        .contains("is not a valid path to java installation");
    assert!(!sandbox.root.join("versions").join("21").exists());
}

#[test]
fn add_names_an_unrecognised_architecture_64_bit() {
    let sandbox = Sandbox::new("add-arch");

    for (arch, expected) in [
        ("ppc64le", "temurin64"),
        ("s390x", "temurin64"),
        ("x86", "temurin32"),
    ] {
        let jdk = sandbox.dir.join(format!("jdk-{arch}"));
        std::fs::create_dir_all(jdk.join("bin")).unwrap();
        std::fs::write(
            jdk.join("release"),
            format!(
                "JAVA_VERSION=\"21.0.2\"\nIMPLEMENTOR=\"Eclipse Adoptium\"\nOS_ARCH=\"{arch}\"\n"
            ),
        )
        .unwrap();
        write_script(&jdk.join("bin").join("java"), "echo java");
        sandbox.run(&["add", &jdk.display().to_string()]).succeeds();
        // Reading an unknown architecture as 32-bit would register a 64-bit
        // JDK under a `...32-...` name, which nothing downstream could correct.
        assert!(
            sandbox
                .root
                .join("versions")
                .join(format!("{expected}-21.0.2"))
                .exists(),
            "{arch} should register as {expected}"
        );
    }
}

#[test]
fn add_twice_keeps_the_existing_registration() {
    let sandbox = Sandbox::new("add-twice");
    let jdk = sandbox.jdk.display().to_string();
    sandbox.run(&["add", &jdk]).succeeds();

    let second = sandbox.run(&["add", &jdk]).succeeds();
    assert!(
        second.stdout.contains("already present, skip installation"),
        "{}",
        second.stdout
    );
    assert!(!second.stdout.contains("temurin64-21.0.2 added"));
}

// ------------------------------------------------------------ selection

#[test]
fn local_writes_a_file_that_then_selects_the_version() {
    let sandbox = Sandbox::new("local");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();

    sandbox.run(&["local", "21"]).succeeds();
    assert_eq!(
        std::fs::read_to_string(sandbox.project().join(".java-version")).unwrap(),
        "21\n"
    );

    sandbox.run(&["version-name"]).succeeds().stdout_is("21");
    sandbox
        .run(&["version-origin"])
        .succeeds()
        .contains(".java-version");
    // The registration path, not the JDK's real location: this is what gets
    // pinned into an IDE config and exported as JAVA_HOME, and jenv keeps it
    // stable across a re-registration.
    sandbox
        .run(&["prefix"])
        .succeeds()
        .stdout_is(sandbox.registered("21").display().to_string().as_str());
}

#[test]
fn local_refuses_an_uninstalled_version_and_writes_nothing() {
    let sandbox = Sandbox::new("local-missing");
    sandbox
        .run(&["local", "99"])
        .fails()
        .contains("version `99' not installed");
    assert!(!sandbox.project().join(".java-version").exists());
}

#[test]
fn a_version_file_in_a_parent_directory_wins_over_the_global_one() {
    let sandbox = Sandbox::new("walk-up");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["global", "21"]).succeeds();

    let nested = sandbox.project().join("src/main");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(sandbox.project().join(".java-version"), "21\n").unwrap();

    sandbox
        .run_in(&nested, &["version-name"])
        .succeeds()
        .stdout_is("21");
}

#[test]
fn java_version_takes_precedence_over_the_legacy_jenv_version() {
    let sandbox = Sandbox::new("legacy-file");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    std::fs::write(sandbox.project().join(".java-version"), "21\n").unwrap();
    std::fs::write(sandbox.project().join(".jenv-version"), "17\n").unwrap();

    sandbox.run(&["version-name"]).succeeds().stdout_is("21");
}

#[test]
fn local_migrates_a_legacy_file_instead_of_shadowing_it() {
    let sandbox = Sandbox::new("legacy-migrate");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    std::fs::write(sandbox.project().join(".jenv-version"), "21\n").unwrap();

    sandbox
        .run(&["local", "21"])
        .succeeds()
        .contains("migrated");
    assert!(!sandbox.project().join(".jenv-version").exists());
    assert!(sandbox.project().join(".java-version").exists());
}

#[test]
fn local_unset_clears_both_file_names() {
    let sandbox = Sandbox::new("local-unset");
    std::fs::write(sandbox.project().join(".java-version"), "21\n").unwrap();
    std::fs::write(sandbox.project().join(".jenv-version"), "17\n").unwrap();

    sandbox.run(&["local", "--unset"]).succeeds();
    assert!(!sandbox.project().join(".java-version").exists());
    assert!(!sandbox.project().join(".jenv-version").exists());
}

#[test]
fn the_environment_variable_outranks_every_file() {
    let sandbox = Sandbox::new("env-override");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    std::fs::write(sandbox.project().join(".java-version"), "21\n").unwrap();

    let output = sandbox
        .command(&sandbox.project())
        .args(["version-name"])
        .env("JENV_VERSION", "21.0.2")
        .output();
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "21.0.2");
}

#[test]
fn an_uninstalled_version_is_an_error_not_a_silent_fallback() {
    let sandbox = Sandbox::new("missing-version");
    let output = sandbox
        .command(&sandbox.project())
        .args(["version-name"])
        .env("JENV_VERSION", "21")
        .output();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not installed"));
}

#[test]
fn system_is_a_version_you_can_select_wherever_versions_are_written() {
    let sandbox = Sandbox::new("select-system");
    // `system` is not a registration, so it has to be admitted by every
    // writer — `jenv shell system` accepts it, and the other two used to
    // disagree with it.
    sandbox.run(&["global", "system"]).succeeds();
    assert_eq!(
        std::fs::read_to_string(sandbox.root.join("version")).unwrap(),
        "system\n"
    );

    sandbox.run(&["local", "system"]).succeeds();
    assert_eq!(
        std::fs::read_to_string(sandbox.project().join(".java-version")).unwrap(),
        "system\n"
    );

    // And a name that is neither registered nor `system` is still refused.
    sandbox.run(&["global", "99"]).fails();
    sandbox.run(&["local", "99"]).fails();
}

#[test]
fn the_powershell_hook_and_shell_line_use_powershell_syntax() {
    let sandbox = Sandbox::new("powershell-dialect");
    write_script(&sandbox.jdk.join("bin").join("javac"), "echo javac");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    // PowerShell has no `export` and no `unset`, and this text is handed to
    // Invoke-Expression, so POSIX syntax throws on every single prompt.
    let hook = sandbox.spawn_text_with(&[("JENV_SHELL", "powershell")], &["export-hook"]);
    assert!(!hook.contains("export "), "{hook}");
    assert!(!hook.contains("unset "), "{hook}");
    assert!(hook.contains("$env:JAVA_HOME = "), "{hook}");

    let shell_line = sandbox.spawn_text_with(&[("JENV_SHELL", "powershell")], &["shell", "21"]);
    assert_eq!(shell_line.trim(), "$env:JENV_VERSION = \"21\"");

    // Clearing is a cmdlet in PowerShell, not a keyword. The hook only speaks
    // when the environment disagrees, so there has to be something to clear.
    sandbox.run(&["local", "--unset"]).succeeds();
    let cleared = sandbox.spawn_text_with(
        &[
            ("JENV_SHELL", "powershell"),
            ("JAVA_HOME", "/stale/jdk"),
            ("JDK_HOME", "/stale/jdk"),
        ],
        &["export-hook"],
    );
    assert!(cleared.contains("Remove-Item Env:JAVA_HOME"), "{cleared}");
    assert!(cleared.contains("Remove-Item Env:JDK_HOME"), "{cleared}");
}

#[test]
fn prefix_system_ignores_which_version_is_selected() {
    let sandbox = Sandbox::new("prefix-system");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    sandbox.install_system_java();

    // `system` asks what the JDK on PATH would be. It must not be answered out
    // of the selected version's bin/, which is what a plain `which java` does
    // while 21 is selected.
    sandbox
        .run(&["prefix", "system"])
        .succeeds()
        .stdout_is(sandbox.root.display().to_string().as_str());

    // And the selection is still what plain `prefix` reports.
    sandbox
        .run(&["prefix"])
        .succeeds()
        .stdout_is(sandbox.registered("21").display().to_string().as_str());
}

#[test]
fn the_powershell_init_names_the_binary_that_exists_on_this_platform() {
    let sandbox = Sandbox::new("powershell-exe");
    // `jenv init - powershell` also works where pwsh runs on macOS or Linux,
    // and there the installed binary is `jenv`, not `jenv.exe`.
    let code = sandbox.run(&["init", "-", "powershell"]).succeeds();
    let expected = format!("jenv{}", std::env::consts::EXE_SUFFIX);
    assert!(code.stdout.contains(&expected), "{}", code.stdout);
    if std::env::consts::EXE_SUFFIX.is_empty() {
        assert!(!code.stdout.contains("jenv.exe"), "{}", code.stdout);
    }
}

#[test]
fn shell_prints_the_line_the_parent_shell_has_to_eval() {
    let sandbox = Sandbox::new("shell");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();

    sandbox
        .run(&["shell", "21"])
        .succeeds()
        .stdout_is("export JENV_VERSION=\"21\"");
    sandbox
        .run(&["shell", "--unset"])
        .succeeds()
        .stdout_is("unset JENV_VERSION");
    sandbox
        .run(&["shell", "99"])
        .fails()
        .contains("not installed");
}

// --------------------------------------------------------------- rehash

#[test]
fn add_rehashes_so_the_new_binaries_are_immediately_callable() {
    let sandbox = Sandbox::new("rehash-on-add");
    write_script(&sandbox.jdk.join("bin").join("javac"), "echo javac");

    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    assert_eq!(
        sandbox.shim_names(),
        vec![binary_name("java"), binary_name("javac")]
    );
}

#[test]
fn every_shim_is_the_same_binary_rather_than_a_script() {
    let sandbox = Sandbox::new("rehash-hardlink");
    write_script(&sandbox.jdk.join("bin").join("javac"), "echo javac");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let template = std::fs::metadata(sandbox.shims().join("java")).unwrap();
        let javac = std::fs::metadata(sandbox.shims().join("javac")).unwrap();
        assert_eq!(
            template.ino(),
            javac.ino(),
            "shims should be hard links to one binary, not N copies"
        );
    }
}

#[test]
fn a_shim_only_the_removed_version_provided_disappears() {
    let sandbox = Sandbox::new("rehash-stale");
    let other = sandbox.dir.join("jdk-17");
    std::fs::create_dir_all(other.join("bin")).unwrap();
    write_script(&other.join("bin").join("java"), "echo \"java $1\"");
    write_script(
        &other.join("bin").join("jenvrs-only-in-17"),
        "echo seventeen",
    );
    std::fs::write(
        other.join("release"),
        "JAVA_VERSION=\"17.0.9\"\nIMPLEMENTOR=\"Zulu\"\nOS_ARCH=\"x86_64\"\n",
    )
    .unwrap();
    // A tool that only 21 has, so removing 21 must take its shim with it.
    write_script(
        &sandbox.jdk.join("bin").join("jenvrs-only-in-21"),
        "echo twentyone",
    );

    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox
        .run(&["add", &other.display().to_string()])
        .succeeds();
    assert!(
        sandbox
            .shim_names()
            .contains(&"jenvrs-only-in-21".to_string())
    );

    // `jenv add` registers four names for one JDK, and `remove` takes a single
    // name, so all four have to go before 21 stops being selectable.
    sandbox
        .run(&["remove", "21", "21.0", "21.0.2", "temurin64-21.0.2"])
        .succeeds();

    assert_eq!(
        sandbox.shim_names(),
        vec![binary_name("java"), "jenvrs-only-in-17".to_string(),]
    );
}

#[test]
fn rehash_refuses_to_run_twice_and_force_overrides_the_lock() {
    let sandbox = Sandbox::new("rehash-lock");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();

    std::fs::write(sandbox.shims().join(".jenv-shim-lock"), "").unwrap();
    sandbox
        .run(&["rehash"])
        .fails()
        .contains("try 'jenv rehash --force' to override");

    sandbox.run(&["rehash", "--force"]).succeeds();

    // `--force` is advertised as the way out of a poisoned lock, so it has to
    // actually leave no lock behind. If it only overrode without clearing, the
    // next plain rehash — and every `jenv add` after it — would keep failing.
    assert!(!sandbox.shims().join(".jenv-shim-lock").exists());
    sandbox.run(&["rehash"]).succeeds();
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
}

#[test]
fn remove_refuses_a_name_that_would_leave_the_versions_directory() {
    let sandbox = Sandbox::new("remove-traversal");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();

    // `Path::join("")` is the directory itself and `join("../x")` escapes it,
    // so an unchecked name reaches the delete. `jenv remove "$V"` with `V`
    // unset is the ordinary way to get here.
    for name in ["", ".", "..", "../escape", "sub/dir"] {
        sandbox.run(&["remove", name]).fails();
    }

    let survivors: Vec<String> = std::fs::read_dir(sandbox.root.join("versions"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(survivors.contains(&"21".to_string()), "{survivors:?}");
    assert!(sandbox.root.join("versions").is_dir());
    assert!(!sandbox.dir.join("escape").exists());
}

#[test]
fn a_shim_generation_bump_rewrites_every_shim() {
    let sandbox = Sandbox::new("rehash-version");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();

    // Stand in for an upgrade of the shim binary.
    std::fs::write(sandbox.shims().join(".jenv-shim-version"), "0").unwrap();
    write_script(&sandbox.jdk.join("bin").join("jshell"), "echo jshell");
    std::fs::write(sandbox.shims().join("jshell"), "stale script").unwrap();

    sandbox.run(&["rehash"]).succeeds();
    let shim = sandbox.shims().join(binary_name("jshell"));
    assert!(shim.exists());
    assert_ne!(std::fs::read(&shim).unwrap(), b"stale script");
}

// ----------------------------------------------------------------- exec

#[test]
fn exec_gives_the_tool_its_java_home_and_version_bin() {
    let sandbox = Sandbox::new("exec-env");
    write_script(
        &sandbox.jdk.join("bin").join("java"),
        "echo \"JAVA_HOME=$JAVA_HOME\"\necho \"FIRST=$(echo \"$PATH\" | cut -d: -f1)\"\necho \"JENV_VERSION=$JENV_VERSION\"",
    );
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    let run = sandbox.run(&["exec", "java", "-version"]).succeeds();
    assert!(
        run.stdout
            .contains(&format!("JAVA_HOME={}", sandbox.jdk.display()))
    );
    assert!(
        run.stdout
            .contains(&format!("FIRST={}", sandbox.jdk.join("bin").display()))
    );
    assert!(run.stdout.contains("JENV_VERSION=21"));
}

#[test]
fn exec_prepends_options_to_the_tools_arguments() {
    let sandbox = Sandbox::new("exec-options");
    write_script(&sandbox.jdk.join("bin").join("java"), "echo \"$*\"");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();
    std::fs::write(sandbox.project().join(".java-options"), "-Xmx1g -Dx=a b").unwrap();

    sandbox
        .run(&["exec", "java", "App"])
        .succeeds()
        .stdout_is("-Xmx1g -Dx=a b App");
}

#[test]
fn exec_reports_a_command_that_no_version_provides() {
    let sandbox = Sandbox::new("exec-missing");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    sandbox
        .run(&["exec", "definitely-not-a-tool"])
        .fails()
        .contains("command not found");
}

#[test]
fn which_names_the_active_versions_binary() {
    let sandbox = Sandbox::new("which");
    write_script(&sandbox.jdk.join("bin").join("javac"), "echo javac");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    // Like `jenv which` in the shell version, this names the path under
    // `versions/` that will be executed, not the JDK home behind the symlink.
    sandbox.run(&["which", "javac"]).succeeds().stdout_is(
        sandbox
            .registered("21")
            .join("bin")
            .join("javac")
            .display()
            .to_string()
            .as_str(),
    );

    // `whence` reports every registered name that provides the command, which
    // for a JDK added by `jenv add` is all four of its aliases.
    let whence = sandbox.run(&["whence", "javac"]).succeeds();
    assert_eq!(
        whence.lines(),
        vec!["21", "21.0", "21.0.2", "temurin64-21.0.2"]
    );
}

#[test]
fn which_says_which_other_versions_would_provide_a_missing_command() {
    let sandbox = Sandbox::new("which-hint");
    let other = sandbox.dir.join("jdk-17");
    std::fs::create_dir_all(other.join("bin")).unwrap();
    write_script(&other.join("bin").join("java"), "echo \"java $1\"");
    write_script(
        &other.join("bin").join("jenvrs-only-in-17"),
        "echo seventeen",
    );
    std::fs::write(
        other.join("release"),
        "JAVA_VERSION=\"17.0.9\"\nIMPLEMENTOR=\"Zulu\"\nOS_ARCH=\"x86_64\"\n",
    )
    .unwrap();

    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox
        .run(&["add", &other.display().to_string()])
        .succeeds();
    // Select the JDK that lacks the tool, so the lookup has to fail.
    sandbox.run(&["local", "21"]).succeeds();

    let run = sandbox.run(&["which", "jenvrs-only-in-17"]).fails();
    assert!(run.stderr.contains("command not found"), "{}", run.stderr);
    assert!(
        run.stderr.contains("exists in these Java versions"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("17.0.9"), "{}", run.stderr);
}

// -------------------------------------------------------------- export

#[test]
fn javahome_points_at_the_version_and_fails_for_system() {
    let sandbox = Sandbox::new("javahome");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    // The registration path, which is what the export plugin writes into
    // JAVA_HOME; a value that survives the JDK moving is the point of it.
    sandbox
        .run(&["javahome"])
        .succeeds()
        .stdout_is(sandbox.registered("21").display().to_string().as_str());

    sandbox.run(&["local", "--unset"]).succeeds();
    sandbox
        .run(&["javahome"])
        .fails()
        .contains("no JAVA_HOME set");
}

#[test]
fn the_export_hook_publishes_java_home_and_jdk_home_for_a_jdk() {
    let sandbox = Sandbox::new("export-hook");
    write_script(&sandbox.jdk.join("bin").join("javac"), "echo javac");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    let hook = sandbox.spawn_text(&["export-hook"]);
    let home = sandbox.registered("21").display().to_string();
    assert!(
        hook.contains(&format!("export JAVA_HOME=\"{home}\"")),
        "{hook}"
    );
    assert!(
        hook.contains("export JENV_FORCEJAVAHOME=\"true\""),
        "{hook}"
    );
    // javac is present, so this is a JDK and JDK_HOME is published too.
    assert!(hook.contains("export JDK_HOME="), "{hook}");
    assert!(hook.contains("export JENV_FORCEJDKHOME=\"true\""), "{hook}");
}

#[test]
fn the_export_hook_clears_java_home_for_a_jre_and_for_system() {
    let sandbox = Sandbox::new("export-hook-jre");
    // No javac, so this is a JRE and JDK_HOME has nothing to point at.
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    let stale = sandbox.spawn_text_with(
        &[
            ("JAVA_HOME", "/stale/jdk"),
            ("JENV_FORCEJAVAHOME", "true"),
            ("JDK_HOME", "/stale/jdk"),
            ("JENV_FORCEJDKHOME", "true"),
        ],
        &["export-hook"],
    );
    assert!(
        stale.contains(&format!(
            "export JAVA_HOME=\"{}\"",
            sandbox.registered("21").display()
        )),
        "{stale}"
    );
    assert!(stale.contains("unset JDK_HOME"), "{stale}");
    assert!(stale.contains("unset JENV_FORCEJDKHOME"), "{stale}");

    // With no version configured the system java is in play, and the only
    // defensible answer is to clear the variables rather than guess.
    sandbox.run(&["local", "--unset"]).succeeds();
    sandbox.run(&["global", "--unset"]).succeeds();
    let system = sandbox.spawn_text_with(
        &[("JAVA_HOME", "/stale/jdk"), ("JDK_HOME", "/stale/jdk")],
        &["export-hook"],
    );
    assert!(system.contains("unset JAVA_HOME"), "{system}");
    assert!(system.contains("unset JDK_HOME"), "{system}");
    assert!(!system.contains("export JAVA_HOME"), "{system}");
}

#[test]
fn the_export_hook_repoints_jdk_home_when_the_version_moves() {
    let sandbox = Sandbox::new("export-hook-repoint");
    write_script(&sandbox.jdk.join("bin").join("javac"), "echo javac");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    let hook = sandbox.spawn_text_with(
        &[("JAVA_HOME", "/stale/jdk"), ("JDK_HOME", "/stale/jdk")],
        &["export-hook"],
    );
    let home = sandbox.registered("21").display().to_string();
    assert!(
        hook.contains(&format!("export JDK_HOME=\"{home}\"")),
        "{hook}"
    );
}

#[test]
fn the_export_hook_is_silent_when_the_environment_already_agrees() {
    let sandbox = Sandbox::new("export-hook-noop");
    write_script(&sandbox.jdk.join("bin").join("javac"), "echo javac");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    let home = sandbox.registered("21").display().to_string();
    let already_set = sandbox.spawn_text_with(
        &[
            ("JAVA_HOME", &home),
            ("JENV_FORCEJAVAHOME", "true"),
            ("JDK_HOME", &home),
            ("JENV_FORCEJDKHOME", "true"),
        ],
        &["export-hook"],
    );

    assert_eq!(
        already_set.trim(),
        "",
        "the hook must print nothing when nothing changed"
    );
}

#[test]
fn the_install_location_does_not_matter_as_long_as_it_is_on_path() {
    // `$JENV_ROOT/libexec` is a convention from the bash jenv, not a
    // requirement. These binaries are reachable only through PATH, and the
    // sandbox clears the environment so nothing else can satisfy the lookup.
    let sandbox = Sandbox::new("anywhere");
    let elsewhere = sandbox.dir.join("opt").join("somewhere").join("else");
    std::fs::create_dir_all(&elsewhere).unwrap();
    copy(JENV, &elsewhere.join(binary_name("jenv")));

    // The sandbox's own copy stays put; PATH is what the child sees. Built by
    // hand rather than through the sandbox, because the point of the test is a
    // different install location, so it has to reach the retry itself.
    let mut command =
        std::process::Command::new(sandbox.root.join("bin").join(binary_name("jenv")));
    command
        .args(["add", &sandbox.jdk.display().to_string()])
        .current_dir(sandbox.project())
        .env_clear()
        .env(
            "PATH",
            format!(
                "{}{}{}",
                elsewhere.display(),
                jenv::proc::SEPARATOR,
                env_path()
            ),
        )
        .env("JENV_ROOT", &sandbox.root)
        .env("JENV_SHELL", "bash")
        .env("JENV_DIR", sandbox.project());
    let run = output_of(command).unwrap();
    assert!(
        run.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    // rehash links the running binary, wherever the user put it.
    let shim = sandbox.shims().join(binary_name("java"));
    assert!(shim.is_file(), "rehash did not find the shim binary");
}

#[test]
fn a_shim_only_the_selected_jdk_provides_runs_through() {
    let sandbox = Sandbox::new("no-hooks");
    // The whole point of removing the hook layer: a JDK's binaries are
    // dispatched with no shell and no plugin scripts anywhere.
    write_script(
        &sandbox.jdk.join("bin").join("jenvrs-only-here"),
        "echo ran",
    );
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    let output = sandbox.spawn(&sandbox.shims().join(binary_name("jenvrs-only-here")), &[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ran");
}

// ----------------------------------------------------------------- init

#[test]
fn init_emits_a_working_setup_for_every_supported_shell() {
    let sandbox = Sandbox::new("init");

    let bash = sandbox.run(&["init", "-", "bash"]).succeeds();
    assert!(bash.stdout.contains("export PATH="));
    assert!(bash.stdout.contains("jenv() {"));
    // The export hook is what keeps JAVA_HOME following the selected version.
    assert!(bash.stdout.contains("jenv export-hook"), "{}", bash.stdout);
    assert!(!bash.stdout.contains("unset JAVA_HOME"), "{}", bash.stdout);

    let plain = sandbox
        .run(&["init", "-", "--no-export", "bash"])
        .succeeds();
    assert!(plain.stdout.contains("unset JAVA_HOME"), "{}", plain.stdout);
    assert!(
        !plain.stdout.contains("jenv export-hook"),
        "{}",
        plain.stdout
    );

    let zsh = sandbox.run(&["init", "-", "zsh"]).succeeds();
    assert!(
        zsh.stdout.contains(r#"export JENV_SHELL="zsh""#),
        "{}",
        zsh.stdout
    );
    assert!(zsh.stdout.contains("#compdef jenv"), "{}", zsh.stdout);

    let fish = sandbox.run(&["init", "-", "fish"]).succeeds();
    assert!(fish.stdout.contains("set -gx PATH"));
    assert!(fish.stdout.contains("complete -c jenv"));

    let powershell = sandbox.run(&["init", "-", "powershell"]).succeeds();
    assert!(powershell.stdout.contains("$env:PATH"));
    assert!(powershell.stdout.contains("function global:jenv"));
    assert!(powershell.stdout.contains("Register-ArgumentCompleter"));
}

#[test]
fn init_inlines_the_completion_script_rather_than_sourcing_a_pipe() {
    let sandbox = Sandbox::new("init-completions");
    let run = sandbox.run(&["init", "-", "bash"]).succeeds();

    // `source <(...)` reads nothing under bash, so the script has to be part of
    // what init prints.
    assert!(!run.stdout.contains("source <("), "{}", run.stdout);
    assert!(
        run.stdout
            .contains("complete -F _jenv_dynamic -o default jenv"),
        "{}",
        run.stdout
    );
}

#[test]
fn the_bash_completion_keeps_the_static_answers_alongside_the_dynamic_ones() {
    // `complete -F` replaces a registration rather than adding to it, so a
    // dynamic wrapper that never calls clap's function silently deletes every
    // subcommand name. `jenv <TAB>` coming back empty is the symptom.
    let sandbox = Sandbox::new("completion-chaining");
    let script = sandbox.spawn_text(&["init", "-", "--no-rehash", "--no-export", "bash"]);

    assert!(
        script.contains("_jenv_dynamic() {\n    _jenv \"$@\""),
        "the dynamic wrapper does not chain to the static one:\n{script}"
    );

    // And the static function it chains to is the one that knows the
    // subcommands, which is the part that was being lost.
    assert!(script.contains("jenv__subcmd__add"), "{script}");
}

#[test]
fn init_without_the_dash_explains_what_to_add_to_a_profile() {
    let sandbox = Sandbox::new("init-instructions");
    let run = sandbox.run(&["init", "bash"]).fails();
    assert!(run.stderr.contains("~/.bash_profile"), "{}", run.stderr);
    assert!(
        run.stderr.contains(r#"eval "$(jenv init -)""#),
        "{}",
        run.stderr
    );
}

#[test]
fn init_creates_the_directories_the_shell_setup_promises() {
    let sandbox = Sandbox::new("init-dirs");
    sandbox.run(&["init", "-", "bash"]).succeeds();
    assert!(sandbox.root.join("shims").is_dir());
    assert!(sandbox.root.join("versions").is_dir());
}

// -------------------------------------------------------------- shim run

#[test]
fn the_shim_binary_dispatches_on_its_own_file_name() {
    let sandbox = Sandbox::new("shim-run");
    write_script(&sandbox.jdk.join("bin").join("java"), "echo \"java $1\"");
    write_script(&sandbox.jdk.join("bin").join("javac"), "echo \"javac $1\"");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    for name in ["java", "javac"] {
        let output = sandbox.spawn(&sandbox.shims().join(binary_name(name)), &["-version"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!("{name} -version")
        );
    }
}

#[test]
fn the_shim_and_the_cli_are_the_same_file() {
    let sandbox = Sandbox::new("one-file");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();
    sandbox.run(&["local", "21"]).succeeds();

    let installed = sandbox.root.join("bin").join(binary_name("jenv"));
    let shim = sandbox.shims().join(binary_name("java"));
    assert!(shim.is_file(), "rehash produced no shim");

    // Same inode, not a copy: a shim can never be a different build from the
    // jenv that manages it, and a hundred shims cost one file's disk.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            std::fs::metadata(&installed).unwrap().ino(),
            std::fs::metadata(&shim).unwrap().ino(),
            "the shim is not a hard link to the jenv binary"
        );
    }

    // The same bytes answer to both questions.
    let as_cli = sandbox.spawn(&installed, &["version-name"]);
    assert!(as_cli.status.success());

    let as_shim = sandbox.spawn(&shim, &["-version"]);
    assert_eq!(
        String::from_utf8_lossy(&as_shim.stdout).trim(),
        "java -version"
    );
}

#[test]
fn a_shim_named_jenv_is_still_the_command_line() {
    // `rehash` never creates this, but a user who hard-links the binary by
    // hand should get the CLI rather than a shim that tries to run a JDK
    // binary called `jenv`.
    let sandbox = Sandbox::new("jenv-named-shim");
    let shim = sandbox.shims().join(binary_name("jenv"));
    std::fs::create_dir_all(sandbox.shims()).unwrap();
    copy(JENV, &shim);

    let output = sandbox.spawn(&shim, &["version-name"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_closed_pipe_ends_the_command_quietly() {
    let sandbox = Sandbox::new("sigpipe");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();

    // `jenv versions | head -1` must not panic; every other Unix tool just
    // stops writing.
    // The busy binary is exec'd by the shell, not by this process, so there is
    // no spawn to retry here: the whole pipeline is re-run instead, and a real
    // regression simply fails every attempt.
    let mut out = None;
    for _ in 0..40 {
        let child = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "{} versions | head -1 && {} completions bash | head -c 20 >/dev/null && echo SURVIVED",
                sandbox.root.join("bin").join(binary_name("jenv")).display(),
                sandbox.root.join("bin").join(binary_name("jenv")).display(),
            ))
            .current_dir(sandbox.project())
            .env_clear()
            .env("PATH", env_path())
            .env("JENV_ROOT", &sandbox.root)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let attempt = child.wait_with_output().unwrap();
        if attempt.status.success() && String::from_utf8_lossy(&attempt.stdout).contains("SURVIVED")
        {
            out = Some(attempt);
            break;
        }
        back_off();
    }
    let out = out.expect("the pipeline must succeed within the retry budget");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("SURVIVED"), "{stdout}");
    assert!(!stdout.contains("panicked"), "{stdout}");
}

// ------------------------------------------------------------ completion

#[test]
fn the_generated_completion_script_covers_every_supported_shell() {
    let sandbox = Sandbox::new("completions");

    for (shell, marker) in [
        ("bash", "complete -F _jenv_dynamic -o default jenv"),
        ("zsh", "#compdef jenv"),
        ("fish", "complete -c jenv"),
        ("powershell", "Register-ArgumentCompleter"),
    ] {
        let script = sandbox
            .run(&["completions", shell])
            .succeeds()
            .stdout
            .clone();
        assert!(script.contains(marker), "{shell} script lacks {marker:?}");
    }
}

#[test]
fn the_generated_script_offers_the_installed_versions_and_shims() {
    let sandbox = Sandbox::new("completions-dynamic");
    sandbox
        .run(&["add", &sandbox.jdk.display().to_string()])
        .succeeds();

    // The static script cannot know these, so it has to shell back out.
    let script = sandbox
        .run(&["completions", "bash"])
        .succeeds()
        .stdout
        .clone();
    assert!(script.contains("jenv complete versions"), "{script}");
    assert!(script.contains("jenv complete shims"), "{script}");

    let versions = sandbox.run(&["complete", "versions"]).succeeds();
    assert!(versions.lines().contains(&"21"), "{:?}", versions.lines());
    assert!(
        versions.lines().contains(&"system"),
        "{:?}",
        versions.lines()
    );

    let shims = sandbox.run(&["complete", "shims"]).succeeds();
    assert!(shims.lines().contains(&"java"), "{:?}", shims.lines());
}

#[test]
fn the_generated_script_tracks_new_subcommands() {
    let sandbox = Sandbox::new("completions-drift");
    let script = sandbox
        .run(&["completions", "fish"])
        .succeeds()
        .stdout
        .clone();

    // `doctor` was added after the completion script first shipped; a
    // hand-maintained list is exactly what would have missed it.
    assert!(script.contains("doctor"), "doctor missing from:\n{script}");

    let commands = sandbox.run(&["commands"]).succeeds();
    assert!(commands.lines().contains(&"doctor"));
    assert!(commands.lines().contains(&"completions"));
}
