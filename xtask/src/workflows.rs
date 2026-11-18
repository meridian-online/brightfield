//! Reading the workflow files, so the command and CI cannot drift apart, and
//! so no hosted Mac runs on anything but a release tag or a person's say-so.
//!
//! Four things are reported, each with the file and line it was read at:
//!
//! * a step the command carries that no workflow invokes, so that no step is
//!   left without a hosted way to run it (a workflow run by hand counts);
//! * a workflow invoking a step the command does not carry, which CI would
//!   fail on with an unknown step;
//! * a workflow that starts on its own, on anything but a tag push, calling a
//!   gate script or cargo directly rather than through the command, which CI
//!   would run and a local run would not;
//! * a macOS job in a workflow that starts on its own, on anything but a tag
//!   push, or a call from such a workflow to one that has a macOS job. Hosted
//!   macOS minutes are for release tags; the Mac Studio's nightly runs every
//!   step of the command on `main`, and the hosted Mac workflows are run by
//!   hand.
//!
//! The reader is line-based rather than a YAML parser: it reads `run:` values
//! (a single line, or a block scalar's more-indented lines), `runs-on:` and
//! `uses:` lines, and the `on:` block, and it skips every line whose first
//! non-blank character is `#`, which is a comment in YAML and in the shell
//! alike.

use std::fs;
use std::path::Path;

/// A workflow file, by its path from the repository root.
pub struct Workflow {
    pub path: String,
    pub text: String,
}

/// Every `.yml` and `.yaml` file in `.github/workflows`, sorted by path.
pub fn read_all(root: &Path) -> Result<Vec<Workflow>, String> {
    let dir = root.join(".github/workflows");
    let entries = fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut workflows = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        let is_yaml = matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml" | "yaml")
        );
        if !is_yaml {
            continue;
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let rel = path.strip_prefix(root).unwrap_or(&path);
        workflows.push(Workflow {
            path: rel.display().to_string(),
            text,
        });
    }
    workflows.sort_by(|a, b| a.path.cmp(&b.path));
    if workflows.is_empty() {
        return Err(format!("no workflow file in {}", dir.display()));
    }
    Ok(workflows)
}

/// The lines of every `run:` value in `text`, each with its 1-based line
/// number, comment lines left out.
pub fn run_lines(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let Some((key_indent, value)) = run_key(line) else {
            i += 1;
            continue;
        };
        let value = value.trim();
        if value.starts_with('|') || value.starts_with('>') {
            i += 1;
            while i < lines.len() {
                let body = lines[i];
                if body.trim().is_empty() {
                    i += 1;
                    continue;
                }
                if indent(body) <= key_indent {
                    break;
                }
                if !body.trim_start().starts_with('#') {
                    out.push((i + 1, body.trim().to_owned()));
                }
                i += 1;
            }
        } else {
            if !value.is_empty() {
                out.push((i + 1, value.to_owned()));
            }
            i += 1;
        }
    }
    out
}

/// `Some((indent of the key, the value after it))` when `line` is a `run:`
/// key, in either the `run:` or the `- run:` form, and not a comment.
fn run_key(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return None;
    }
    let mut at = indent(line);
    let mut rest = trimmed;
    if let Some(after) = rest.strip_prefix("- ") {
        let after_trimmed = after.trim_start();
        at += 2 + (after.len() - after_trimmed.len());
        rest = after_trimmed;
    }
    rest.strip_prefix("run:").map(|value| (at, value))
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The steps a line of shell invokes as `cargo xtask ci --step <name>` (or
/// `--step=<name>`), in order.
pub fn invoked_steps(line: &str) -> Vec<String> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let mut steps = Vec::new();
    let mut i = 0;
    while i + 2 < tokens.len() {
        if tokens[i..i + 3] != ["cargo", "xtask", "ci"] {
            i += 1;
            continue;
        }
        let mut j = i + 3;
        while j < tokens.len() && !matches!(tokens[j], "&&" | "||" | ";" | "|") {
            if tokens[j] == "--step" {
                if let Some(name) = tokens.get(j + 1) {
                    steps.push(unquote(name));
                    j += 1;
                }
            } else if let Some(name) = tokens[j].strip_prefix("--step=") {
                steps.push(unquote(name));
            }
            j += 1;
        }
        i = j;
    }
    steps
}

fn unquote(word: &str) -> String {
    word.trim_matches(|c| c == '"' || c == '\'').to_owned()
}

/// The triggers under the workflow's `on:` that start it without a person:
/// every one but `workflow_dispatch` and `workflow_call`, in the order
/// written. A `push` whose filter names tags and no branches is reported as
/// `push(tags-only)`, since only a tag push starts it. A form the reader cannot
/// rule out as a branch push, such as `push: ~`, is a plain `push`, so a
/// workflow it misreads is checked rather than passed over.
pub fn automatic_triggers(text: &str) -> Vec<String> {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    let mut in_on = false;
    let mut trigger_indent = None;
    for raw in text.lines() {
        if raw.trim_start().starts_with('#') || raw.trim().is_empty() {
            continue;
        }
        let line = raw.split(" #").next().unwrap_or(raw).trim_end();
        if indent(line) == 0 {
            if in_on {
                break;
            }
            if let Some(rest) = line.strip_prefix("on:") {
                let rest = rest.trim();
                if !rest.is_empty() {
                    // `on: push` or `on: [push, pull_request]`: each word is a
                    // trigger with no filter.
                    found = rest
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .filter(|word| !word.is_empty())
                        .map(|word| (word.to_owned(), Vec::new()))
                        .collect();
                    break;
                }
                in_on = true;
            }
            continue;
        }
        if !in_on {
            continue;
        }
        if indent(line) == *trigger_indent.get_or_insert(indent(line)) {
            let item = line.trim();
            let (name, rest) = item.split_once(':').unwrap_or((item, ""));
            let rest = rest.trim();
            // An inline value is read as one line of the trigger's block.
            let body = if rest.is_empty() {
                Vec::new()
            } else {
                vec![rest.to_owned()]
            };
            found.push((name.trim().to_owned(), body));
        } else if let Some((_, body)) = found.last_mut() {
            body.push(line.trim().to_owned());
        }
    }
    found
        .into_iter()
        .filter(|(name, _)| name != "workflow_dispatch" && name != "workflow_call")
        .map(|(name, body)| {
            let tags = body.iter().any(|line| line.contains("tags"));
            let branches = body.iter().any(|line| line.contains("branches"));
            if name == "push" && tags && !branches {
                "push(tags-only)".to_owned()
            } else {
                name
            }
        })
        .collect()
}

/// The first trigger that starts the workflow on something other than a tag
/// push, if it has one.
fn untagged_trigger(text: &str) -> Option<String> {
    automatic_triggers(text)
        .into_iter()
        .find(|trigger| trigger != "push(tags-only)")
}

/// Each job runner in `text` that names a hosted macOS image (`macos-*`), with
/// its 1-based line number and the `runs-on` value as written. A runner chosen
/// through an expression is not read.
pub fn macos_jobs(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some(value) = trimmed.strip_prefix("runs-on:") else {
            continue;
        };
        let value = value.split(" #").next().unwrap_or(value).trim();
        let names_macos = value
            .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '.' && c != '_')
            .any(|word| word.starts_with("macos-"));
        if names_macos {
            out.push((i + 1, value.to_owned()));
        }
    }
    out
}

/// Each `uses: ./.github/workflows/<file>` call in `text`, with its 1-based
/// line number and the file name it calls.
fn workflow_calls(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let trimmed = raw.trim_start();
        let trimmed = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        let Some(value) = trimmed.strip_prefix("uses:") else {
            continue;
        };
        let value = unquote(value.split(" #").next().unwrap_or(value).trim());
        if let Some(file) = value.strip_prefix("./.github/workflows/") {
            out.push((i + 1, file.to_owned()));
        }
    }
    out
}

/// A call in `line` to a gate script or to cargo other than `cargo xtask`.
pub fn direct_gate_call(line: &str) -> Option<String> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    for (i, token) in tokens.iter().enumerate() {
        if token.contains("scripts/") {
            return Some(line.to_owned());
        }
        let is_cargo = token.trim_start_matches("$(") == "cargo";
        if is_cargo && tokens.get(i + 1) != Some(&"xtask") {
            return Some(line.to_owned());
        }
    }
    None
}

/// Every disagreement between the command's `steps` and what `workflows`
/// invoke, and every macOS job a workflow can start without a person; empty
/// when there is none.
pub fn coverage(steps: &[&str], workflows: &[Workflow]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut invoked: Vec<String> = Vec::new();
    let has_macos = runs_a_macos_job(workflows);
    for workflow in workflows {
        let started_by = untagged_trigger(&workflow.text);
        for (line_no, line) in run_lines(&workflow.text) {
            for name in invoked_steps(&line) {
                if !steps.contains(&name.as_str()) {
                    problems.push(format!(
                        "{}:{line_no} invokes the step '{name}', which `cargo xtask ci` does not carry; `cargo xtask ci --list` names the steps it has",
                        workflow.path
                    ));
                }
                invoked.push(name);
            }
            let Some(trigger) = &started_by else {
                continue;
            };
            if let Some(call) = direct_gate_call(&line) {
                problems.push(format!(
                    "{}:{line_no} runs `{call}` directly in a workflow that {trigger} starts; call the command's step instead (`cargo xtask ci --step <name>`), so the check runs what a local run runs",
                    workflow.path
                ));
            }
        }
        let Some(trigger) = &started_by else {
            continue;
        };
        let macos_lines = macos_jobs(&workflow.text).into_iter().map(|(at, _)| at);
        let calls = workflow_calls(&workflow.text)
            .into_iter()
            .filter(|(_, called)| {
                workflows
                    .iter()
                    .zip(&has_macos)
                    .any(|(other, mac)| *mac && file_name(other) == called)
            })
            .map(|(at, _)| at);
        let mut lines: Vec<usize> = macos_lines.chain(calls).collect();
        lines.sort_unstable();
        for line_no in lines {
            problems.push(format!(
                "{}:{line_no}: a macOS job in a workflow that {trigger} starts; macOS runners are for release tags only",
                workflow.path
            ));
        }
    }
    for step in steps {
        if !invoked.iter().any(|name| name == step) {
            problems.push(format!(
                "the step '{step}' is carried by `cargo xtask ci` and invoked by no workflow, so no hosted run can run it; add `cargo xtask ci --step {step}` to the job that should run it, or remove the step"
            ));
        }
    }
    problems
}

/// The workflow's file name, without its directory.
fn file_name(workflow: &Workflow) -> &str {
    workflow.path.rsplit('/').next().unwrap_or(&workflow.path)
}

/// For each of `workflows`, whether it runs a macOS job itself or through a
/// workflow it calls, at any depth.
fn runs_a_macos_job(workflows: &[Workflow]) -> Vec<bool> {
    let calls: Vec<Vec<String>> = workflows
        .iter()
        .map(|w| {
            workflow_calls(&w.text)
                .into_iter()
                .map(|(_, f)| f)
                .collect()
        })
        .collect();
    let mut mac: Vec<bool> = workflows
        .iter()
        .map(|w| !macos_jobs(&w.text).is_empty())
        .collect();
    // Each pass marks the callers of a workflow marked before it, so as many
    // passes as there are workflows reach every depth a chain of calls has.
    for _ in 0..workflows.len() {
        let marked: Vec<&str> = workflows
            .iter()
            .zip(&mac)
            .filter(|(_, m)| **m)
            .map(|(w, _)| file_name(w))
            .collect();
        for (i, called) in calls.iter().enumerate() {
            mac[i] |= called.iter().any(|file| marked.contains(&file.as_str()));
        }
    }
    mac
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow(path: &str, text: &str) -> Workflow {
        Workflow {
            path: format!(".github/workflows/{path}"),
            text: text.to_owned(),
        }
    }

    const PR: &str =
        "on:\n  push:\n    branches: [main]\n  pull_request:\n\njobs:\n  a:\n    steps:\n";

    #[test]
    fn run_values_are_read_in_both_forms_and_comments_are_left_out() {
        let text = "jobs:\n  a:\n    steps:\n      - name: cargo fmt --all --check\n        run: cargo xtask ci --step fmt\n      # run: cargo xtask ci --step ghost\n      - run: |\n          git fetch origin main\n          # cargo xtask ci --step commented\n\n          cargo xtask ci --step clippy\n      - name: after\n        run: echo done\n";
        let lines: Vec<String> = run_lines(text).into_iter().map(|(_, l)| l).collect();
        assert_eq!(
            lines,
            [
                "cargo xtask ci --step fmt",
                "git fetch origin main",
                "cargo xtask ci --step clippy",
                "echo done"
            ]
        );
    }

    #[test]
    fn invocations_are_read_in_both_spellings_and_across_a_chain() {
        assert_eq!(invoked_steps("cargo xtask ci --step fmt"), ["fmt"]);
        assert_eq!(invoked_steps("cargo xtask ci --step=doc"), ["doc"]);
        assert_eq!(
            invoked_steps("git fetch && cargo xtask ci --step 'a' && cargo xtask ci --step b"),
            ["a", "b"]
        );
        assert!(invoked_steps("cargo xtask ci --list").is_empty());
        assert!(invoked_steps("echo cargo xtask --step fmt").is_empty());
    }

    #[test]
    fn a_direct_gate_call_in_a_workflow_only_main_triggers_is_reported() {
        let main_only = "on:\n  push:\n    branches: [main]\n  schedule:\n    - cron: '17 3 * * *'\n  workflow_dispatch:\n\njobs:\n  a:\n    steps:\n      - run: cargo xtask ci --step one\n      - run: cargo test --workspace\n";
        let problems = coverage(&["one"], &[workflow("test.yml", main_only)]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("cargo test --workspace"));
    }

    #[test]
    fn agreeing_files_report_nothing() {
        let files = [workflow("a.yml", &format!("{PR}      - run: cargo xtask ci --step one\n      - run: cargo xtask ci --step two\n"))];
        assert_eq!(coverage(&["one", "two"], &files), Vec::<String>::new());
    }

    #[test]
    fn a_carried_step_no_workflow_invokes_is_reported_by_name() {
        let files = [workflow(
            "a.yml",
            &format!("{PR}      - run: cargo xtask ci --step one\n"),
        )];
        let problems = coverage(&["one", "orphan"], &files);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("'orphan'") && problems[0].contains("invoked by no workflow"));
    }

    #[test]
    fn an_invoked_step_the_command_lacks_is_reported_at_its_file_and_line() {
        let files = [workflow("a.yml", &format!("{PR}      - run: cargo xtask ci --step one\n      - run: cargo xtask ci --step ghost\n"))];
        let problems = coverage(&["one"], &files);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].starts_with(".github/workflows/a.yml:10 "),
            "{}",
            problems[0]
        );
        assert!(problems[0].contains("'ghost'") && problems[0].contains("does not carry"));
    }

    #[test]
    fn a_direct_gate_call_in_a_pull_request_workflow_is_reported_and_elsewhere_is_not() {
        let direct = format!("{PR}      - run: cargo xtask ci --step one\n      - run: ./scripts/check-something.sh\n      - run: cargo test --workspace\n");
        let problems = coverage(&["one"], &[workflow("a.yml", &direct)]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("./scripts/check-something.sh"));
        assert!(problems[1].contains("cargo test --workspace"));

        let tag_only = "on:\n  push:\n    tags: ['v*']\njobs:\n  a:\n    steps:\n      - run: ./scripts/package.sh\n";
        let files = [
            workflow(
                "a.yml",
                &format!("{PR}      - run: cargo xtask ci --step one\n"),
            ),
            workflow("release.yml", tag_only),
            workflow(
                "by-hand.yml",
                "on:\n  workflow_dispatch:\njobs:\n  a:\n    steps:\n      - run: scripts/package.sh\n",
            ),
        ];
        assert_eq!(coverage(&["one"], &files), Vec::<String>::new());

        let scheduled = "on:\n  schedule:\n    - cron: '37 6 * * *'\njobs:\n  a:\n    steps:\n      - run: cargo xtask ci --step one\n      - run: scripts/nightly-network.sh\n";
        let problems = coverage(&["one"], &[workflow("a.yml", scheduled)]);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("a workflow that schedule starts"),
            "{}",
            problems[0]
        );
    }

    const MAC_JOB: &str =
        "jobs:\n  a:\n    runs-on: macos-15\n    steps:\n      - run: cargo xtask ci --step one\n";

    fn macos_findings(files: &[Workflow]) -> Vec<String> {
        coverage(&["one"], files)
            .into_iter()
            .filter(|p| p.contains("a macOS job"))
            .collect()
    }

    #[test]
    fn a_macos_job_on_a_pull_request_is_refused() {
        let files = [workflow(
            "a.yml",
            &format!("on:\n  pull_request:\n  workflow_dispatch:\n{MAC_JOB}"),
        )];
        assert_eq!(
            macos_findings(&files),
            [".github/workflows/a.yml:6: a macOS job in a workflow that pull_request starts; macOS runners are for release tags only"]
        );
    }

    #[test]
    fn a_macos_job_on_a_schedule_is_refused() {
        let files = [workflow(
            "a.yml",
            &format!("on:\n  schedule:\n    - cron: '17 3 * * *'\n{MAC_JOB}"),
        )];
        let found = macos_findings(&files);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("that schedule starts"), "{}", found[0]);
    }

    #[test]
    fn a_macos_job_on_a_branch_push_beside_tags_is_refused() {
        let files = [workflow(
            "a.yml",
            &format!("on:\n  push:\n    branches: [main]\n    tags: ['v*']\n{MAC_JOB}"),
        )];
        let found = macos_findings(&files);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("that push starts"), "{}", found[0]);
    }

    #[test]
    fn a_macos_job_on_a_tag_push_or_dispatch_or_call_is_allowed() {
        let tag = workflow(
            "tag.yml",
            &format!("on:\n  push:\n    tags: ['v*']\n{MAC_JOB}"),
        );
        let dispatch = workflow(
            "dispatch.yml",
            &format!("on:\n  workflow_dispatch:\n{MAC_JOB}"),
        );
        let call = workflow(
            "call.yml",
            &format!("on:\n  workflow_call:\n    inputs:\n      tag:\n        type: string\n  workflow_dispatch:\n{MAC_JOB}"),
        );
        assert_eq!(macos_findings(&[tag, dispatch, call]), Vec::<String>::new());
    }

    #[test]
    fn the_trigger_reader_names_each_automatic_trigger_and_reads_a_tag_push() {
        assert_eq!(automatic_triggers(PR), ["push", "pull_request"]);
        assert_eq!(
            automatic_triggers("on: [push, pull_request]\njobs:\n"),
            ["push", "pull_request"]
        );
        assert_eq!(
            automatic_triggers("on: workflow_dispatch\njobs:\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            automatic_triggers("on:\n  # 06:37 UTC.\n  schedule:\n    - cron: '37 6 * * *'\n  workflow_dispatch:\njobs:\n"),
            ["schedule"]
        );
        assert_eq!(
            automatic_triggers("on:\n  push:\n    # a version tag\n    tags: ['v*']\njobs:\n"),
            ["push(tags-only)"]
        );
        assert_eq!(
            automatic_triggers("on:\n  push: {tags: ['v*']}\njobs:\n"),
            ["push(tags-only)"]
        );
        assert_eq!(
            automatic_triggers(
                "on:\n  push:\n    branches-ignore: [main]\n    tags: ['v*']\njobs:\n"
            ),
            ["push"]
        );
        assert_eq!(
            automatic_triggers("on:\n  push:  # every branch\njobs:\n"),
            ["push"]
        );
        assert_eq!(automatic_triggers("on:\n  push: ~\njobs:\n"), ["push"]);
        assert_eq!(
            automatic_triggers(
                "on:\n  workflow_call:\n    inputs:\n      push:\n        type: string\njobs:\n"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_macos_runner_is_read_in_its_forms_and_no_other_runner_is() {
        let text = "jobs:\n  a:\n    runs-on: macos-15\n  b:\n    runs-on: ubuntu-latest\n  c:\n    runs-on: \"macos-latest\"  # newest\n    # runs-on: macos-14\n  d:\n    runs-on: [self-hosted, macos-14]\n";
        assert_eq!(
            macos_jobs(text),
            [
                (3, "macos-15".to_owned()),
                (7, "\"macos-latest\"".to_owned()),
                (10, "[self-hosted, macos-14]".to_owned())
            ]
        );
    }

    #[test]
    fn a_pull_request_workflow_calling_a_macos_workflow_is_refused() {
        let called = workflow(
            "brew-install.yml",
            &format!("on:\n  workflow_call:\n  workflow_dispatch:\n{MAC_JOB}"),
        );
        let caller = workflow(
            "a.yml",
            "on:\n  pull_request:\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo xtask ci --step one\n  b:\n    uses: ./.github/workflows/brew-install.yml\n",
        );
        let release = workflow(
            "release.yml",
            "on:\n  push:\n    tags: ['v*']\njobs:\n  c:\n    uses: ./.github/workflows/brew-install.yml\n",
        );
        assert_eq!(
            macos_findings(&[caller, called, release]),
            [".github/workflows/a.yml:9: a macOS job in a workflow that pull_request starts; macOS runners are for release tags only"]
        );
    }

    /// Goal 1, read from the real files: nothing a push, a pull request or a
    /// schedule starts runs on a hosted Mac.
    #[test]
    fn the_repository_s_workflows_start_no_macos_job_automatically() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let files = read_all(&root).expect("the workflow files read");
        let names: Vec<&str> = crate::steps::STEPS.iter().map(|s| s.name).collect();
        let found: Vec<String> = coverage(&names, &files)
            .into_iter()
            .filter(|p| p.contains("a macOS job"))
            .collect();
        assert_eq!(found, Vec::<String>::new());
    }

    /// The repository's own workflow files against the command's own steps.
    /// The `workflow-coverage` step runs the same check; this keeps it red in
    /// the suite as well, beside the code it reads.
    #[test]
    fn this_repositorys_workflows_and_steps_agree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let files = read_all(&root).expect("the workflow files read");
        let names: Vec<&str> = crate::steps::STEPS.iter().map(|s| s.name).collect();
        assert_eq!(coverage(&names, &files), Vec::<String>::new());
    }
}
