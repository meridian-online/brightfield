//! Which test targets a test step runs: all of them locally, a shard's share
//! of them in CI.
//!
//! The suite is two groups. The targets named on `.github/workflows/test.yml`'s
//! `serial-targets:` comment lines, with every crate's `--lib` unit tests, run
//! on one thread; every other target, with `--bins` and the doctests, runs on
//! cargo's default threads. That file's comment says why each target is on
//! the list, and this module reads the list from there so the comment and the
//! runs cannot disagree.
//!
//! Locally a test step runs its whole group. In CI each shard of `test.yml`'s
//! matrix names itself in `SHARD_NAME`, `SHARD_INDEX`, `SHARD_TOTAL` and
//! `ONE_THREAD_SHARDS`, and the step runs that shard's share: the first
//! `ONE_THREAD_SHARDS` shards split the one-thread group and the rest split
//! the other, each item placed, longest first, on the part with the fewest
//! seconds so far, using the `target-seconds:` lines of the same file. Every
//! shard computes the same packing, so each target runs on exactly one shard.
//! The pin-staleness and conformance steps run on the last default shard.

use std::collections::{BTreeMap, HashMap};

/// The two groups a test step can run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    OneThread,
    Default,
}

impl Role {
    fn word(self) -> &'static str {
        match self {
            Role::OneThread => "one-thread",
            Role::Default => "default",
        }
    }
}

/// A CI shard, as `test.yml`'s matrix names it.
#[derive(Debug, PartialEq)]
pub struct Shard {
    pub name: String,
    pub index: usize,
    pub total: usize,
    pub one_thread: usize,
}

/// One group's items for this run, and the packing it came from.
#[derive(Debug, PartialEq)]
pub struct Part {
    pub items: Vec<String>,
    /// The lines that say how the group was split, for the step's log.
    pub report: Vec<String>,
}

impl Part {
    /// The `cargo test` target flags for this part's items, `--doc` aside.
    pub fn cargo_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        for item in &self.items {
            match item.as_str() {
                "@lib" => args.push("--lib".to_owned()),
                "@bins" => args.push("--bins".to_owned()),
                "@doc" => {}
                name => {
                    args.push("--test".to_owned());
                    args.push(name.to_owned());
                }
            }
        }
        args
    }

    /// True when this part holds the doctests.
    pub fn doc(&self) -> bool {
        self.items.iter().any(|item| item == "@doc")
    }
}

/// What this run's test steps run: a part of either group or both, and
/// whether the pin-staleness and conformance steps run here.
#[derive(Debug, PartialEq)]
pub struct Selection {
    pub one_thread: Option<Part>,
    pub default: Option<Part>,
    pub protocols: bool,
}

/// The two lists `test.yml` carries in its comments: the one-thread target
/// names, and seconds per item.
pub fn read_lists(test_yml: &str) -> Result<(Vec<String>, HashMap<String, f64>), String> {
    let words = |prefix: &str| -> Vec<String> {
        test_yml
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix(prefix))
            .flat_map(str::split_whitespace)
            .map(str::to_owned)
            .collect()
    };
    let serial = words("# serial-targets:");
    if serial.is_empty() {
        return Err("no line of .github/workflows/test.yml begins 'serial-targets:'. The GPU-bound targets are listed in those lines; without them no target would run on one thread and every target would run in parallel".to_owned());
    }
    let mut seconds = HashMap::new();
    for word in words("# target-seconds:") {
        let (name, value) = word.split_once('=').unwrap_or((&word, ""));
        let value: f64 = value.parse().map_err(|_| {
            format!("'{word}' in a line of .github/workflows/test.yml that begins 'target-seconds:' is not name=seconds")
        })?;
        seconds.insert(name.to_owned(), value);
    }
    Ok((serial, seconds))
}

/// Test target names in `cargo metadata` output, each with how many packages
/// carry a test target of that name.
pub fn test_targets(metadata_json: &str) -> Result<BTreeMap<String, usize>, String> {
    let metadata: serde_json::Value = serde_json::from_str(metadata_json)
        .map_err(|e| format!("cargo metadata did not print JSON: {e}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata printed no packages array")?;
    let mut targets = BTreeMap::new();
    for package in packages {
        for target in package["targets"].as_array().into_iter().flatten() {
            let is_test = target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|k| k == "test"));
            if let (true, Some(name)) = (is_test, target["name"].as_str()) {
                *targets.entry(name.to_owned()).or_insert(0) += 1;
            }
        }
    }
    Ok(targets)
}

/// The shard this process is, from the environment `test.yml` sets, or `None`
/// when none of the four variables is set, which is a local run.
pub fn shard_from_env(get: impl Fn(&str) -> Option<String>) -> Result<Option<Shard>, String> {
    const VARS: [&str; 4] = ["SHARD_NAME", "SHARD_INDEX", "SHARD_TOTAL", "ONE_THREAD_SHARDS"];
    let values: Vec<Option<String>> = VARS.iter().map(|v| get(v)).collect();
    if values.iter().all(Option::is_none) {
        return Ok(None);
    }
    let missing: Vec<&str> = VARS
        .iter()
        .zip(&values)
        .filter(|(_, value)| value.is_none())
        .map(|(var, _)| *var)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "a shard is named by {} together, and {} is not set",
            VARS.join(", "),
            missing.join(", ")
        ));
    }
    let number = |at: usize| -> Result<usize, String> {
        let raw = values[at].as_deref().unwrap_or_default();
        raw.parse()
            .map_err(|_| format!("{} is '{raw}', not a whole number", VARS[at]))
    };
    Ok(Some(Shard {
        name: values[0].clone().unwrap_or_default(),
        index: number(1)?,
        total: number(2)?,
        one_thread: number(3)?,
    }))
}

/// Split `items` over `parts`, longest first, each onto the part with the
/// fewest seconds so far (the lower part on a tie); `preload` seconds start on
/// the last part. An item with no figure counts as 2 seconds.
pub fn pack(
    items: &[String],
    parts: usize,
    preload: f64,
    seconds: &HashMap<String, f64>,
) -> (Vec<Vec<String>>, Vec<f64>) {
    let weight = |item: &String| seconds.get(item).copied().unwrap_or(2.0);
    let mut loads = vec![0.0; parts];
    if let Some(last) = loads.last_mut() {
        *last += preload;
    }
    let mut packed = vec![Vec::new(); parts];
    let mut order: Vec<&String> = items.iter().collect();
    order.sort_by(|a, b| weight(b).total_cmp(&weight(a)).then_with(|| a.cmp(b)));
    for item in order {
        let mut best = 0;
        for part in 1..parts {
            if loads[part] < loads[best] {
                best = part;
            }
        }
        packed[best].push(item.clone());
        loads[best] += weight(item);
    }
    (packed, loads)
}

/// This run's selection: both whole groups locally, a shard's share in CI.
pub fn select(
    serial: &[String],
    seconds: &HashMap<String, f64>,
    targets: &BTreeMap<String, usize>,
    shard: Option<&Shard>,
) -> Result<Selection, String> {
    for name in serial {
        let count = targets.get(name).copied().unwrap_or(0);
        if count != 1 {
            return Err(format!(
                "'{name}' is named in a serial-targets line and matches {count} cargo test targets; it was renamed, deleted, or shares its name with another crate's target, which a --test name cannot tell apart"
            ));
        }
    }
    let mut one_thread_items: Vec<String> = targets
        .keys()
        .filter(|name| serial.contains(name))
        .cloned()
        .collect();
    one_thread_items.push("@lib".to_owned());
    let mut default_items: Vec<String> = targets
        .keys()
        .filter(|name| !serial.contains(name))
        .cloned()
        .collect();
    default_items.extend(["@bins".to_owned(), "@doc".to_owned()]);
    let protocols_seconds = seconds.get("@protocols").copied().unwrap_or(2.0);

    let Some(shard) = shard else {
        let whole = |role: Role, items: Vec<String>| Part {
            report: vec![format!(
                "local run: the whole {} group, {} item(s)",
                role.word(),
                items.len()
            )],
            items,
        };
        return Ok(Selection {
            one_thread: Some(whole(Role::OneThread, one_thread_items)),
            default: Some(whole(Role::Default, default_items)),
            protocols: true,
        });
    };

    let default_shards = shard.total.saturating_sub(shard.one_thread);
    if shard.one_thread < 1 || default_shards < 1 {
        return Err(format!(
            "the matrix has {} shard(s) and ONE_THREAD_SHARDS is {}; each group needs a shard, or its targets would run nowhere",
            shard.total, shard.one_thread
        ));
    }
    if shard.index >= shard.total {
        return Err(format!(
            "SHARD_INDEX is {} and SHARD_TOTAL is {}; a shard's index counts from 0 and is less than the total",
            shard.index, shard.total
        ));
    }
    let (role, part, parts, items, preload) = if shard.index < shard.one_thread {
        (Role::OneThread, shard.index, shard.one_thread, one_thread_items, 0.0)
    } else {
        let part = shard.index - shard.one_thread;
        (Role::Default, part, default_shards, default_items, protocols_seconds)
    };
    if !shard.name.starts_with(role.word()) {
        return Err(format!(
            "shard {} of {} is named '{}' and runs the {} group; the first {} shards in the matrix run the one-thread group and the rest run the other",
            shard.index,
            shard.total,
            shard.name,
            role.word(),
            shard.one_thread
        ));
    }
    let (packed, loads) = pack(&items, parts, preload, seconds);
    let mut report = Vec::new();
    for (number, (members, load)) in packed.iter().zip(&loads).enumerate() {
        let here = if number == part { "  <- this shard" } else { "" };
        report.push(format!(
            "{} part {} of {parts}: {} item(s), about {load:.0} s{here}",
            role.word(),
            number + 1,
            members.len()
        ));
    }
    let mine = packed[part].clone();
    report.push(format!("this shard runs, in {} part {} of {parts}:", role.word(), part + 1));
    report.push(format!("  {}", mine.join(" ")));
    let chosen = Some(Part {
        items: mine,
        report,
    });
    Ok(match role {
        Role::OneThread => Selection {
            one_thread: chosen,
            default: None,
            protocols: false,
        },
        Role::Default => Selection {
            one_thread: None,
            default: chosen,
            protocols: part == parts - 1,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn targets(names: &[&str]) -> BTreeMap<String, usize> {
        names.iter().map(|n| ((*n).to_owned(), 1)).collect()
    }

    fn shard(name: &str, index: usize) -> Shard {
        Shard {
            name: name.to_owned(),
            index,
            total: 5,
            one_thread: 2,
        }
    }

    const YML: &str = "    # serial-targets: gpu_a gpu_b\n    # serial-targets: gpu_c\n    # target-seconds: @protocols=30 @lib=38\n    # target-seconds: gpu_a=100 plain_a=50 plain_b=40\n";

    #[test]
    fn the_lists_are_read_from_the_comment_lines() {
        let (serial, seconds) = read_lists(YML).unwrap();
        assert_eq!(serial, ["gpu_a", "gpu_b", "gpu_c"]);
        assert_eq!(seconds["@protocols"], 30.0);
        assert_eq!(seconds["plain_b"], 40.0);
    }

    #[test]
    fn an_empty_serial_list_and_a_malformed_figure_are_refused() {
        assert!(read_lists("# target-seconds: a=1\n").unwrap_err().contains("serial-targets"));
        assert!(read_lists("# serial-targets: a\n# target-seconds: a=x\n")
            .unwrap_err()
            .contains("'a=x'"));
    }

    #[test]
    fn test_targets_counts_names_across_packages_and_ignores_other_kinds() {
        let json = r#"{"packages":[
            {"targets":[{"kind":["lib"],"name":"lib_a"},{"kind":["test"],"name":"shared"},{"kind":["test"],"name":"only_a"}]},
            {"targets":[{"kind":["bin"],"name":"bin_b"},{"kind":["test"],"name":"shared"}]}]}"#;
        let found = test_targets(json).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found["shared"], 2);
        assert_eq!(found["only_a"], 1);
    }

    #[test]
    fn a_partial_shard_environment_is_refused_and_none_is_a_local_run() {
        assert_eq!(shard_from_env(|_| None).unwrap(), None);
        let partial = shard_from_env(|v| (v == "SHARD_NAME").then(|| "default-1".to_owned()));
        assert!(partial.unwrap_err().contains("SHARD_INDEX, SHARD_TOTAL, ONE_THREAD_SHARDS is not set"));
        let full = shard_from_env(|v| {
            Some(match v {
                "SHARD_NAME" => "default-1".to_owned(),
                "SHARD_INDEX" => "2".to_owned(),
                "SHARD_TOTAL" => "5".to_owned(),
                _ => "2".to_owned(),
            })
        });
        assert_eq!(full.unwrap(), Some(shard("default-1", 2)));
    }

    #[test]
    fn packing_places_the_longest_first_on_the_lightest_part() {
        let seconds: HashMap<String, f64> =
            [("a", 10.0), ("b", 8.0), ("c", 3.0)].iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
        let (parts, loads) = pack(&strings(&["c", "a", "b", "d"]), 2, 0.0, &seconds);
        assert_eq!(parts, [strings(&["a", "d"]), strings(&["b", "c"])]);
        assert_eq!(loads, [12.0, 11.0]);
        let (parts, _) = pack(&strings(&["a", "b"]), 2, 20.0, &seconds);
        assert_eq!(parts, [strings(&["a", "b"]), Vec::<String>::new()], "the preload starts on the last part");
    }

    #[test]
    fn a_local_run_selects_both_whole_groups_and_the_protocol_steps() {
        let (serial, seconds) = read_lists(YML).unwrap();
        let found = targets(&["gpu_a", "gpu_b", "gpu_c", "plain_a", "plain_b"]);
        let sel = select(&serial, &seconds, &found, None).unwrap();
        let one = sel.one_thread.unwrap();
        assert_eq!(one.items, strings(&["gpu_a", "gpu_b", "gpu_c", "@lib"]));
        assert_eq!(
            one.cargo_args(),
            strings(&["--test", "gpu_a", "--test", "gpu_b", "--test", "gpu_c", "--lib"])
        );
        let other = sel.default.unwrap();
        assert_eq!(other.items, strings(&["plain_a", "plain_b", "@bins", "@doc"]));
        assert!(other.doc());
        assert!(sel.protocols);
    }

    #[test]
    fn every_item_runs_on_exactly_one_shard_and_only_the_last_default_shard_runs_the_protocols() {
        let (serial, seconds) = read_lists(YML).unwrap();
        let found = targets(&["gpu_a", "gpu_b", "gpu_c", "plain_a", "plain_b", "plain_c"]);
        let names = ["one-thread-1", "one-thread-2", "default-1", "default-2", "default-3"];
        let mut seen: Vec<String> = Vec::new();
        let mut protocols = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let sel = select(&serial, &seconds, &found, Some(&shard(name, index))).unwrap();
            assert!(sel.one_thread.is_some() != sel.default.is_some(), "a shard runs one group");
            if index < 2 {
                assert!(sel.one_thread.is_some(), "{name} runs the one-thread group");
            }
            seen.extend(sel.one_thread.into_iter().chain(sel.default).flat_map(|p| p.items));
            protocols.push(sel.protocols);
        }
        let mut expected = strings(&["gpu_a", "gpu_b", "gpu_c", "@lib", "plain_a", "plain_b", "plain_c", "@bins", "@doc"]);
        seen.sort();
        expected.sort();
        assert_eq!(seen, expected);
        assert_eq!(protocols, [false, false, false, false, true]);
    }

    #[test]
    fn a_serial_name_that_is_not_one_target_and_a_misnamed_shard_are_refused() {
        let (serial, seconds) = read_lists(YML).unwrap();
        let missing = targets(&["gpu_a", "gpu_b"]);
        assert!(select(&serial, &seconds, &missing, None).unwrap_err().contains("'gpu_c'"));
        let mut twice = targets(&["gpu_a", "gpu_b", "gpu_c"]);
        twice.insert("gpu_b".to_owned(), 2);
        assert!(select(&serial, &seconds, &twice, None).unwrap_err().contains("matches 2"));
        let found = targets(&["gpu_a", "gpu_b", "gpu_c"]);
        let wrong = select(&serial, &seconds, &found, Some(&shard("default-1", 0)));
        assert!(wrong.unwrap_err().contains("runs the one-thread group"));
    }
}
