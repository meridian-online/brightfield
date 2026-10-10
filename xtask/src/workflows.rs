//! Reading the workflow files, so the command and CI cannot drift apart.
//!
//! Three things are reported, each with the file and line it was read at:
//!
//! * a step the command carries that no workflow invokes, which a local run
//!   would run and CI would not;
//! * a workflow invoking a step the command does not carry, which CI would
//!   fail on with an unknown step;
//! * a workflow that a pull request triggers calling a gate script or cargo
//!   directly rather than through the command, which CI would run and a local
//!   run would not. The two calls that are not gates are named in
//!   `DIRECT_CALLS_ALLOWED`, and the one such workflow that stays outside the
//!   command is named in `WORKFLOWS_OUTSIDE_THE_COMMAND`, each with its reason.
//!
//! The reader is line-based rather than a YAML parser: it reads `run:` values
//! (a single line, or a block scalar's more-indented lines) and the `on:`
//! block, and it skips every line whose first non-blank character is `#`,
//! which is a comment in YAML and in the shell alike.

use std::fs;
use std::path::Path;

/// A workflow file, by its path from the repository root.
pub struct Workflow {
    pub path: String,
    pub text: String,
}

/// The direct calls a pull-request workflow may make, by file name and the text
/// the call contains, with the reason each is not a gate the command must run.
pub const DIRECT_CALLS_ALLOWED: &[(&str, &str, &str)] = &[
    (
        "test.yml",
        "scripts/fetch-duckdb-cli.sh",
        "it fetches the pinned DuckDB CLI the test steps hand to arc, and checks nothing in the tree; locally the test steps say where to put it",
    ),
    (
        "pr-text-hygiene.yml",
        "scripts/check-history-hygiene.sh --text",
        "it is the pull-request-text check, re-run on every edit of a description, and the event's text has no local form",
    ),
];

/// Workflows a pull request can trigger that call scripts directly and stay
/// outside the command, by file name, with the reason.
pub const WORKFLOWS_OUTSIDE_THE_COMMAND: &[(&str, &str)] = &[(
    "brew-install-branch.yml",
    "it runs only on a pull request that changes packaging or the formula, builds the release binary and installs it through Homebrew; that is not a step to run on a person's machine before each commit, and release.yml runs the installed-copy check on a tag",
)];

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

/// True when the workflow's `on:` names `pull_request`, inline or in its block.
pub fn runs_on_pull_request(text: &str) -> bool {
    let mut in_on = false;
    for line in text.lines() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let top_level = indent(line) == 0;
        if top_level {
            in_on = false;
            if let Some(rest) = line.strip_prefix("on:") {
                if rest.contains("pull_request") {
                    return true;
                }
                in_on = true;
            }
            continue;
        }
        if in_on && line.contains("pull_request") {
            return true;
        }
    }
    false
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
/// invoke; empty when they agree.
pub fn coverage(steps: &[&str], workflows: &[Workflow]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut invoked: Vec<String> = Vec::new();
    for workflow in workflows {
        let file = workflow.path.rsplit('/').next().unwrap_or(&workflow.path);
        let outside = WORKFLOWS_OUTSIDE_THE_COMMAND
            .iter()
            .any(|(name, _)| *name == file);
        let checked = runs_on_pull_request(&workflow.text) && !outside;
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
            if !checked {
                continue;
            }
            if let Some(call) = direct_gate_call(&line) {
                let allowed = DIRECT_CALLS_ALLOWED
                    .iter()
                    .any(|(name, needle, _)| *name == file && call.contains(needle));
                if !allowed {
                    problems.push(format!(
                        "{}:{line_no} runs `{call}` directly in a workflow a pull request triggers; call the command's step instead (`cargo xtask ci --step <name>`), so the check runs what a local run runs",
                        workflow.path
                    ));
                }
            }
        }
    }
    for step in steps {
        if !invoked.iter().any(|name| name == step) {
            problems.push(format!(
                "the step '{step}' is carried by `cargo xtask ci` and invoked by no workflow, so a local run runs it and CI does not; add `cargo xtask ci --step {step}` to the job that should run it, or remove the step"
            ));
        }
    }
    problems
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
    fn the_trigger_reader_sees_pull_request_inline_and_in_a_block_and_not_elsewhere() {
        assert!(runs_on_pull_request(PR));
        assert!(runs_on_pull_request("on: [push, pull_request]\njobs:\n"));
        assert!(!runs_on_pull_request(
            "on:\n  push:\n    tags: ['v*']\njobs:\n  a:\n    if: github.event_name == 'pull_request'\n"
        ));
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
        ];
        assert_eq!(coverage(&["one"], &files), Vec::<String>::new());
    }

    #[test]
    fn a_workflow_named_outside_the_command_may_call_scripts_and_no_other_may() {
        let direct = format!("{PR}      - run: scripts/package.sh \"$TAG\" aarch64-apple-darwin\n");
        let named = workflow("brew-install-branch.yml", &direct);
        let other = workflow("brew-install-other.yml", &direct);
        let carried = workflow(
            "a.yml",
            &format!("{PR}      - run: cargo xtask ci --step one\n"),
        );
        assert_eq!(coverage(&["one"], &[named, carried]), Vec::<String>::new());
        let carried = workflow(
            "a.yml",
            &format!("{PR}      - run: cargo xtask ci --step one\n"),
        );
        assert_eq!(coverage(&["one"], &[other, carried]).len(), 1);
    }

    #[test]
    fn the_allowed_direct_calls_pass_only_in_the_file_they_are_allowed_in() {
        let fetch =
            "      - run: |\n          engine=$(scripts/fetch-duckdb-cli.sh \"$host\" dir)\n";
        let in_test = workflow(
            "test.yml",
            &format!("{PR}{fetch}      - run: cargo xtask ci --step one\n"),
        );
        assert_eq!(coverage(&["one"], &[in_test]), Vec::<String>::new());
        let elsewhere = workflow(
            "lint.yml",
            &format!("{PR}{fetch}      - run: cargo xtask ci --step one\n"),
        );
        assert_eq!(coverage(&["one"], &[elsewhere]).len(), 1);
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
