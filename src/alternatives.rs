//! A stand-in for a program that is not installed, made of tools every system has
//! (`tree` → `find DIR -maxdepth 2 | sort`). The table is `data/alternatives.toml`.
//!
//! An alternative replaces the whole command, so it is only offered for a command that runs just
//! the missing program: with `tree x | grep y` there would be no telling which part to replace
//! without changing what was asked. And it is only offered when everything the alternative runs
//! is installed.

use crate::system;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
struct Row {
    command: String,
}

type Table = HashMap<String, Row>;

fn parse_table(text: &str) -> Result<Table> {
    toml::from_str(text).context("data/alternatives.toml")
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        parse_table(include_str!("../data/alternatives.toml")).expect("data/alternatives.toml is valid")
    })
}

/// The programs the table has a stand-in for, sorted.
#[cfg(test)]
fn programs() -> Vec<String> {
    let mut p: Vec<String> = table().keys().cloned().collect();
    p.sort();
    p
}

/// The alternative to `command`, whose missing programs are `missing`, or `None` when there is
/// none that applies and could run here.
pub fn alternative_for(command: &str, missing: &[String]) -> Option<String> {
    alternative_for_with(command, missing, table(), |bin| {
        system::is_builtin(bin) || system::which(bin).is_some()
    })
}

fn alternative_for_with(
    command: &str,
    missing: &[String],
    table: &Table,
    installed: impl Fn(&str) -> bool,
) -> Option<String> {
    // the command must run exactly one program, and it is the one that is missing
    let programs = system::binaries(command);
    let [only] = programs.as_slice() else {
        return None;
    };
    if !missing.contains(only) {
        return None;
    }
    let alternative = &table.get(only)?.command;
    // everything the alternative runs has to be there, or it is no alternative
    system::binaries(alternative)
        .iter()
        .all(|bin| installed(bin))
        .then(|| alternative.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_installed(_: &str) -> bool {
        true
    }

    fn alt(command: &str, missing: &[&str]) -> Option<String> {
        let missing: Vec<String> = missing.iter().map(|m| m.to_string()).collect();
        alternative_for_with(command, &missing, table(), all_installed)
    }

    // the cases the plan names
    #[test]
    fn tree_becomes_find() {
        assert_eq!(
            alt("tree -L 2 {{dir}}", &["tree"]).as_deref(),
            Some("find {{dir}} -maxdepth 2 | sort")
        );
    }

    #[test]
    fn the_rest_of_the_plans_examples() {
        assert_eq!(
            alt("ncdu {{dir}}", &["ncdu"]).as_deref(),
            Some("du -h -d1 {{dir}} | sort -h")
        );
        assert_eq!(
            alt("rg {{pattern}}", &["rg"]).as_deref(),
            Some("grep -rn {{pattern}} {{dir}}")
        );
        assert_eq!(
            alt("fd -e rs", &["fd"]).as_deref(),
            Some("find {{dir}} -name {{pattern}}")
        );
        assert_eq!(
            alt("jq . {{file}}", &["jq"]).as_deref(),
            Some("python3 -m json.tool {{file}}")
        );
        assert_eq!(alt("htop", &["htop"]).as_deref(), Some("top"));
    }

    #[test]
    fn only_for_a_command_that_runs_just_the_missing_program() {
        // a pipeline: there is no telling which part to replace
        assert_eq!(alt("tree {{dir}} | grep x", &["tree"]), None);
        assert_eq!(alt("cd /tmp && tree", &["tree"]), None);
        // sudo and assignments are not programs: the command still runs one
        assert!(alt("sudo ncdu /", &["ncdu"]).is_some());
        assert!(alt("LC_ALL=C tree x", &["tree"]).is_some());
        // the missing program is not the one the command runs
        assert_eq!(alt("tree x", &["jq"]), None);
        // nothing missing: nothing to replace
        assert_eq!(alt("tree x", &[]), None);
    }

    #[test]
    fn none_when_the_table_has_nothing_for_it() {
        assert_eq!(alt("unknown-tool x", &["unknown-tool"]), None);
    }

    /// An alternative that cannot run here is not offered.
    #[test]
    fn the_alternatives_own_programs_must_be_installed() {
        let missing = vec!["tree".to_string()];
        let without = |gone: &'static str| move |bin: &str| bin != gone;
        assert!(alternative_for_with("tree x", &missing, table(), without("grep")).is_some());
        assert_eq!(
            alternative_for_with("tree x", &missing, table(), without("find")),
            None
        );
        assert_eq!(
            alternative_for_with("tree x", &missing, table(), without("sort")),
            None,
            "both halves of the pipe"
        );
        // python3 for the json stand-in
        let jq = vec!["jq".to_string()];
        assert_eq!(
            alternative_for_with("jq .", &jq, table(), without("python3")),
            None
        );
    }

    #[test]
    fn every_alternative_is_made_of_base_tools() {
        let base = [
            "find", "sort", "du", "df", "top", "ps", "grep", "cat", "ls", "python3", "sed", "ip", "ss",
            "getent", "curl",
        ];
        for program in programs() {
            let command = &table()[&program].command;
            let used = system::binaries(command);
            assert!(!used.is_empty(), "{program}: {command:?} runs nothing");
            for bin in &used {
                assert!(
                    base.contains(&bin.as_str()),
                    "{program}: {bin} is not a base tool ({command:?})"
                );
            }
            // it must not need the program it stands in for
            assert!(!used.contains(&program), "{program} is its own alternative");
        }
    }

    #[test]
    fn the_table_parses() {
        assert!(table().len() >= 20);
        assert!(parse_table("[x]\ncommand = \"ls\"\n").is_ok());
        assert!(parse_table("[x]\nnope = 1\n").is_err(), "a row needs its command");
    }
}
