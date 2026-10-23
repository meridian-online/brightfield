//! The test executables one CI job builds and the shards run.
//!
//! `test.yml`'s `build-tests` job compiles the suite once, with
//! `cargo test --no-run`, and reads cargo's JSON messages into a manifest:
//! each test executable with the package and target it belongs to, and the
//! workspace binaries the integration tests reach through `CARGO_BIN_EXE_*`.
//! The manifest and the files it names travel to the shards in one archive.
//! A shard then runs each executable as `cargo test` would have, from its
//! package's root with cargo's package variables set, instead of compiling
//! the workspace again.
//!
//! Every path in the manifest is relative to the repository root, and every
//! hosted runner checks the repository out at the same place, so the
//! absolute paths a test binary had baked into it at compile time
//! (`CARGO_BIN_EXE_*`, `CARGO_TARGET_TMPDIR`) hold on the shard too.

use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

/// Which of a package's targets a test executable was built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// The library's unit tests, selected by `cargo test --lib`.
    Lib,
    /// A binary's unit tests, selected by `cargo test --bins`.
    Bin,
    /// An integration test target, selected by `cargo test --test <name>`.
    Test,
}

/// One test executable.
#[derive(Clone, Debug, PartialEq)]
pub struct Executable {
    pub package: String,
    pub version: String,
    pub kind: Kind,
    /// The target's name, as cargo's `--test` takes it.
    pub name: String,
    /// The executable, from the repository root.
    pub path: String,
    /// The package's directory, from the repository root.
    pub package_dir: String,
}

/// What the build job hands to the shards.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub executables: Vec<Executable>,
    /// The workspace's non-test binaries, from the repository root.
    pub bins: Vec<String>,
}

impl Manifest {
    /// Read cargo's `--message-format=json` output, one message a line. A
    /// test-profile artifact with an executable is a test executable; a
    /// non-test `bin` artifact is a binary the tests may spawn. Every other
    /// message is passed over.
    pub fn from_cargo_messages(json_lines: &str, root: &Path) -> Result<Manifest, String> {
        let mut executables = Vec::new();
        let mut bins = Vec::new();
        for line in json_lines.lines().filter(|l| l.starts_with('{')) {
            let message: Value = serde_json::from_str(line)
                .map_err(|e| format!("cargo printed a message that is not JSON ({e}): {line}"))?;
            if message["reason"] != "compiler-artifact" {
                continue;
            }
            let Some(executable) = message["executable"].as_str() else {
                continue;
            };
            let first_kind = message["target"]["kind"][0].as_str().unwrap_or_default();
            let path = relative(root, executable)?;
            if message["profile"]["test"] != true {
                if first_kind == "bin" {
                    bins.push(path);
                }
                continue;
            }
            let kind = match first_kind {
                "test" => Kind::Test,
                "bin" => Kind::Bin,
                "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro" => Kind::Lib,
                _ => continue,
            };
            let package_id = message["package_id"].as_str().unwrap_or_default();
            let (package, version) = package_and_version(package_id).ok_or_else(|| {
                format!("cannot read a package and version from the package id '{package_id}'")
            })?;
            let manifest_path = message["manifest_path"].as_str().unwrap_or_default();
            let package_dir = Path::new(manifest_path)
                .parent()
                .ok_or_else(|| format!("the manifest path '{manifest_path}' has no directory"))?;
            executables.push(Executable {
                package,
                version,
                kind,
                name: message["target"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                path,
                package_dir: relative(root, &package_dir.to_string_lossy())?,
            });
        }
        bins.sort();
        bins.dedup();
        Ok(Manifest { executables, bins })
    }

    pub fn to_json(&self) -> String {
        let executables: Vec<Value> = self
            .executables
            .iter()
            .map(|e| {
                json!({
                    "package": e.package,
                    "version": e.version,
                    "kind": kind_word(e.kind),
                    "name": e.name,
                    "path": e.path,
                    "package_dir": e.package_dir,
                })
            })
            .collect();
        let value = json!({"executables": executables, "bins": self.bins});
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Manifest, String> {
        let value: Value = serde_json::from_str(text)
            .map_err(|e| format!("the test manifest is not JSON: {e}"))?;
        let field = |entry: &Value, key: &str| -> Result<String, String> {
            entry[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("a test manifest entry has no '{key}': {entry}"))
        };
        let mut executables = Vec::new();
        for entry in value["executables"]
            .as_array()
            .ok_or("the test manifest has no executables list")?
        {
            let kind = match field(entry, "kind")?.as_str() {
                "lib" => Kind::Lib,
                "bin" => Kind::Bin,
                "test" => Kind::Test,
                other => {
                    return Err(format!(
                        "a test manifest entry has the unknown kind '{other}'"
                    ))
                }
            };
            executables.push(Executable {
                package: field(entry, "package")?,
                version: field(entry, "version")?,
                kind,
                name: field(entry, "name")?,
                path: field(entry, "path")?,
                package_dir: field(entry, "package_dir")?,
            });
        }
        let bins = value["bins"]
            .as_array()
            .ok_or("the test manifest has no bins list")?
            .iter()
            .map(|b| {
                b.as_str()
                    .map(str::to_owned)
                    .ok_or("a test manifest bin is not a string")
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Manifest { executables, bins })
    }

    /// The executables a shard's items name, in a stable order: `@lib` is
    /// every library's unit tests, `@bins` every binary's, and any other name
    /// the integration target of that name. `@doc` and `@protocols` are run
    /// elsewhere and select nothing. A name nothing was built for is refused,
    /// so a target added after the archive was built cannot pass by running
    /// nowhere.
    pub fn for_items(&self, items: &[String]) -> Result<Vec<&Executable>, String> {
        let mut chosen = Vec::new();
        for item in items {
            let found: Vec<&Executable> = match item.as_str() {
                "@doc" | "@protocols" => continue,
                "@lib" => self
                    .executables
                    .iter()
                    .filter(|e| e.kind == Kind::Lib)
                    .collect(),
                "@bins" => self
                    .executables
                    .iter()
                    .filter(|e| e.kind == Kind::Bin)
                    .collect(),
                name => self
                    .executables
                    .iter()
                    .filter(|e| e.kind == Kind::Test && e.name == name)
                    .collect(),
            };
            if found.is_empty() && !item.starts_with('@') {
                return Err(format!(
                    "the test archive holds no executable for '{item}': the build job built a different set of targets than this shard was given"
                ));
            }
            chosen.extend(found);
        }
        chosen.sort_by(|a, b| {
            (&a.package, kind_word(a.kind), &a.name).cmp(&(&b.package, kind_word(b.kind), &b.name))
        });
        chosen.dedup_by(|a, b| a.path == b.path);
        Ok(chosen)
    }

    /// The command that runs `exe` as `cargo test` would: from its package's
    /// directory, with the variables cargo sets for a test process.
    pub fn command(&self, exe: &Executable, root: &Path) -> Command {
        let package_dir = root.join(&exe.package_dir);
        let mut command = Command::new(root.join(&exe.path));
        command.current_dir(&package_dir);
        let core = exe.version.split('-').next().unwrap_or_default();
        let mut numbers = core.split('.');
        let pre = exe
            .version
            .split_once('-')
            .map(|(_, pre)| pre)
            .unwrap_or_default();
        command
            .env("CARGO_MANIFEST_DIR", &package_dir)
            .env("CARGO_MANIFEST_PATH", package_dir.join("Cargo.toml"))
            .env("CARGO_PKG_NAME", &exe.package)
            .env("CARGO_PKG_VERSION", &exe.version)
            .env(
                "CARGO_PKG_VERSION_MAJOR",
                numbers.next().unwrap_or_default(),
            )
            .env(
                "CARGO_PKG_VERSION_MINOR",
                numbers.next().unwrap_or_default(),
            )
            .env(
                "CARGO_PKG_VERSION_PATCH",
                numbers.next().unwrap_or_default(),
            )
            .env("CARGO_PKG_VERSION_PRE", pre)
            .env("CARGO_CRATE_NAME", exe.name.replace('-', "_"));
        command
    }
}

fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Lib => "lib",
        Kind::Bin => "bin",
        Kind::Test => "test",
    }
}

/// `path` from the repository root; a path outside it is refused, since the
/// shard would have nowhere to put it.
fn relative(root: &Path, path: &str) -> Result<String, String> {
    Path::new(path)
        .strip_prefix(root)
        .map(|rel| rel.to_string_lossy().into_owned())
        .map_err(|_| {
            format!(
                "{path} is outside the repository at {}: the archive keeps paths from the root, so the target directory has to be the root's target/",
                root.display()
            )
        })
}

/// The package name and version in a cargo package id: `name@version`, or a
/// source URL whose fragment is `name@version` or, when the name is the last
/// segment of the path, only `version`.
fn package_and_version(id: &str) -> Option<(String, String)> {
    let Some((source, fragment)) = id.rsplit_once('#') else {
        let (name, version) = id.split_once('@')?;
        return Some((name.to_owned(), version.to_owned()));
    };
    if let Some((name, version)) = fragment.split_once('@') {
        return Some((name.to_owned(), version.to_owned()));
    }
    let name = source.trim_end_matches('/').rsplit('/').next()?;
    Some((name.to_owned(), fragment.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const ROOT: &str = "/r";

    fn artifact(package_id: &str, kind: &str, name: &str, test: bool, exe: Option<&str>) -> String {
        let manifest = if package_id.contains("brightfield-shell") {
            "/r/crates/brightfield-shell/Cargo.toml"
        } else {
            "/r/crates/brightfield-keys/Cargo.toml"
        };
        json!({
            "reason": "compiler-artifact",
            "package_id": package_id,
            "manifest_path": manifest,
            "target": {"kind": [kind], "crate_types": ["bin"], "name": name},
            "profile": {"test": test},
            "executable": exe,
        })
        .to_string()
    }

    fn messages() -> String {
        let shell = "path+file:///r/crates/brightfield-shell#0.1.7";
        let keys = "path+file:///r/crates/brightfield-keys#0.1.0";
        [
            artifact(
                keys,
                "lib",
                "brightfield_keys",
                true,
                Some("/r/target/debug/deps/brightfield_keys-1"),
            ),
            artifact(
                shell,
                "bin",
                "brightfield-shell",
                true,
                Some("/r/target/debug/deps/brightfield_shell-2"),
            ),
            artifact(
                shell,
                "test",
                "front_door",
                true,
                Some("/r/target/debug/deps/front_door-3"),
            ),
            artifact(
                keys,
                "test",
                "provenance",
                true,
                Some("/r/target/debug/deps/provenance-4"),
            ),
            artifact(
                shell,
                "bin",
                "brightfield-shell",
                false,
                Some("/r/target/debug/brightfield-shell"),
            ),
            artifact(keys, "lib", "brightfield_keys", false, None),
            json!({"reason": "build-script-executed", "package_id": shell}).to_string(),
            json!({"reason": "build-finished", "success": true}).to_string(),
        ]
        .join("\n")
    }

    fn manifest() -> Manifest {
        Manifest::from_cargo_messages(&messages(), Path::new(ROOT)).expect("the fixture reads")
    }

    #[test]
    fn test_artifacts_become_executables_with_their_package() {
        let m = manifest();
        assert_eq!(m.executables.len(), 4);
        let front_door = m
            .executables
            .iter()
            .find(|e| e.name == "front_door")
            .expect("front_door");
        assert_eq!(
            front_door,
            &Executable {
                package: "brightfield-shell".to_owned(),
                version: "0.1.7".to_owned(),
                kind: Kind::Test,
                name: "front_door".to_owned(),
                path: "target/debug/deps/front_door-3".to_owned(),
                package_dir: "crates/brightfield-shell".to_owned(),
            }
        );
        let kinds: Vec<Kind> = m.executables.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&Kind::Lib) && kinds.contains(&Kind::Bin));
    }

    #[test]
    fn bins_are_collected_beside_the_test_executables() {
        assert_eq!(
            manifest().bins,
            vec!["target/debug/brightfield-shell".to_owned()]
        );
    }

    #[test]
    fn package_ids_are_read_in_both_forms() {
        let lines = [
            artifact(
                "brightfield-shell@0.1.7",
                "test",
                "a",
                true,
                Some("/r/target/debug/deps/a-1"),
            ),
            artifact(
                "path+file:///r/crates/somewhere#brightfield-shell@0.1.7",
                "test",
                "b",
                true,
                Some("/r/target/debug/deps/b-1"),
            ),
            artifact(
                "path+file:///r/crates/brightfield-shell#0.1.7",
                "test",
                "c",
                true,
                Some("/r/target/debug/deps/c-1"),
            ),
        ]
        .join("\n");
        let m = Manifest::from_cargo_messages(&lines, Path::new(ROOT)).expect("reads");
        for exe in &m.executables {
            assert_eq!(
                (exe.package.as_str(), exe.version.as_str()),
                ("brightfield-shell", "0.1.7"),
                "{}",
                exe.name
            );
        }
    }

    #[test]
    fn an_executable_outside_the_repository_is_refused() {
        let line = artifact(
            "path+file:///r/crates/brightfield-keys#0.1.0",
            "test",
            "provenance",
            true,
            Some("/elsewhere/debug/deps/provenance-4"),
        );
        let err = Manifest::from_cargo_messages(&line, Path::new(ROOT)).expect_err("refused");
        assert!(err.contains("/elsewhere/debug/deps/provenance-4"), "{err}");
    }

    #[test]
    fn items_select_lib_bins_and_named_targets() {
        let m = manifest();
        let items = ["@lib", "front_door", "@doc", "@protocols"].map(str::to_owned);
        let names: Vec<&str> = m
            .for_items(&items)
            .expect("selects")
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, vec!["brightfield_keys", "front_door"]);
        let bins = m.for_items(&["@bins".to_owned()]).expect("selects");
        assert_eq!(bins.len(), 1);
        assert_eq!(bins[0].kind, Kind::Bin);
    }

    #[test]
    fn an_item_with_no_executable_is_refused() {
        let err = manifest()
            .for_items(&["no_such".to_owned()])
            .expect_err("refused");
        assert!(err.contains("no_such"), "{err}");
    }

    #[test]
    fn the_manifest_survives_json() {
        let m = manifest();
        assert_eq!(Manifest::from_json(&m.to_json()).expect("reads back"), m);
    }

    #[test]
    fn runs_from_the_package_root_with_cargo_variables() {
        let m = manifest();
        let exe = m
            .executables
            .iter()
            .find(|e| e.name == "front_door")
            .expect("front_door");
        let command = m.command(exe, Path::new(ROOT));
        assert_eq!(command.get_program(), "/r/target/debug/deps/front_door-3");
        assert_eq!(
            command.get_current_dir(),
            Some(PathBuf::from("/r/crates/brightfield-shell").as_path())
        );
        let env = |key: &str| {
            command
                .get_envs()
                .find(|(k, _)| *k == key)
                .and_then(|(_, v)| v)
                .map(|v| v.to_string_lossy().into_owned())
        };
        assert_eq!(
            env("CARGO_MANIFEST_DIR").as_deref(),
            Some("/r/crates/brightfield-shell")
        );
        assert_eq!(env("CARGO_PKG_NAME").as_deref(), Some("brightfield-shell"));
        assert_eq!(env("CARGO_PKG_VERSION").as_deref(), Some("0.1.7"));
        assert_eq!(env("CARGO_PKG_VERSION_MINOR").as_deref(), Some("1"));
        assert_eq!(env("CARGO_CRATE_NAME").as_deref(), Some("front_door"));
    }
}
