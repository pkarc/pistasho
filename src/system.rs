//! What the machine has: is a command's binary installed?
//!
//! `binaries` reads a shell command and lists the programs it would run; `which` looks one up in
//! `PATH` without spawning anything; `missing_binaries` combines the two. Used to mark candidates
//! as available or missing (the install flow of task 2.6 builds on it).
//!
//! The parser is not a shell: it is a tokenizer that knows quotes, `$(...)` and backticks,
//! pipes and separators, redirections, assignments, the control keywords, and the wrappers that
//! just run another command (`sudo`, `env`, `xargs`…). That is enough to name the programs in
//! the commands of tldr, your cheats and your history.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- PATH lookup

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Looks `bin` up in the directories of `path` (`:`-separated, like `$PATH`). A name with a `/`
/// is checked as it is. Nothing is executed.
pub fn which_in(bin: &str, path: &OsStr) -> Option<PathBuf> {
    if bin.contains('/') {
        return is_executable(Path::new(bin)).then(|| PathBuf::from(bin));
    }
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(bin))
        .find(|candidate| is_executable(candidate))
}

/// `which_in` over the real `$PATH`. Not used yet: the install flow (task 2.6) checks with it
/// that the program is there once the package manager is done.
#[allow(dead_code)]
pub fn which(bin: &str) -> Option<PathBuf> {
    which_in(bin, &std::env::var_os("PATH")?)
}

/// Commands the shell itself provides, so they are never "missing" even if no file has the name.
/// Aliases and functions of the user's shell are not visible from here.
const BUILTINS: &[&str] = &[
    ":", ".", "alias", "autoload", "bg", "bindkey", "builtin", "cd", "command", "declare", "dirs", "disown",
    "echo", "emulate", "eval", "exec", "exit", "export", "false", "fc", "fg", "getopts", "hash", "history",
    "jobs", "let", "local", "logout", "popd", "print", "printf", "pushd", "pwd", "read", "rehash", "return",
    "set", "setopt", "shift", "source", "suspend", "test", "times", "trap", "true", "type", "typeset",
    "ulimit", "umask", "unalias", "unset", "unsetopt", "wait", "whence", "zle", "zstyle",
];

pub fn is_builtin(bin: &str) -> bool {
    BUILTINS.contains(&bin)
}

/// The programs `cmd` runs that are neither shell builtins nor found in `path`.
pub fn missing_binaries_in(cmd: &str, path: &OsStr) -> Vec<String> {
    binaries(cmd)
        .into_iter()
        .filter(|b| !is_builtin(b) && which_in(b, path).is_none())
        .collect()
}

/// `missing_binaries_in` over the real `$PATH`.
pub fn missing_binaries(cmd: &str) -> Vec<String> {
    match std::env::var_os("PATH") {
        Some(path) => missing_binaries_in(cmd, &path),
        None => binaries(cmd).into_iter().filter(|b| !is_builtin(b)).collect(),
    }
}

// ---------------------------------------------------------------- reading a command

#[derive(Debug, PartialEq)]
enum Tok {
    /// A word, the commands of the `$(...)` / backtick substitutions inside it, and where it sits
    /// in the command (char indices, quotes included).
    Word {
        text: String,
        subs: Vec<String>,
        span: (usize, usize),
    },
    /// `|`, `||`, `&&`, `&`, `;`, newline, `(`, `)`: the next word starts a command.
    Sep,
    /// `>`, `>>`, `<`, `<<`, `2>&1`…: the next word is a target, not a command.
    Redir,
}

/// The text up to the `)` that closes a `$(`, and the index after it.
fn take_paren(chars: &[char], mut i: usize) -> (String, usize) {
    let start = i;
    let mut depth = 1;
    while i < chars.len() {
        match chars[i] {
            '\'' | '"' => {
                let quote = chars[i];
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '\\' => i += 1,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (chars[start..i].iter().collect(), i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (chars[start..].iter().collect(), chars.len())
}

/// The text up to the closing backtick, and the index after it.
fn take_backtick(chars: &[char], mut i: usize) -> (String, usize) {
    let start = i;
    while i < chars.len() && chars[i] != '`' {
        if chars[i] == '\\' {
            i += 1;
        }
        i += 1;
    }
    (
        chars[start..i.min(chars.len())].iter().collect(),
        (i + 1).min(chars.len()),
    )
}

fn tokenize(cmd: &str) -> Vec<Tok> {
    let chars: Vec<char> = cmd.chars().collect();
    let n = chars.len();
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut subs: Vec<String> = Vec::new();
    let mut in_word = false;
    let mut i = 0;

    let mut word_start = 0;

    let flush = |toks: &mut Vec<Tok>,
                 cur: &mut String,
                 subs: &mut Vec<String>,
                 in_word: &mut bool,
                 start: usize,
                 end: usize| {
        if *in_word {
            toks.push(Tok::Word {
                text: std::mem::take(cur),
                subs: std::mem::take(subs),
                span: (start, end.min(n)),
            });
            *in_word = false;
        }
    };

    while i < n {
        if !in_word {
            word_start = i;
        }
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\\' => {
                // `\<newline>` continues the line; anything else is the next char, literally
                match next {
                    Some('\n') => {}
                    Some(escaped) => {
                        cur.push(escaped);
                        in_word = true;
                    }
                    None => {}
                }
                i += 2;
                continue;
            }
            '\'' => {
                in_word = true;
                i += 1;
                while i < n && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
                i += 1;
                continue;
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < n && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < n {
                        cur.push(chars[i + 1]);
                        i += 2;
                    } else if chars[i] == '$' && chars.get(i + 1) == Some(&'(') {
                        let (inner, after) = take_paren(&chars, i + 2);
                        subs.push(inner);
                        cur.push_str("$SUB");
                        i = after;
                    } else if chars[i] == '`' {
                        let (inner, after) = take_backtick(&chars, i + 1);
                        subs.push(inner);
                        cur.push_str("$SUB");
                        i = after;
                    } else {
                        cur.push(chars[i]);
                        i += 1;
                    }
                }
                i += 1;
                continue;
            }
            '{' if next == Some('{') => {
                // a `{{placeholder}}` is one piece, whatever it contains (`{{name|id}}`,
                // `{{[logs|container logs]}}`): its `|` and spaces are not shell syntax
                in_word = true;
                let rest: String = chars[i..].iter().collect();
                match rest.find("}}") {
                    Some(end) => {
                        let len = rest[..end + 2].chars().count();
                        cur.extend(&chars[i..i + len]);
                        i += len;
                    }
                    None => {
                        cur.push(c);
                        i += 1;
                    }
                }
                continue;
            }
            '$' if next == Some('(') => {
                in_word = true;
                let (inner, after) = take_paren(&chars, i + 2);
                subs.push(inner);
                cur.push_str("$SUB");
                i = after;
                continue;
            }
            '`' => {
                in_word = true;
                let (inner, after) = take_backtick(&chars, i + 1);
                subs.push(inner);
                cur.push_str("$SUB");
                i = after;
                continue;
            }
            '#' if !in_word => {
                // a comment runs to the end of the line
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            ' ' | '\t' => flush(&mut toks, &mut cur, &mut subs, &mut in_word, word_start, i),
            '\n' | ';' | '(' | ')' => {
                flush(&mut toks, &mut cur, &mut subs, &mut in_word, word_start, i);
                toks.push(Tok::Sep);
            }
            '|' | '&' => {
                flush(&mut toks, &mut cur, &mut subs, &mut in_word, word_start, i);
                if c == '&' && next == Some('>') {
                    // `&>file` redirects both streams
                    toks.push(Tok::Redir);
                    i += 1;
                    while chars.get(i + 1) == Some(&'>') {
                        i += 1;
                    }
                } else {
                    // `||`, `&&`, `|&`
                    if matches!(next, Some('|' | '&')) {
                        i += 1;
                    }
                    toks.push(Tok::Sep);
                }
            }
            '<' | '>' => {
                // a file descriptor number in front (`2>`) belongs to the redirection
                if in_word && cur.chars().all(|d| d.is_ascii_digit()) {
                    cur.clear();
                    subs.clear();
                    in_word = false;
                } else {
                    flush(&mut toks, &mut cur, &mut subs, &mut in_word, word_start, i);
                }
                while matches!(chars.get(i + 1), Some('<' | '>')) {
                    i += 1;
                }
                if chars.get(i + 1) == Some(&'&') {
                    i += 1;
                }
                toks.push(Tok::Redir);
            }
            _ => {
                cur.push(c);
                in_word = true;
            }
        }
        i += 1;
    }
    flush(&mut toks, &mut cur, &mut subs, &mut in_word, word_start, i);
    toks
}

/// `NAME=value`
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Words that open or close a construct and are followed by a command (`then ls`).
const SKIP_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "esac", "!", "{", "}", "[[", "]]",
    "[",
];

/// Words that start a construct with no command in the rest of the segment (`for f in a b`).
const HEAD_KEYWORDS: &[&str] = &["for", "case", "select", "function"];

/// A command that only runs another one: the program to look for is its payload.
struct Wrapper {
    /// Options that take the next word as their value (`sudo -u user`).
    with_arg: &'static [&'static str],
    /// `VAR=value` words before the payload (`env FOO=1 make`).
    assignments: bool,
}

fn wrapper(word: &str) -> Option<Wrapper> {
    // `/usr/bin/env` is `env`
    let name = word.rsplit('/').next().unwrap_or(word);
    let (with_arg, assignments): (&'static [&'static str], bool) = match name {
        "sudo" => (&["-u", "-g", "-h", "-p", "-C", "-D", "-R", "-T", "-U"], true),
        "doas" => (&["-u", "-C"], false),
        "env" => (&["-u", "-C", "-S"], true),
        "nice" => (&["-n"], false),
        "ionice" => (&["-c", "-n", "-p"], false),
        "xargs" => (
            &["-a", "-d", "-E", "-e", "-I", "-i", "-L", "-l", "-n", "-P", "-s"],
            false,
        ),
        "stdbuf" => (&["-i", "-o", "-e"], false),
        "nohup" | "time" | "exec" | "command" | "builtin" | "setsid" => (&[], false),
        _ => return None,
    };
    Some(Wrapper {
        with_arg,
        assignments,
    })
}

/// Whether a word in command position names something to look up: not a `{{placeholder}}`, a
/// variable, a history expansion (`!!`), a relative path, a glob or an option.
fn names_a_program(word: &str) -> bool {
    word.chars().any(char::is_alphanumeric)
        && !word.chars().any(char::is_control)
        && !word.contains("{{")
        && !word.contains("$SUB")
        && !word.starts_with(['$', '!', '~', '-', '.', '*', '%', '@', '='])
}

/// A program a command runs, and where its word sits in the command (char indices, quotes
/// included). Programs inside `$(...)` have no span: it would be relative to the inner text.
#[derive(Debug, PartialEq)]
pub struct Program {
    pub name: String,
    pub span: Option<(usize, usize)>,
}

/// The programs a shell command runs, in the order they appear, repeats included.
/// `sudo`, `env`, `xargs` and similar wrappers are skipped in favor of what they run.
pub fn programs(cmd: &str) -> Vec<Program> {
    let mut found: Vec<Program> = Vec::new();
    let mut add = |name: String, span: Option<(usize, usize)>| found.push(Program { name, span });

    let mut at_start = true;
    let mut no_command = false;
    let mut skip_target = false;
    let mut pending: Option<(Wrapper, bool, bool)> = None; // (wrapper, expecting an option value, options ended)

    for tok in tokenize(cmd) {
        let (text, subs, span) = match tok {
            Tok::Sep => {
                at_start = true;
                no_command = false;
                skip_target = false;
                pending = None;
                continue;
            }
            Tok::Redir => {
                skip_target = true;
                continue;
            }
            Tok::Word { text, subs, span } => (text, subs, span),
        };

        if skip_target {
            skip_target = false;
        } else if no_command {
            // inside `for f in …` or `case …`: nothing to run
        } else if let Some((w, expecting_value, options_ended)) = pending.as_mut() {
            if *expecting_value {
                *expecting_value = false;
            } else if text == "--" {
                *options_ended = true;
            } else if !*options_ended && text.len() > 1 && text.starts_with('-') {
                *expecting_value = w.with_arg.contains(&text.as_str());
            } else if w.assignments && is_assignment(&text) {
                // `env FOO=1 make`: keep looking for the program
            } else if text.contains("{{") {
                // `sudo {{[-u|--user]}} {{name}} tar x`: a placeholder here may be an option, its
                // value or the program itself; none of them can be named, so look past it
            } else {
                // the payload: handle it as the start of a command
                pending = None;
                at_start = true;
                handle_command_word(
                    &text,
                    span,
                    &mut at_start,
                    &mut no_command,
                    &mut pending,
                    &mut add,
                );
            }
        } else if at_start {
            handle_command_word(
                &text,
                span,
                &mut at_start,
                &mut no_command,
                &mut pending,
                &mut add,
            );
        }

        for sub in subs {
            for p in programs(&sub) {
                add(p.name, None);
            }
        }
    }
    found
}

/// The names of the programs a shell command runs, in the order they appear, without repeats.
///
/// `sudo tar czf a.tgz dir` → `tar`; `ss -tlnp | grep :80` → `ss`, `grep`;
/// `find . | xargs rg x` → `find`, `rg`; `FOO=1 make` → `make`.
pub fn binaries(cmd: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for p in programs(cmd) {
        if !names.contains(&p.name) {
            names.push(p.name);
        }
    }
    names
}

/// One simple command: the program, its arguments, and where it redirects to. Wrappers such as
/// `sudo` and `env` are skipped, so `sudo rm -rf /` is `rm` with `-rf` and `/`.
#[derive(Debug, Default, PartialEq)]
pub struct Simple {
    pub program: String,
    pub args: Vec<String>,
    /// Targets of `>`, `>>`, `<`, `2>`…, without the operators.
    pub redirects: Vec<String>,
}

/// The simple commands of a shell command, in order, including those inside `$(...)`.
pub fn commands(cmd: &str) -> Vec<Simple> {
    let starts: std::collections::HashMap<(usize, usize), String> = programs(cmd)
        .into_iter()
        .filter_map(|p| Some((p.span?, p.name)))
        .collect();
    let mut out: Vec<Simple> = Vec::new();
    let mut cur: Option<Simple> = None;
    let mut before: Vec<String> = Vec::new(); // redirects seen before the program (`> f cmd`)
    let mut redirecting = false;

    for tok in tokenize(cmd) {
        match tok {
            Tok::Sep => {
                out.extend(cur.take());
                before.clear();
                redirecting = false;
            }
            Tok::Redir => redirecting = true,
            Tok::Word { text, subs, span } => {
                if redirecting {
                    redirecting = false;
                    match cur.as_mut() {
                        Some(c) => c.redirects.push(text),
                        None => before.push(text),
                    }
                } else if let Some(name) = starts.get(&span) {
                    out.extend(cur.take());
                    cur = Some(Simple {
                        program: name.clone(),
                        args: Vec::new(),
                        redirects: std::mem::take(&mut before),
                    });
                } else if let Some(c) = cur.as_mut() {
                    c.args.push(text);
                }
                for sub in subs {
                    out.extend(commands(&sub));
                }
            }
        }
    }
    out.extend(cur);
    out
}

/// `cmd` with the program words named in `renames` replaced (`fd` → `fdfind`), wherever they
/// stand in the command. Programs inside `$(...)` are left alone.
pub fn rename_programs(cmd: &str, renames: &std::collections::HashMap<String, String>) -> String {
    let mut chars: Vec<char> = cmd.chars().collect();
    let mut spots: Vec<(usize, usize, &String)> = programs(cmd)
        .into_iter()
        .filter_map(|p| Some((p.span?, renames.get(&p.name)?)))
        .map(|((start, end), to)| (start, end, to))
        .collect();
    // from the end, so the earlier positions stay valid
    spots.sort_by_key(|spot| std::cmp::Reverse(spot.0));
    for (start, end, to) in spots {
        chars.splice(start..end, to.chars());
    }
    chars.into_iter().collect()
}

fn handle_command_word(
    text: &str,
    span: (usize, usize),
    at_start: &mut bool,
    no_command: &mut bool,
    pending: &mut Option<(Wrapper, bool, bool)>,
    add: &mut impl FnMut(String, Option<(usize, usize)>),
) {
    if is_assignment(text) || SKIP_KEYWORDS.contains(&text) {
        // still waiting for the command itself
    } else if HEAD_KEYWORDS.contains(&text) {
        *no_command = true;
        *at_start = false;
    } else if let Some(w) = wrapper(text) {
        *pending = Some((w, false, false));
        *at_start = false;
    } else {
        *at_start = false;
        if names_a_program(text) {
            add(text.to_string(), Some(span));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bins(cmd: &str) -> Vec<String> {
        binaries(cmd)
    }

    fn eq(cmd: &str, expected: &[&str]) {
        assert_eq!(bins(cmd), expected, "command: {cmd}");
    }

    // the four cases the plan names
    #[test]
    fn sudo_is_skipped() {
        eq("sudo tar czf a.tgz dir", &["tar"]);
    }

    #[test]
    fn pipes_name_every_program() {
        eq("ss -tlnp | grep :80", &["ss", "grep"]);
    }

    #[test]
    fn xargs_is_skipped_for_what_it_runs() {
        eq("find . | xargs rg x", &["find", "rg"]);
    }

    #[test]
    fn leading_assignments_are_skipped() {
        eq("FOO=1 make", &["make"]);
        eq("FOO=1 BAR=2 make -j4", &["make"]);
        eq("FOO=bar", &[]);
    }

    #[test]
    fn wrappers_and_their_options() {
        eq("sudo -u postgres psql -c 'select 1'", &["psql"]);
        eq("sudo -E -- make install", &["make"]);
        eq("sudo FOO=1 make", &["make"]);
        eq("env FOO=1 BAR=2 make -j4", &["make"]);
        eq("env -u HOME ls", &["ls"]);
        eq("/usr/bin/env python3 x.py", &["python3"]);
        eq("nohup sleep 5 &", &["sleep"]);
        eq("time -p make", &["make"]);
        eq("xargs -I {} mv {} dir", &["mv"]);
        eq("find . -name '*.o' -print0 | xargs -0 -n1 rm", &["find", "rm"]);
        eq("sudo env X=1 sudo tar x", &["tar"]);
        eq("nice -n 10 gzip big", &["gzip"]);
    }

    #[test]
    fn separators_start_new_commands() {
        eq("cd /tmp && ls -la", &["cd", "ls"]);
        eq("make; make install", &["make"]);
        eq("a || b && c", &["a", "b", "c"]);
        eq("sleep 1 & echo hi", &["sleep", "echo"]);
        eq("(cd /tmp && make)", &["cd", "make"]);
        eq("ls\nwc -l", &["ls", "wc"]);
        eq("ls |& wc", &["ls", "wc"]);
    }

    #[test]
    fn quotes_hide_separators() {
        eq("git commit -m \"fix: a | b && c\"", &["git"]);
        eq("echo 'a; b' | wc -c", &["echo", "wc"]);
        eq("echo a\\|b", &["echo"]);
    }

    #[test]
    fn command_substitution_is_read_too() {
        eq("echo $(date) | tr a b", &["echo", "date", "tr"]);
        eq("echo `whoami`", &["echo", "whoami"]);
        eq("echo \"today is $(date +%F)\"", &["echo", "date"]);
        eq("kill $(pidof {{name}})", &["kill", "pidof"]);
        eq("ls $(dirname $(which ls))", &["ls", "dirname", "which"]);
    }

    #[test]
    fn redirection_targets_are_not_commands() {
        eq("cat a | tee out.txt > /dev/null 2>&1", &["cat", "tee"]);
        eq("make &> build.log", &["make"]);
        eq("sort < in.txt > out.txt", &["sort"]);
        eq("cmd 2>err.txt", &["cmd"]);
        eq("diff <(sort a) <(sort b)", &["diff", "sort"]);
        eq("cat <<EOF", &["cat"]);
    }

    #[test]
    fn control_structures() {
        eq(
            "for f in *.jpeg; do mv -- \"$f\" \"${f%.jpeg}.jpg\"; done",
            &["mv"],
        );
        eq("if [ -f x ]; then echo y; else echo n; fi", &["echo"]);
        eq("while read l; do echo \"$l\"; done < file", &["read", "echo"]);
        eq("! grep -q x file", &["grep"]);
        eq("{ ls; wc; }", &["ls", "wc"]);
    }

    #[test]
    fn placeholders_variables_and_relative_paths_are_not_programs() {
        eq("{{command}} {{!^|!$|!*}}", &[]);
        eq("sudo !!", &[]);
        eq("$EDITOR file", &[]);
        eq("./deploy.sh", &[]);
        eq("../bin/run", &[]);
        eq("~/bin/tool", &[]);
        eq("docker run -it --rm {{image}}", &["docker"]);
        eq("{{tool}} --help", &[]);
    }

    /// Found by running the parser over every tldr command: placeholders with a `|` or spaces
    /// inside were cut up as if they were pipes, producing programs like `id}}`.
    #[test]
    fn placeholders_are_never_cut_up() {
        eq("docker {{[logs|container logs]}} {{name|id}}", &["docker"]);
        eq(
            "kubectl get {{pods|services}} | grep {{text}}",
            &["kubectl", "grep"],
        );
        eq("{{a b|c}} --help", &[]);
        eq("ssh {{user}}@{{host}} {{command; other}}", &["ssh"]);
        eq("sudo {{[-u|--user]}} {{name}} tar x", &["tar"]);
        // an unclosed `{{` is plain text
        eq("echo {{oops | wc", &["echo", "wc"]);
        // braces that are not placeholders still work as shell syntax
        eq("{ ls; wc; }", &["ls", "wc"]);
        eq("echo ${HOME}/x | wc", &["echo", "wc"]);
    }

    /// Found in the history: comments, the `:` command, pasted escape sequences and stray
    /// punctuation were reported as programs.
    #[test]
    fn comments_and_noise_are_not_programs() {
        eq("ls # list files | wc", &["ls"]);
        eq("# just a comment", &[]);
        eq("echo a#b", &["echo"]);
        eq("ls # c\nwc -l", &["ls", "wc"]);
        eq(": > file", &[]);
        eq("\u{1b}[200~cd /tmp", &[]);
        eq("? x", &[]);
        eq("\\", &[]);
        assert!(is_builtin(":"));
    }

    fn renames(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn programs_know_where_they_are() {
        let cmd = "sudo fd -e rs | rg x";
        let p = programs(cmd);
        assert_eq!(
            p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["fd", "rg"]
        );
        let chars: Vec<char> = cmd.chars().collect();
        let text = |s: (usize, usize)| chars[s.0..s.1].iter().collect::<String>();
        assert_eq!(text(p[0].span.unwrap()), "fd");
        assert_eq!(text(p[1].span.unwrap()), "rg");
        // programs inside a substitution have no position
        let inner = programs("echo $(fd x)");
        assert_eq!(inner[1].name, "fd");
        assert_eq!(inner[1].span, None);
    }

    #[test]
    fn renaming_replaces_program_words_only() {
        let r = renames(&[("fd", "fdfind")]);
        assert_eq!(rename_programs("fd -e rs", &r), "fdfind -e rs");
        assert_eq!(
            rename_programs("sudo fd x | fd y", &r),
            "sudo fdfind x | fdfind y"
        );
        assert_eq!(
            rename_programs("rg fd src", &r),
            "rg fd src",
            "an argument is not a program"
        );
        assert_eq!(rename_programs("echo fd", &r), "echo fd");
        assert_eq!(
            rename_programs("echo $(fd x) | fd y", &r),
            "echo $(fd x) | fdfind y"
        );
        assert_eq!(rename_programs("FOO=1 fd x", &r), "FOO=1 fdfind x");
        assert_eq!(rename_programs("fd", &r), "fdfind");
        assert_eq!(rename_programs("\"fd\" x", &r), "fdfind x");
        assert_eq!(
            rename_programs("ls ñandú | fd é", &r),
            "ls ñandú | fdfind é",
            "positions are chars, not bytes"
        );
        assert_eq!(rename_programs("fd x", &renames(&[])), "fd x");
    }

    fn simple(program: &str, args: &[&str], redirects: &[&str]) -> Simple {
        Simple {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            redirects: redirects.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn commands_have_their_arguments_and_redirects() {
        assert_eq!(commands("sudo rm -rf /"), vec![simple("rm", &["-rf", "/"], &[])]);
        assert_eq!(
            commands("ls -la | grep x > out.txt 2>&1"),
            vec![
                simple("ls", &["-la"], &[]),
                simple("grep", &["x"], &["out.txt", "1"])
            ]
        );
        assert_eq!(
            commands("FOO=1 make -j4 all"),
            vec![simple("make", &["-j4", "all"], &[])]
        );
        assert_eq!(
            commands("sudo -u bob env X=1 tar x"),
            vec![simple("tar", &["x"], &[])]
        );
        assert_eq!(
            commands("> log.txt echo hi"),
            vec![simple("echo", &["hi"], &["log.txt"])]
        );
        assert_eq!(
            commands("find . | xargs rm -f"),
            vec![simple("find", &["."], &[]), simple("rm", &["-f"], &[])]
        );
        assert_eq!(
            commands("a; b && c"),
            vec![
                simple("a", &[], &[]),
                simple("b", &[], &[]),
                simple("c", &[], &[])
            ]
        );
        assert_eq!(
            commands("echo \"a b\" 'c d'"),
            vec![simple("echo", &["a b", "c d"], &[])]
        );
        assert_eq!(commands(""), Vec::<Simple>::new());
        assert_eq!(commands("FOO=1"), Vec::<Simple>::new());
    }

    #[test]
    fn commands_inside_substitutions_are_included() {
        let c = commands("echo $(rm -rf x) done");
        assert!(c.contains(&simple("echo", &["$SUB", "done"], &[])), "{c:?}");
        assert!(c.contains(&simple("rm", &["-rf", "x"], &[])), "{c:?}");
    }

    #[test]
    fn absolute_paths_are_kept() {
        eq("/usr/local/bin/mytool --x", &["/usr/local/bin/mytool"]);
    }

    #[test]
    fn repeats_are_listed_once_in_order() {
        eq("grep a f | grep b | sort | sort -u", &["grep", "sort"]);
    }

    #[test]
    fn empty_and_odd_input_do_not_panic() {
        eq("", &[]);
        eq("   ", &[]);
        eq("|", &[]);
        eq("&&", &[]);
        eq("echo \"unterminated", &["echo"]);
        eq("echo $(unterminated", &["echo", "unterminated"]);
        eq("echo \\", &["echo"]);
        eq("a b c |", &["a"]);
    }

    // ---- PATH lookup, on a temporary directory

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pistasho-sys-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_file(dir: &Path, name: &str, mode: u32) {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn which_finds_executables_only() {
        let a = temp_dir("a");
        let b = temp_dir("b");
        make_file(&a, "tool", 0o755);
        make_file(&a, "notexec", 0o644);
        make_file(&b, "other", 0o700);
        std::fs::create_dir_all(a.join("adir")).unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();

        assert_eq!(which_in("tool", &path), Some(a.join("tool")));
        assert_eq!(
            which_in("other", &path),
            Some(b.join("other")),
            "second directory"
        );
        assert_eq!(which_in("notexec", &path), None, "no execute bit");
        assert_eq!(which_in("adir", &path), None, "a directory is not a program");
        assert_eq!(which_in("nope", &path), None);
        // a name with a slash is checked where it says
        let abs = a.join("tool");
        assert_eq!(which_in(abs.to_str().unwrap(), OsStr::new("")), Some(abs));
        std::fs::remove_dir_all(a).ok();
        std::fs::remove_dir_all(b).ok();
    }

    #[test]
    fn empty_path_entries_are_ignored() {
        let a = temp_dir("empty");
        make_file(&a, "tool", 0o755);
        // an empty entry would mean "current directory" in a real shell; it must not match here
        let cwd_tool = std::env::current_dir().unwrap().join("tool-in-cwd");
        assert!(!cwd_tool.exists());
        let path = std::env::join_paths([PathBuf::new(), a.clone()]).unwrap();
        assert_eq!(which_in("tool", &path), Some(a.join("tool")));
        std::fs::remove_dir_all(a).ok();
    }

    #[test]
    fn missing_binaries_skips_builtins_and_finds_what_is_absent() {
        let dir = temp_dir("missing");
        make_file(&dir, "tar", 0o755);
        make_file(&dir, "grep", 0o755);
        let path = std::env::join_paths([&dir]).unwrap();

        assert_eq!(
            missing_binaries_in("tar czf a.tgz d | grep x", &path),
            Vec::<String>::new()
        );
        assert_eq!(missing_binaries_in("ncdu /var", &path), vec!["ncdu"]);
        assert_eq!(
            missing_binaries_in("sudo tree src | grep x | wc -l", &path),
            vec!["tree", "wc"],
            "sudo is skipped, grep is there, tree and wc are not"
        );
        // builtins need no file: `fc -ln 1 | grep root` works with only grep on PATH
        assert_eq!(
            missing_binaries_in("fc -ln 1 | grep root", &path),
            Vec::<String>::new()
        );
        assert_eq!(
            missing_binaries_in("cd /tmp && tar x", &path),
            Vec::<String>::new()
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
