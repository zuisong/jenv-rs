//! Working out what a JDK directory actually is.
//!
//! `jenv add` shells out to `java -version` and greps the banner. That is
//! unreliable on Windows (the banner is localized, and `java.exe` may be
//! absent from a JRE-only layout) and slow. A JDK ships a `release` file next
//! to `bin/` that states the same facts as plain `KEY="value"` lines, so we
//! read that first and only spawn `java` when it is missing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jdk {
    pub home: PathBuf,
    /// Normalized provider slug, e.g. `temurin`.
    pub provider: String,
    /// Full version with `_` turned into `.`, e.g. `21.0.2.13`.
    pub version: String,
    /// `64` or `32`, matching jenv's naming.
    pub platform: String,
}

impl Jdk {
    /// The canonical name `jenv add` registers, e.g. `temurin64-21.0.2.13`.
    pub fn alias(&self) -> String {
        format!("{}{}-{}", self.provider, self.platform, self.version)
    }

    /// Every name this JDK answers to, most specific first. jenv registers all
    /// of them so that `21`, `21.0` and the full string all select it.
    pub fn aliases(&self) -> Vec<String> {
        let mut names = vec![self.alias(), self.version.clone()];

        if let Some(short) = self.short_version() {
            names.push(short);
        }
        // A `1.x` JDK's major component is `1`, which is not a useful alias.
        if let Some(major) = self.major_version()
            && major != "1"
            && Some(major.as_str()) != self.short_version().as_deref()
        {
            names.push(major);
        }

        let mut seen = std::collections::HashSet::new();
        names.retain(|n| seen.insert(n.clone()));
        names
    }

    /// `21.0` for `21.0.2`, or `1.8` for `1.8.0_292`.
    pub fn short_version(&self) -> Option<String> {
        let mut parts = self.version.split('.');
        let major = parts.next()?;
        let minor = parts.next()?;
        Some(format!("{major}.{minor}"))
    }

    pub fn major_version(&self) -> Option<String> {
        self.version.split('.').next().map(str::to_string)
    }
}

/// Is `home` a usable JDK home?
pub fn java_binary(home: &Path) -> Option<PathBuf> {
    let base = home.join("bin").join("java");
    [base.with_extension("exe"), base]
        .into_iter()
        .find(|candidate| candidate.is_file())
}

pub fn probe(home: &Path) -> Result<Jdk, String> {
    let home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());

    // A `release` file describes a JDK, it does not make one. Without a
    // `bin/java` there is nothing to run, and registering it would produce a
    // version that resolves for `jenv local` and then fails on every use.
    let java = java_binary(&home).ok_or_else(|| {
        format!(
            "{} is not a valid path to java installation",
            home.display()
        )
    })?;

    if let Some(meta) = read_release(&home)
        && let Some(jdk) = from_release(&home, &meta)
    {
        return Ok(jdk);
    }

    let output = std::process::Command::new(&java)
        .arg("-version")
        .output()
        .map_err(|e| format!("failed to run {}: {e}", java.display()))?;
    // `java -version` writes to stderr, which is not a bug on any JDK.
    let banner = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let version = banner
        .lines()
        .find(|l| l.contains("version"))
        .and_then(|l| l.split('"').nth(1).map(|v| v.replace('_', ".")))
        .ok_or_else(|| format!("could not parse a version out of `{}`", banner.trim()))?;

    Ok(Jdk {
        provider: provider_from_banner(&banner),
        platform: platform_from_banner(&banner),
        version,
        home,
    })
}

/// Parse a JDK's `release` file into its `KEY="value"` pairs.
pub fn read_release(home: &Path) -> Option<BTreeMap<String, String>> {
    let raw = std::fs::read_to_string(home.join("release")).ok()?;
    let mut map = BTreeMap::new();
    for line in raw.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        map.insert(
            key.trim().to_string(),
            value.trim().trim_matches('"').to_string(),
        );
    }
    (!map.is_empty()).then_some(map)
}

fn from_release(home: &Path, meta: &BTreeMap<String, String>) -> Option<Jdk> {
    let version = meta.get("JAVA_VERSION")?.replace('_', ".");
    // `IMPLEMENTOR_VERSION` carries the full banner text on most vendors, so it
    // is a better provider signal than `IMPLEMENTOR` alone.
    let banner = meta
        .get("IMPLEMENTOR_VERSION")
        .cloned()
        .unwrap_or_else(|| meta.get("IMPLEMENTOR").cloned().unwrap_or_default());

    Some(Jdk {
        provider: provider_from_banner(&format!(
            "{} {}",
            meta.get("IMPLEMENTOR")
                .map(String::as_str)
                .unwrap_or_default(),
            banner
        )),
        platform: platform_from_arch(meta.get("OS_ARCH").map(String::as_str).unwrap_or_default()),
        version,
        home: home.to_path_buf(),
    })
}

/// jenv's banner matching table, extended with the `release`-file spellings.
pub fn provider_from_banner(banner: &str) -> String {
    let lower = banner.to_ascii_lowercase();
    let has = |needle: &str| lower.contains(needle);

    if has("graalvm") {
        "graalvm"
    } else if has("temurin") || has("adoptium") {
        "temurin"
    } else if has("corretto") || has("amazon") {
        "corretto"
    } else if has("zulu") || has("azul") {
        "zulu"
    } else if has("zing") {
        "zulu_prime"
    } else if has("sapmachine") || has("sap machine") || has("sap se") {
        "sap"
    } else if has("jetbrains") || has("jbr") {
        "jetbrains"
    } else if has("kona") {
        "kona"
    } else if has("openlogic") {
        "openlogic"
    } else if has("dragonwell") || has("alibaba") {
        "dragonwell"
    } else if has("semeru runtime open") {
        "semeru"
    } else if has("semeru runtime certified") {
        "semeru_certified"
    } else if has("liberica") || has("bellsoft") {
        "bellsoft"
    } else if has("j9") || has("ibm") {
        "ibm"
    } else if has("hotspot") || has("oracle") {
        "oracle"
    } else if has("openjdk") {
        "openjdk"
    } else {
        "other"
    }
    .to_string()
}

pub fn platform_from_banner(banner: &str) -> String {
    if banner.contains("64-Bit") || banner.contains("64 bit") {
        "64".into()
    } else {
        "32".into()
    }
}

/// `java -version` only ever says "64-Bit", so a JDK on aarch64 Linux can be
/// mislabelled 32-bit by the shell implementation. `OS_ARCH` is exact.
///
/// The 32-bit names are enumerated; anything else is taken as 64-bit. Reading
/// an unknown architecture as 32-bit is the worse error — it would register
/// `ppc64le`, `s390x` or `riscv64` under a `…32-…` name, and no amount of
/// `jenv versions` reading would make that look right.
pub fn platform_from_arch(arch: &str) -> String {
    match arch.trim().to_ascii_lowercase().as_str() {
        "x86" | "i386" | "i586" | "i686" | "i86pc" | "x86-32" => "32".into(),
        _ => "64".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_match_jenvs_table() {
        assert_eq!(
            provider_from_banner(
                "openjdk version \"21\" 2023-09-19\nOpenJDK Runtime Environment (build 21+35)"
            ),
            "openjdk"
        );
        assert_eq!(provider_from_banner("Eclipse Temurin-21.0.2+13"), "temurin");
        assert_eq!(provider_from_banner("Zulu17.46.19-CA"), "zulu");
        assert_eq!(provider_from_banner("Amazon Corretto 17"), "corretto");
        assert_eq!(
            provider_from_banner("openjdk version \"1.8.0_292\""),
            "openjdk"
        );
    }

    #[test]
    fn arch_detection_handles_aarch64() {
        assert_eq!(platform_from_arch("aarch64"), "64");
        assert_eq!(platform_from_arch("x86_64"), "64");
        assert_eq!(platform_from_arch("x86"), "32");
    }

    #[test]
    fn aliases_skip_the_bare_major_for_legacy_jdks() {
        let jdk = Jdk {
            home: PathBuf::from("/jdk"),
            provider: "openjdk".into(),
            platform: "64".into(),
            version: "1.8.0.292".into(),
        };
        assert_eq!(jdk.alias(), "openjdk64-1.8.0.292");
        assert_eq!(
            jdk.aliases(),
            vec!["openjdk64-1.8.0.292", "1.8.0.292", "1.8"]
        );

        let jdk = Jdk {
            version: "21.0.2.13".into(),
            ..jdk
        };
        assert_eq!(
            jdk.aliases(),
            vec!["openjdk64-21.0.2.13", "21.0.2.13", "21.0", "21"]
        );
    }

    #[test]
    fn release_file_is_parsed() {
        let dir = std::env::temp_dir().join(format!("jenv-rs-rel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("release"),
            "JAVA_VERSION=\"21.0.2\"\nIMPLEMENTOR=\"Eclipse Adoptium\"\nOS_ARCH=\"aarch64\"\n",
        )
        .unwrap();

        let meta = read_release(&dir).unwrap();
        assert_eq!(meta.get("JAVA_VERSION").unwrap(), "21.0.2");

        let jdk = from_release(&dir, &meta).unwrap();
        assert_eq!(jdk.provider, "temurin");
        assert_eq!(jdk.platform, "64");
        assert_eq!(jdk.version, "21.0.2");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
