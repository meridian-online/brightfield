//! The Kata issues that flag main's state for Hugh: one open issue per label,
//! raised when its condition holds and cleared when it ends. The pull-request
//! gate and the nightly gate share this code, so both keep the same issue in
//! step.
//!
//! Nothing here stops a gate: a failed tracker call is printed and passed
//! over, since a flag is a note for Hugh and not part of any verdict.

use std::ffi::OsStr;

use serde_json::Value;

use crate::gate::output_of;

/// One kind of flag: the label that finds its one open issue, the title and
/// body an issue is opened with, and the line `work.attention_msg` shows.
#[derive(Debug)]
pub struct Flag {
    pub label: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub attention: &'static str,
}

/// Main's nightly gate failed: main is red.
pub const MAIN_RED: Flag = Flag {
    label: "main-red",
    title: "main is red",
    body: "The nightly gate on the Mac Studio runs `cargo xtask ci` on main's newest commit and posts its verdict as the `nightly-gate` commit status. Its newest verdict is a failure, so main is red. The nightly and `cargo xtask gate`, which reads that status before each pull request's gate, each comment a red commit here once, and either closes this issue when the newest verdict is a success. A red main does not stop a pull request's gate, since the fix goes forward.",
    attention: "main is red: the nightly gate failed",
};

/// Main has gone too long without a nightly verdict.
pub const NIGHTLY_MISSING: Flag = Flag {
    label: "nightly-missing",
    title: "main's nightly gate has not given a verdict",
    body: "The nightly gate posts a `success` or `failure` `nightly-gate` status on main each night. Main has gone more than 36 hours without one, or the nightly stopped before its steps, so main's state is unknown. The nightly may not be scheduled, the machine may have been off, or the run may have stopped early; its log says which. `cargo xtask gate` raises this issue but never closes it: only the nightly's next verdict does.",
    attention: "main's nightly gate has not given a verdict",
};

/// Main's nightly network tests failed. They read published data and remote
/// hosts, so a failure is more often the world moving than a commit breaking
/// something, and it does not make main red. Only the nightly gate raises
/// and clears it, and no caller does yet outside the tests.
#[cfg_attr(not(test), allow(dead_code))]
pub const NIGHTLY_NETWORK: Flag = Flag {
    label: "nightly-network",
    title: "main's nightly network tests failed",
    body: "The nightly gate runs the network tests on main and posts their verdict as the `nightly-network` commit status. They failed. They read published data and remote hosts, so a failure more often means the data moved or a host was down than that a commit broke something, and it does not make main red. The nightly closes this issue when they pass again.",
    attention: "main's nightly network tests failed",
};

fn say(message: &str) {
    eprintln!("cargo xtask: the issue tracker call failed, and the run carries on: {message}");
}

/// Run kata through `program` in brightfield's project, so a test can stand a
/// shim in for it.
fn kata_with(program: &OsStr, args: &[&str]) -> Result<String, String> {
    let mut all = args.to_vec();
    all.extend(["--project", "brightfield"]);
    output_of(program, &all)
}

/// The short id of the open issue labelled `label`, if there is one.
fn open_issue(program: &OsStr, label: &str) -> Result<Option<String>, String> {
    let json = kata_with(program, &["list", "--label", label, "--json"])?;
    let doc: Value =
        serde_json::from_str(&json).map_err(|e| format!("`kata list` is not JSON: {e}"))?;
    Ok(doc
        .pointer("/issues/0/short_id")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

/// Raise `flag` with the real `kata`.
pub fn raise(flag: &Flag, comment: &str) {
    raise_with(OsStr::new("kata"), flag, comment);
}

/// Clear `flag` with the real `kata`.
pub fn clear(flag: &Flag, sha: &str, note: &str) {
    clear_with(OsStr::new("kata"), flag, sha, note);
}

/// Find the open issue labelled `flag.label`, or open one, flag it for Hugh,
/// and add `comment` unless a comment on it already holds that text. A caller
/// that raises the same flag on every run passes a comment naming what it saw
/// (a commit, a day), so the issue gains one comment per new sighting.
pub fn raise_with(kata: &OsStr, flag: &Flag, comment: &str) {
    if let Err(e) = try_raise(kata, flag, comment) {
        say(&e);
    }
}

fn try_raise(kata: &OsStr, flag: &Flag, comment: &str) -> Result<(), String> {
    let issue = match open_issue(kata, flag.label)? {
        Some(issue) => issue,
        None => {
            // `--force-new` past a closed look-alike is safe because this runs
            // under the gate lock, after finding no open one.
            kata_with(
                kata,
                &[
                    "create",
                    flag.title,
                    "--body",
                    flag.body,
                    "--label",
                    flag.label,
                    "--force-new",
                    "--agent",
                ],
            )?;
            open_issue(kata, flag.label)?
                .ok_or_else(|| format!("`kata create` left no open {} issue", flag.label))?
        }
    };
    // Flag it on every raise, not only when it is opened, so an issue Hugh set
    // back to `ok` while its condition still holds asks for him again.
    for (key, value) in [
        ("work.attention", "needs-human"),
        ("work.attention_msg", flag.attention),
    ] {
        if let Err(e) = kata_with(kata, &["meta", "set", &issue, key, value, "--agent"]) {
            say(&e);
        }
    }
    let shown = kata_with(kata, &["show", &issue, "--json"])?;
    let noted = serde_json::from_str::<Value>(&shown)
        .ok()
        .and_then(|doc| doc.get("comments").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c.get("body").and_then(Value::as_str))
        .any(|body| body.contains(comment));
    if !noted {
        kata_with(kata, &["comment", &issue, "--body", comment, "--agent"])?;
    }
    Ok(())
}

/// Close the open issue labelled `flag.label`, if there is one, with `sha` as
/// its evidence: kata refuses a done close without evidence. `note` rides on
/// the close as its `--comment`, which kata posts only once the close
/// succeeds, so a refused close leaves no note to repeat on the next run.
pub fn clear_with(kata: &OsStr, flag: &Flag, sha: &str, note: &str) {
    let issue = match open_issue(kata, flag.label) {
        Ok(Some(issue)) => issue,
        Ok(None) => return,
        Err(e) => {
            say(&e);
            return;
        }
    };
    // kata wants a done close's message at least 40 characters long.
    let message = format!("The condition this issue flagged has cleared: {note}");
    let closed = kata_with(
        kata,
        &[
            "close",
            &issue,
            "--done",
            "--message",
            &message,
            "--commit",
            sha,
            "--comment",
            note,
            "--agent",
        ],
    );
    if let Err(e) = closed {
        say(&e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("flags-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A kata that logs every call to `calls`, lists an open issue `flag1`
    /// once one has been created (or from the start, when `open`), shows the
    /// comments made on it so far, and refuses a done close without evidence,
    /// as the real one does.
    fn shim(dir: &Path, open: bool) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let kata = dir.join("kata");
        if open {
            fs::write(dir.join("created"), "").unwrap();
        }
        let d = dir.display();
        fs::write(
            &kata,
            format!(
                r#"#!/bin/sh
echo "$*" >> '{d}/calls'
case "$1" in
  list) if [ -f '{d}/created' ]; then echo '{{"issues":[{{"short_id":"flag1"}}]}}'; else echo '{{"issues":[]}}'; fi ;;
  create) touch '{d}/created' ;;
  comment) printf '%s' "$4" > '{d}/comment' ;;
  show) if [ -f '{d}/comment' ]; then printf '{{"comments":[{{"body":"%s"}}]}}' "$(cat '{d}/comment')"; else echo '{{"comments":[]}}'; fi ;;
  close) case "$*" in *--commit*) ;; *) echo 'evidence required for reason=done' >&2; exit 3 ;; esac ;;
esac
"#
            ),
        )
        .unwrap();
        fs::set_permissions(&kata, fs::Permissions::from_mode(0o755)).unwrap();
        kata
    }

    fn calls(dir: &Path) -> Vec<String> {
        fs::read_to_string(dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn raise_opens_flags_and_comments_once() {
        let dir = scratch("raise");
        let kata = shim(&dir, false);
        let comment = "the nightly gate failed on main at abc1234: failed at clippy";
        raise_with(kata.as_os_str(), &MAIN_RED, comment);
        let first = calls(&dir);
        let creates: Vec<_> = first.iter().filter(|c| c.starts_with("create ")).collect();
        assert_eq!(creates.len(), 1, "{first:?}");
        assert!(creates[0].contains("--label main-red"), "{first:?}");
        assert!(creates[0].contains("--force-new"), "{first:?}");
        let metas: Vec<_> = first
            .iter()
            .filter(|c| c.starts_with("meta set flag1 "))
            .collect();
        assert_eq!(metas.len(), 2, "{first:?}");
        assert!(metas[0].contains("work.attention needs-human"), "{first:?}");
        assert!(
            metas[1].contains("work.attention_msg main is red: the nightly gate failed"),
            "{first:?}"
        );
        let comments: Vec<_> = first.iter().filter(|c| c.starts_with("comment ")).collect();
        assert_eq!(comments.len(), 1, "{first:?}");
        assert!(comments[0].contains(comment), "{first:?}");

        // The shim now lists the issue and its comment: a second raise with
        // the same comment opens nothing and adds no comment.
        raise_with(kata.as_os_str(), &MAIN_RED, comment);
        let second = &calls(&dir)[first.len()..];
        assert!(
            !second.iter().any(|c| c.starts_with("create ")),
            "{second:?}"
        );
        assert!(
            !second.iter().any(|c| c.starts_with("comment ")),
            "{second:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clear_closes_with_the_commit_as_evidence() {
        let dir = scratch("clear");
        let kata = shim(&dir, true);
        clear_with(
            kata.as_os_str(),
            &MAIN_RED,
            "abc1234def",
            "main is green at abc1234",
        );
        let lines = calls(&dir);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("list --label main-red "), "{lines:?}");
        assert!(lines[1].starts_with("close flag1 --done"), "{lines:?}");
        assert!(lines[1].contains("--commit abc1234def"), "{lines:?}");
        assert!(
            lines[1].contains("--comment main is green at abc1234"),
            "{lines:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clear_with_nothing_open_calls_only_list() {
        let dir = scratch("clear-none");
        let kata = shim(&dir, false);
        clear_with(kata.as_os_str(), &NIGHTLY_NETWORK, "abc1234def", "passed");
        let lines = calls(&dir);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("list --label nightly-network "),
            "{lines:?}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
