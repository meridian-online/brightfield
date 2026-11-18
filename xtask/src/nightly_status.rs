//! Main's nightly verdict as GitHub holds it: a commit status that the
//! nightly gate posts on the commit it ran. The pull-request gate reads the
//! newest verdict among main's recent commits to keep the `main-red` and
//! `nightly-missing` issues in step, and both gates post their own statuses
//! through [`post_status`].
//!
//! There is no date crate in `xtask`, so GitHub's `createdAt` times are read to
//! unix seconds here by hand.

use std::ffi::OsStr;
use std::time::Duration;

use serde_json::Value;

use crate::gate::{output_within, CALL_TIMEOUT, REPO};

/// How many of main's newest commits are searched for a verdict. A nightly
/// runs once a day, so a verdict older than this many merges is stale anyway.
pub const COMMITS_READ: u32 = 50;

/// How long main may go without a nightly verdict before the nightly counts as
/// missing: a day and a half, so one late or skipped night is not flagged but
/// two are.
pub const MISSING_AFTER: Duration = Duration::from_secs(36 * 60 * 60);

/// What a nightly verdict said.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VerdictState {
    Success,
    Failure,
}

/// The newest verdict found on main, or none among the commits read.
#[derive(Debug, Clone, PartialEq)]
pub enum Seen {
    Verdict {
        sha: String,
        state: VerdictState,
        description: String,
        /// When the status was posted, as GitHub gives it:
        /// `YYYY-MM-DDTHH:MM:SSZ`.
        at: String,
    },
    None,
}

/// The GraphQL query for main's newest commits and their statuses, for
/// `gh api graphql -f query=...`. It asks for every context on each commit,
/// since a commit's statuses are a short list; the leading comment names the
/// context the caller looks for, so the query reads plainly in a log.
pub fn query(context: &str) -> String {
    let (owner, name) = REPO.split_once('/').unwrap_or((REPO, REPO));
    format!(
        "# main's newest `{context}` verdict\n\
         query {{ repository(owner: \"{owner}\", name: \"{name}\") {{ \
         ref(qualifiedName: \"refs/heads/main\") {{ target {{ ... on Commit {{ \
         history(first: {COMMITS_READ}) {{ nodes {{ oid \
         status {{ contexts {{ context state description createdAt }} }} \
         }} }} }} }} }} }} }}"
    )
}

/// Read the answer to [`query`]: the newest commit, first in the history,
/// whose `context` status is `SUCCESS` or `FAILURE`. `PENDING` and `ERROR`
/// are passed over, since neither is a verdict on the commit.
pub fn read_newest_verdict(graphql_json: &str, context: &str) -> Result<Seen, String> {
    let doc: Value = serde_json::from_str(graphql_json)
        .map_err(|e| format!("the GraphQL answer is not JSON: {e}"))?;
    let nodes = doc
        .pointer("/data/repository/ref/target/history/nodes")
        .and_then(Value::as_array)
        .ok_or("the GraphQL answer has no history of main")?;
    for node in nodes {
        let Some(contexts) = node.pointer("/status/contexts").and_then(Value::as_array) else {
            continue;
        };
        for status in contexts {
            let text = |key: &str| status.get(key).and_then(Value::as_str);
            if text("context") != Some(context) {
                continue;
            }
            let state = match text("state") {
                Some("SUCCESS") => VerdictState::Success,
                Some("FAILURE") => VerdictState::Failure,
                _ => continue,
            };
            let sha = node
                .get("oid")
                .and_then(Value::as_str)
                .ok_or("a commit in main's history has no oid")?;
            return Ok(Seen::Verdict {
                sha: sha.to_owned(),
                state,
                description: text("description").unwrap_or_default().to_owned(),
                at: text("createdAt").unwrap_or_default().to_owned(),
            });
        }
    }
    Ok(Seen::None)
}

/// Whether the nightly is missing at `now_unix`: no verdict, a verdict whose
/// time cannot be read, or one more than [`MISSING_AFTER`] old.
pub fn is_missing(seen: &Seen, now_unix: u64) -> bool {
    match seen {
        Seen::None => true,
        Seen::Verdict { at, .. } => match unix_seconds(at) {
            Ok(then) => now_unix.saturating_sub(then) > MISSING_AFTER.as_secs(),
            Err(_) => true,
        },
    }
}

/// Read `YYYY-MM-DDTHH:MM:SSZ`, the form GitHub gives `createdAt` in, to
/// seconds since the unix epoch.
pub fn unix_seconds(iso: &str) -> Result<u64, String> {
    let bad = || format!("`{iso}` is not a YYYY-MM-DDTHH:MM:SSZ time");
    let b = iso.as_bytes();
    let shape_ok = b.len() == 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'Z';
    if !shape_ok {
        return Err(bad());
    }
    let number = |range: std::ops::Range<usize>| -> Result<u64, String> {
        let digits = &iso[range];
        if digits.bytes().all(|c| c.is_ascii_digit()) {
            digits.parse().map_err(|_| bad())
        } else {
            Err(bad())
        }
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1..=12 => 31,
        _ => return Err(bad()),
    };
    if year < 1970 || day == 0 || day > month_days || hour > 23 || minute > 59 || second > 59 {
        return Err(bad());
    }
    let days = days_from_civil(year, month, day);
    Ok(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days from 1970-01-01 to a date on or after it in the proleptic Gregorian
/// calendar, by Howard Hinnant's `days_from_civil`: the year is counted from
/// March, so the leap day falls at its end.
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let year_of_era = y - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    // 719468 is the number of days from 0000-03-01 to 1970-01-01.
    era * 146_097 + day_of_era - 719_468
}

/// The pauses between tries of a status post: three tries in all. GitHub
/// sometimes answers a post with an error and no body, which `gh` reports as
/// `unexpected end of JSON input`; one such answer cost main a night's
/// verdict while the post after it, a moment later, landed.
const POST_PAUSES: [Duration; 2] = [Duration::from_secs(15), Duration::from_secs(60)];

/// Post a commit status in `context` on `sha` with `gh`, which runs with this
/// process's own HOME and so its own credentials. A post that fails is tried
/// again after each of `POST_PAUSES`, unless GitHub refused the request
/// itself (`a_post_github_refused_is_not_tried_again`), since the same
/// request would be refused again. A post happens under the gate lock, so
/// each try is held to `CALL_TIMEOUT` like the gate's other calls
/// (`a_status_post_that_hangs_is_stopped_and_is_an_error`).
pub fn post_status(
    gh: &OsStr,
    sha: &str,
    context: &str,
    state: &str,
    text: &str,
) -> Result<(), String> {
    post_status_with(gh, sha, context, state, text, CALL_TIMEOUT, &POST_PAUSES)
}

/// Whether `gh` said GitHub refused the request as the client's fault: an
/// HTTP 4xx other than 429, and other than a rate limit, which GitHub's
/// secondary limits answer with a 403; both ask the client to wait and try
/// again.
fn is_refusal(error: &str) -> bool {
    if error.to_ascii_lowercase().contains("rate limit") {
        return false;
    }
    error.match_indices("HTTP 4").any(|(at, _)| {
        let code = error.get(at + 5..at + 8).unwrap_or("");
        code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()) && code != "429"
    })
}

fn post_status_with(
    gh: &OsStr,
    sha: &str,
    context: &str,
    state: &str,
    text: &str,
    timeout: Duration,
    pauses: &[Duration],
) -> Result<(), String> {
    let endpoint = format!("repos/{REPO}/statuses/{sha}");
    let state = format!("state={state}");
    let context = format!("context={context}");
    let description = format!("description={text}");
    let args = [
        "api",
        "--silent",
        &endpoint,
        "-f",
        &state,
        "-f",
        &context,
        "-f",
        &description,
    ];
    let mut tries = 0;
    loop {
        tries += 1;
        let error = match output_within(gh, &args, timeout) {
            Ok(_) => return Ok(()),
            Err(e) => e,
        };
        if is_refusal(&error) {
            return Err(format!("the verdict was not posted: {error}"));
        }
        let Some(pause) = pauses.get(tries - 1) else {
            return Err(format!(
                "the verdict was not posted after {tries} tries: {error}"
            ));
        };
        eprintln!(
            "the status post failed, trying again in {}s: {error}",
            pause.as_secs()
        );
        std::thread::sleep(*pause);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    /// A GraphQL answer whose history holds one commit per entry, newest
    /// first, each with the statuses given as `(context, state)`.
    fn answer(commits: &[(&str, &[(&str, &str)])]) -> String {
        let nodes: Vec<String> = commits
            .iter()
            .map(|(oid, statuses)| {
                let contexts: Vec<String> = statuses
                    .iter()
                    .map(|(context, state)| {
                        format!(
                            r#"{{"context":"{context}","state":"{state}","description":"{context} {state}","createdAt":"2026-11-18T02:30:00Z"}}"#
                        )
                    })
                    .collect();
                format!(
                    r#"{{"oid":"{oid}","status":{{"contexts":[{}]}}}}"#,
                    contexts.join(",")
                )
            })
            .collect();
        format!(
            r#"{{"data":{{"repository":{{"ref":{{"target":{{"history":{{"nodes":[{}]}}}}}}}}}}}}"#,
            nodes.join(",")
        )
    }

    #[test]
    fn a_failure_on_the_newest_commit_is_red() {
        let json = answer(&[
            ("new", &[("nightly-gate", "FAILURE")]),
            ("old", &[("nightly-gate", "SUCCESS")]),
        ]);
        assert_eq!(
            read_newest_verdict(&json, "nightly-gate"),
            Ok(Seen::Verdict {
                sha: "new".into(),
                state: VerdictState::Failure,
                description: "nightly-gate FAILURE".into(),
                at: "2026-11-18T02:30:00Z".into(),
            })
        );
    }

    #[test]
    fn a_pending_or_error_status_is_not_a_verdict() {
        for state in ["PENDING", "ERROR"] {
            let json = answer(&[("only", &[("nightly-gate", state)])]);
            assert_eq!(read_newest_verdict(&json, "nightly-gate"), Ok(Seen::None));
        }
    }

    #[test]
    fn the_newest_verdict_wins_over_a_newer_error() {
        let json = answer(&[
            ("one", &[("nightly-gate", "ERROR")]),
            ("two", &[("nightly-gate", "FAILURE")]),
        ]);
        assert!(matches!(
            read_newest_verdict(&json, "nightly-gate"),
            Ok(Seen::Verdict { sha, state: VerdictState::Failure, .. }) if sha == "two"
        ));
    }

    #[test]
    fn another_context_is_ignored() {
        let json = answer(&[
            ("one", &[("local-gate", "FAILURE")]),
            ("two", &[("local-gate", "SUCCESS")]),
        ]);
        assert_eq!(read_newest_verdict(&json, "nightly-gate"), Ok(Seen::None));
        // A commit with no statuses at all has `status: null`.
        let null = r#"{"data":{"repository":{"ref":{"target":{"history":{"nodes":[{"oid":"a","status":null}]}}}}}}"#;
        assert_eq!(read_newest_verdict(null, "nightly-gate"), Ok(Seen::None));
        assert!(read_newest_verdict(r#"{"data":null}"#, "nightly-gate").is_err());
    }

    #[test]
    fn iso_times_read_to_unix_seconds() {
        assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Ok(0));
        assert_eq!(unix_seconds("2026-11-18T02:30:00Z"), Ok(1_794_969_000));
        assert_eq!(unix_seconds("2028-02-29T00:00:00Z"), Ok(1_835_395_200));
        for bad in [
            "",
            "2026-11-18",
            "2026-11-18 02:30:00Z",
            "2026-11-18T02:30:00+00:00",
            "2026-13-01T00:00:00Z",
            "2027-02-29T00:00:00Z",
            "2026-11-18T24:00:00Z",
            "2026-1a-18T02:30:00Z",
            "1969-12-31T23:59:59Z",
        ] {
            assert!(unix_seconds(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn missing_after_thirty_six_hours() {
        let at = "2026-11-18T02:30:00Z";
        let then = unix_seconds(at).unwrap();
        let seen = Seen::Verdict {
            sha: "a".into(),
            state: VerdictState::Success,
            description: String::new(),
            at: at.into(),
        };
        let hours = |h: u64| then + h * 60 * 60;
        assert!(!is_missing(&seen, hours(35)));
        assert!(is_missing(&seen, hours(37)));
        assert!(is_missing(&Seen::None, hours(0)));
    }

    #[test]
    fn the_query_reads_mains_last_commits_statuses() {
        let text = query("nightly-gate");
        assert!(text.contains("nightly-gate"), "{text}");
        assert!(text.contains(r#"repository(owner: "meridian-online", name: "brightfield")"#));
        assert!(text.contains(r#"ref(qualifiedName: "refs/heads/main")"#));
        assert!(text.contains("history(first: 50)"), "{text}");
        assert!(text.contains("contexts { context state description createdAt }"));
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("status-{tag}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_status_that_cannot_be_posted_is_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("post");
        let gh = dir.join("gh");
        fs::write(
            &gh,
            "#!/bin/sh\necho 'HTTP 401: Bad credentials' >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        let error =
            post_status(gh.as_os_str(), "abc123", "local-gate", "success", "passed").unwrap_err();
        assert!(error.contains("not posted"), "{error}");
        assert!(error.contains("Bad credentials"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_status_is_posted_in_the_context_given() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("context");
        let gh = dir.join("gh");
        let said = dir.join("args");
        fs::write(
            &gh,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", said.display()),
        )
        .unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        post_status(gh.as_os_str(), "abc123", "nightly-gate", "failure", "x").unwrap();
        let args = fs::read_to_string(&said).unwrap();
        assert!(args.lines().any(|l| l == "context=nightly-gate"), "{args}");
        assert!(
            args.lines()
                .any(|l| l == "repos/meridian-online/brightfield/statuses/abc123"),
            "{args}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A fake `gh` that counts its calls in the file it returns and fails the
    /// first `fails` of them, saying `said` on stderr, then succeeds.
    fn flaky_gh(dir: &Path, fails: u32, said: &str) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let gh = dir.join("gh");
        let calls = dir.join("calls");
        fs::write(
            &gh,
            format!(
                "#!/bin/sh\necho x >> '{calls}'\nn=$(wc -l < '{calls}')\n\
                 if [ \"$n\" -le {fails} ]; then echo '{said}' >&2; exit 1; fi\n",
                calls = calls.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        (gh, calls)
    }

    fn calls_made(calls: &Path) -> usize {
        fs::read_to_string(calls).map_or(0, |text| text.lines().count())
    }

    const NO_PAUSE: [Duration; 2] = [Duration::ZERO, Duration::ZERO];

    fn post_quickly(gh: &Path) -> Result<(), String> {
        post_status_with(
            gh.as_os_str(),
            "abc123",
            "nightly-gate",
            "failure",
            "x",
            CALL_TIMEOUT,
            &NO_PAUSE,
        )
    }

    #[test]
    fn a_post_github_answered_with_an_empty_error_is_tried_again() {
        // What `gh` printed the night a `nightly-gate` verdict went unposted
        // while the `nightly-network` post just after it landed.
        let dir = scratch("retry");
        let (gh, calls) = flaky_gh(&dir, 1, "unexpected end of JSON input");
        post_quickly(&gh).unwrap();
        assert_eq!(calls_made(&calls), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_post_that_keeps_failing_is_tried_three_times_and_is_an_error() {
        let dir = scratch("retry-out");
        let (gh, calls) = flaky_gh(&dir, 9, "HTTP 502: Bad Gateway");
        let error = post_quickly(&gh).unwrap_err();
        assert_eq!(calls_made(&calls), 3);
        assert!(error.contains("not posted after 3 tries"), "{error}");
        assert!(error.contains("Bad Gateway"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_post_github_refused_is_not_tried_again() {
        let dir = scratch("retry-refused");
        let (gh, calls) = flaky_gh(&dir, 9, "HTTP 422: Validation Failed");
        let error = post_quickly(&gh).unwrap_err();
        assert_eq!(calls_made(&calls), 1);
        assert!(error.contains("Validation Failed"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_post_github_rate_limited_is_tried_again() {
        let dir = scratch("retry-rate");
        let (gh, calls) = flaky_gh(&dir, 2, "HTTP 429: Too Many Requests");
        post_quickly(&gh).unwrap();
        assert_eq!(calls_made(&calls), 3);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_post_held_by_githubs_secondary_rate_limit_is_tried_again() {
        let dir = scratch("retry-secondary");
        let (gh, calls) = flaky_gh(
            &dir,
            1,
            "You have exceeded a secondary rate limit. Please wait a few minutes before you try again. (HTTP 403)",
        );
        post_quickly(&gh).unwrap();
        assert_eq!(calls_made(&calls), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_status_post_that_hangs_is_stopped_and_is_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("post-hang");
        let gh = dir.join("gh");
        fs::write(&gh, "#!/bin/sh\nexec sleep 30\n").unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        let started = Instant::now();
        let error = post_status_with(
            gh.as_os_str(),
            "abc123",
            "local-gate",
            "success",
            "passed",
            Duration::from_millis(300),
            &NO_PAUSE,
        )
        .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
        assert!(error.contains("not posted"), "{error}");
        assert!(error.contains("timed out"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
