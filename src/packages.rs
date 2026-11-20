//! Which operating system this is, which package manager it uses, and which package provides a
//! command that is not installed.
//!
//! A curated table (`data/packages.toml`) lists the commands whose package is not named like the
//! command, and the programs a distro installs under another name (`fd` is `fdfind` on Debian).
//! Everything else is assumed to be packaged under its own name, and the package manager itself
//! is asked whether a package exists before it is proposed.

use crate::system;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

// ---------------------------------------------------------------- the OS

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsInfo {
    pub id: String,
    pub id_like: Vec<String>,
    pub pretty_name: String,
}

/// Parses the text of `/etc/os-release`: `KEY=value` lines, the value optionally quoted.
pub fn parse_os_release(text: &str) -> OsInfo {
    let mut info = OsInfo::default();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
        match key.trim() {
            "ID" => info.id = value.to_lowercase(),
            "ID_LIKE" => info.id_like = value.split_whitespace().map(str::to_lowercase).collect(),
            "PRETTY_NAME" => info.pretty_name = value.to_string(),
            _ => {}
        }
    }
    info
}

/// The OS this runs on, from `/etc/os-release` (or `/usr/lib/os-release`).
pub fn detect_os() -> Option<OsInfo> {
    ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|text| parse_os_release(&text))
        .filter(|info| !info.id.is_empty())
}

// ---------------------------------------------------------------- package managers

/// The official package managers pistasho knows (never `curl | sh`, see CLAUDE.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Apt,
    Pacman,
    Dnf,
    Zypper,
}

impl Manager {
    /// The name used in `data/packages.toml` and shown to the user.
    pub fn name(self) -> &'static str {
        match self {
            Manager::Apt => "apt",
            Manager::Pacman => "pacman",
            Manager::Dnf => "dnf",
            Manager::Zypper => "zypper",
        }
    }

    /// The manager for an OS: its own `ID` first, then the families in `ID_LIKE`
    /// (Linux Mint is `ubuntu debian`, Manjaro is `arch`, Rocky is `rhel centos fedora`).
    pub fn from_os(os: &OsInfo) -> Option<Manager> {
        std::iter::once(os.id.as_str())
            .chain(os.id_like.iter().map(String::as_str))
            .find_map(|id| match id {
                "debian" | "ubuntu" | "linuxmint" | "pop" | "raspbian" | "kali" | "elementary" => {
                    Some(Manager::Apt)
                }
                "arch" | "manjaro" | "endeavouros" | "cachyos" | "garuda" | "artix" => Some(Manager::Pacman),
                "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "amzn" | "nobara" => {
                    Some(Manager::Dnf)
                }
                "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" | "suse" => {
                    Some(Manager::Zypper)
                }
                _ => None,
            })
    }

    /// The manager of this machine: from the OS, or else the first one found on `PATH`.
    /// Worked out once per run.
    pub fn detect() -> Option<Manager> {
        static DETECTED: OnceLock<Option<Manager>> = OnceLock::new();
        *DETECTED.get_or_init(Manager::detect_now)
    }

    fn detect_now() -> Option<Manager> {
        detect_os().and_then(|os| Manager::from_os(&os)).or_else(|| {
            [
                (Manager::Apt, "apt-get"),
                (Manager::Pacman, "pacman"),
                (Manager::Dnf, "dnf"),
                (Manager::Zypper, "zypper"),
            ]
            .into_iter()
            .find(|(_, bin)| system::which(bin).is_some())
            .map(|(m, _)| m)
        })
    }

    /// The command that installs `packages`, as the argument list to run and show.
    /// It asks for no further confirmation: pistasho has already asked, showing this command.
    pub fn install_command(self, packages: &[String]) -> Vec<String> {
        let args: &[&str] = match self {
            Manager::Apt => &["sudo", "apt", "install", "-y"],
            Manager::Pacman => &["sudo", "pacman", "-S", "--noconfirm"],
            Manager::Dnf => &["sudo", "dnf", "install", "-y"],
            Manager::Zypper => &["sudo", "zypper", "--non-interactive", "install"],
        };
        args.iter()
            .map(|a| a.to_string())
            .chain(packages.iter().cloned())
            .collect()
    }

    /// The command that asks the manager about a package (read-only, no root needed).
    fn query_command(self, package: &str) -> Vec<String> {
        let args: &[&str] = match self {
            Manager::Apt => &["apt-cache", "policy"],
            Manager::Pacman => &["pacman", "-Si"],
            Manager::Dnf => &["dnf", "-q", "info"],
            // `what-provides`, not `info`: openSUSE names Python packages by version
            // (`python313`) and offers `python3` / `python3-pip` as capabilities, which
            // `zypper install` accepts but `zypper info` does not find
            Manager::Zypper => &["zypper", "--non-interactive", "--quiet", "what-provides"],
        };
        args.iter()
            .map(|a| a.to_string())
            .chain([package.to_string()])
            .collect()
    }

    /// Reads the answer of `query_command`. `apt-cache policy` exits 0 even for a package that
    /// does not exist, so its output is what says; the others say it with their exit status
    /// (zypper also says `No matching items found.` or `not found`).
    fn package_exists_from(self, success: bool, stdout: &str) -> bool {
        match self {
            Manager::Apt => apt_has_candidate(stdout),
            Manager::Zypper => {
                let out = stdout.to_lowercase();
                success && !out.contains("not found") && !out.contains("no matching items")
            }
            Manager::Pacman | Manager::Dnf => success,
        }
    }
}

/// Whether the output of `apt-cache policy X` shows an installable version.
fn apt_has_candidate(stdout: &str) -> bool {
    stdout
        .lines()
        .filter_map(|l| l.trim().strip_prefix("Candidate:"))
        .any(|v| {
            let v = v.trim();
            !v.is_empty() && v != "(none)"
        })
}

/// Asks the package manager whether `package` exists. No root, nothing is changed.
pub fn exists(manager: Manager, package: &str) -> Result<bool> {
    let cmd = manager.query_command(package);
    let out = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .with_context(|| format!("could not run {}", cmd[0]))?;
    Ok(manager.package_exists_from(out.status.success(), &String::from_utf8_lossy(&out.stdout)))
}

// ---------------------------------------------------------------- the curated table

#[derive(Debug, Default, Deserialize)]
struct Row {
    #[serde(default)]
    package: HashMap<String, String>,
    #[serde(default)]
    binary: HashMap<String, String>,
}

type Table = HashMap<String, Row>;

fn parse_table(text: &str) -> Result<Table> {
    toml::from_str(text).context("data/packages.toml")
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        parse_table(include_str!("../data/packages.toml")).expect("data/packages.toml is valid")
    })
}

/// Every command the table knows, sorted. For checking the table against a real repository.
pub fn table_commands() -> Vec<String> {
    let mut commands: Vec<String> = table().keys().cloned().collect();
    commands.sort();
    commands
}

fn pick(map: &HashMap<String, String>, manager: Manager) -> Option<&str> {
    map.get(manager.name())
        .or_else(|| map.get("default"))
        .map(String::as_str)
}

/// The package that provides `command` on `manager`: the table's answer, or the command's own
/// name when the table does not list it.
pub fn package_for(command: &str, manager: Manager) -> String {
    table()
        .get(command)
        .and_then(|row| pick(&row.package, manager))
        .unwrap_or(command)
        .to_string()
}

/// The name `command` has once installed on `manager`, when it is not its own (`fd` → `fdfind`
/// on Debian and Ubuntu).
pub fn installed_name(command: &str, manager: Manager) -> Option<&'static str> {
    table().get(command).and_then(|row| pick(&row.binary, manager))
}

// ---------------------------------------------------------------- putting it together

/// A command with the programs this machine has under other names put right.
pub struct Adapted {
    pub command: String,
    /// `(asked for, installed as)`, e.g. `("fd", "fdfind")`.
    pub renames: Vec<(String, String)>,
}

/// Rewrites the programs of `cmd` that are not installed but exist under another name here
/// (`fd` → `fdfind` on Debian), so the command can run as it is. Leaves the rest untouched.
pub fn adapt(cmd: &str) -> Adapted {
    adapt_with(cmd, Manager::detect(), |bin| system::which(bin).is_some())
}

fn adapt_with(cmd: &str, manager: Option<Manager>, installed: impl Fn(&str) -> bool) -> Adapted {
    let mut renames: Vec<(String, String)> = Vec::new();
    if let Some(manager) = manager {
        for bin in system::binaries(cmd) {
            if system::is_builtin(&bin) || installed(&bin) {
                continue;
            }
            if let Some(other) = installed_name(&bin, manager) {
                if other != bin && installed(other) {
                    renames.push((bin, other.to_string()));
                }
            }
        }
    }
    let map: HashMap<String, String> = renames.iter().cloned().collect();
    Adapted {
        command: system::rename_programs(cmd, &map),
        renames,
    }
}

/// How to get programs that are not installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub manager: Manager,
    pub packages: Vec<String>,
    /// The command to run, as arguments (`["sudo", "apt", "install", "-y", "ncdu"]`).
    pub command: Vec<String>,
}

impl InstallPlan {
    /// The command as one line, the way it is shown for confirmation.
    pub fn display(&self) -> String {
        self.command.join(" ")
    }
}

/// The plan to install the package that provides one program. See [`plan_install_for`].
pub fn plan_install(program: &str) -> Option<InstallPlan> {
    plan_install_for(&[program.to_string()])
}

/// The plan to install the packages that provide `programs`, if this machine has a package
/// manager pistasho knows and that manager confirms every package exists. All or nothing: a plan
/// that would leave one of the programs missing is not offered.
pub fn plan_install_for(programs: &[String]) -> Option<InstallPlan> {
    let manager = Manager::detect()?;
    plan_install_for_with(programs, manager, |pkg| exists(manager, pkg).unwrap_or(false))
}

/// A package name as the managers write them. Anything else (a space, a `;`, a leading `-`)
/// never reaches the install command: the names come from commands pistasho has read, and the
/// command is run without a shell, but a name that is not a package is not a plan either.
fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['-', '.', '+'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-._".contains(c))
}

fn plan_install_for_with(
    programs: &[String],
    manager: Manager,
    exists: impl Fn(&str) -> bool,
) -> Option<InstallPlan> {
    let mut packages: Vec<String> = Vec::new();
    for program in programs {
        let package = package_for(program, manager);
        if !valid_package_name(&package) || !exists(&package) {
            return None;
        }
        if !packages.contains(&package) {
            packages.push(package);
        }
    }
    (!packages.is_empty()).then(|| InstallPlan {
        manager,
        command: manager.install_command(&packages),
        packages,
    })
}

/// Runs the install in the foreground with the terminal as stdin, stdout and stderr, so the
/// package manager's output shows and `sudo` can ask for the password. Returns whether it
/// succeeded. This is the one thing pistasho runs by itself, and only after the user said yes
/// to this exact command.
///
/// The terminal is `/dev/tty` and not the process's own stdout, which inside the zsh widget is
/// the pipe that carries the result back.
pub fn run_install(plan: &InstallPlan) -> Result<bool> {
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("could not open /dev/tty")?;
    let status = Command::new(&plan.command[0])
        .args(&plan.command[1..])
        .stdin(Stdio::from(tty.try_clone()?))
        .stdout(Stdio::from(tty.try_clone()?))
        .stderr(Stdio::from(tty))
        .status()
        .with_context(|| format!("could not run {}", plan.command[0]))?;
    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UBUNTU: &str = r#"PRETTY_NAME="Ubuntu 24.04.5 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
ID=ubuntu
ID_LIKE=debian
HOME_URL="https://www.ubuntu.com/"
"#;

    fn os(id: &str, like: &[&str]) -> OsInfo {
        OsInfo {
            id: id.into(),
            id_like: like.iter().map(|s| s.to_string()).collect(),
            pretty_name: String::new(),
        }
    }

    #[test]
    fn parses_os_release() {
        let info = parse_os_release(UBUNTU);
        assert_eq!(info.id, "ubuntu");
        assert_eq!(info.id_like, vec!["debian"]);
        assert_eq!(info.pretty_name, "Ubuntu 24.04.5 LTS");
    }

    #[test]
    fn os_release_quirks() {
        // Arch: no ID_LIKE, no quotes
        let arch =
            parse_os_release("NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\nBUILD_ID=rolling\n");
        assert_eq!((arch.id.as_str(), arch.id_like.len()), ("arch", 0));
        // several families, single quotes, comments and blank lines, upper case
        let rocky = parse_os_release("# a comment\n\nID='Rocky'\nID_LIKE=\"rhel centos fedora\"\n");
        assert_eq!(rocky.id, "rocky");
        assert_eq!(rocky.id_like, vec!["rhel", "centos", "fedora"]);
        // `=` inside a value
        assert_eq!(parse_os_release("PRETTY_NAME=\"a=b\"\n").pretty_name, "a=b");
        // junk and empty input
        assert_eq!(parse_os_release("no equals here\n"), OsInfo::default());
        assert_eq!(parse_os_release(""), OsInfo::default());
    }

    #[test]
    fn manager_from_os() {
        assert_eq!(Manager::from_os(&os("ubuntu", &["debian"])), Some(Manager::Apt));
        assert_eq!(Manager::from_os(&os("debian", &[])), Some(Manager::Apt));
        assert_eq!(Manager::from_os(&os("arch", &[])), Some(Manager::Pacman));
        assert_eq!(Manager::from_os(&os("fedora", &[])), Some(Manager::Dnf));
        assert_eq!(
            Manager::from_os(&os("opensuse-tumbleweed", &["opensuse", "suse"])),
            Some(Manager::Zypper)
        );
        // derivatives go through ID_LIKE
        assert_eq!(
            Manager::from_os(&os("linuxmint", &["ubuntu", "debian"])),
            Some(Manager::Apt)
        );
        assert_eq!(Manager::from_os(&os("manjaro", &["arch"])), Some(Manager::Pacman));
        assert_eq!(
            Manager::from_os(&os("rocky", &["rhel", "centos", "fedora"])),
            Some(Manager::Dnf)
        );
        assert_eq!(
            Manager::from_os(&os("somethingnew", &["debian"])),
            Some(Manager::Apt)
        );
        // an OS with no known family
        assert_eq!(Manager::from_os(&os("nixos", &[])), None);
        assert_eq!(Manager::from_os(&OsInfo::default()), None);
    }

    #[test]
    fn install_commands_never_ask_again() {
        let ncdu = ["ncdu".to_string()];
        assert_eq!(
            Manager::Apt.install_command(&ncdu).join(" "),
            "sudo apt install -y ncdu"
        );
        assert_eq!(
            Manager::Pacman.install_command(&ncdu).join(" "),
            "sudo pacman -S --noconfirm ncdu"
        );
        assert_eq!(
            Manager::Dnf.install_command(&ncdu).join(" "),
            "sudo dnf install -y ncdu"
        );
        assert_eq!(
            Manager::Zypper.install_command(&ncdu).join(" "),
            "sudo zypper --non-interactive install ncdu"
        );
        let two = ["tree".to_string(), "ncdu".to_string()];
        assert_eq!(
            Manager::Apt.install_command(&two).join(" "),
            "sudo apt install -y tree ncdu"
        );
    }

    #[test]
    fn apt_policy_output() {
        let found = "ncdu:\n  Installed: (none)\n  Candidate: 1.19-0.1\n  Version table:\n";
        let none = "foo:\n  Installed: (none)\n  Candidate: (none)\n";
        assert!(apt_has_candidate(found));
        assert!(apt_has_candidate(
            "tree:\n  Installed: 2.1.1-2\n  Candidate: 2.1.1-2\n"
        ));
        assert!(!apt_has_candidate(none));
        assert!(
            !apt_has_candidate(""),
            "apt-cache prints nothing for an unknown package"
        );
        assert!(Manager::Apt.package_exists_from(true, found));
        assert!(
            !Manager::Apt.package_exists_from(true, ""),
            "exit 0 is not enough for apt"
        );
    }

    #[test]
    fn other_managers_answer_with_their_exit_status() {
        assert!(Manager::Pacman.package_exists_from(true, "Name : ncdu"));
        assert!(!Manager::Pacman.package_exists_from(false, ""));
        assert!(Manager::Dnf.package_exists_from(true, ""));
        assert!(!Manager::Dnf.package_exists_from(false, ""));
        assert!(Manager::Zypper.package_exists_from(
            true,
            "S  | Name      | Summary\n   | python313 | Python 3 Interpreter"
        ));
        assert!(!Manager::Zypper.package_exists_from(true, "No matching items found."));
        assert!(!Manager::Zypper.package_exists_from(true, "package 'nope' not found."));
        assert!(!Manager::Zypper.package_exists_from(false, ""));
    }

    // ---- the table

    #[test]
    fn the_table_parses_and_its_rows_are_well_formed() {
        let t = table();
        assert!(t.len() > 30);
        let managers = ["apt", "pacman", "dnf", "zypper", "default"];
        for (command, row) in t {
            assert!(
                !row.package.is_empty() || !row.binary.is_empty(),
                "{command}: empty row"
            );
            for key in row.package.keys().chain(row.binary.keys()) {
                assert!(
                    managers.contains(&key.as_str()),
                    "{command}: unknown manager {key:?}"
                );
            }
            // a package for a manager the row names must be reachable even without `default`
            for m in [Manager::Apt, Manager::Pacman, Manager::Dnf, Manager::Zypper] {
                if !row.package.is_empty() {
                    assert!(
                        pick(&row.package, m).is_some(),
                        "{command}: no package for {}",
                        m.name()
                    );
                }
            }
        }
    }

    /// The cases the plan names, and the ones that tripped other tools up.
    #[test]
    fn rare_cases() {
        assert_eq!(package_for("rg", Manager::Apt), "ripgrep");
        assert_eq!(package_for("rg", Manager::Pacman), "ripgrep");
        // fd: `fd-find` on Debian and Fedora, `fd` on Arch and openSUSE; and `fdfind` on Debian
        assert_eq!(package_for("fd", Manager::Apt), "fd-find");
        assert_eq!(package_for("fd", Manager::Dnf), "fd-find");
        assert_eq!(package_for("fd", Manager::Pacman), "fd");
        assert_eq!(package_for("fd", Manager::Zypper), "fd");
        assert_eq!(installed_name("fd", Manager::Apt), Some("fdfind"));
        assert_eq!(installed_name("fd", Manager::Pacman), None);
        // bat: the package is `bat` everywhere, the Debian program is `batcat`
        assert_eq!(package_for("bat", Manager::Apt), "bat");
        assert_eq!(installed_name("bat", Manager::Apt), Some("batcat"));
        assert_eq!(installed_name("bat", Manager::Dnf), None);
        assert_eq!(package_for("delta", Manager::Apt), "git-delta");
        assert_eq!(package_for("dig", Manager::Apt), "dnsutils");
        assert_eq!(package_for("dig", Manager::Dnf), "bind-utils");
        assert_eq!(package_for("dig", Manager::Pacman), "bind");
        assert_eq!(package_for("ss", Manager::Dnf), "iproute");
        assert_eq!(package_for("ss", Manager::Apt), "iproute2");
        assert_eq!(package_for("docker", Manager::Apt), "docker.io");
        assert_eq!(package_for("docker", Manager::Pacman), "docker");
        assert_eq!(package_for("mkfs.ext4", Manager::Apt), "e2fsprogs");
        assert_eq!(installed_name("python", Manager::Dnf), Some("python3"));
        // names that changed on one distro and were caught by checking real repositories
        assert_eq!(package_for("docker", Manager::Dnf), "moby-engine");
        assert_eq!(package_for("7z", Manager::Dnf), "7zip");
        assert_eq!(package_for("sensors", Manager::Zypper), "sensors");
    }

    #[test]
    fn commands_not_in_the_table_are_packaged_under_their_own_name() {
        for m in [Manager::Apt, Manager::Pacman, Manager::Dnf, Manager::Zypper] {
            for cmd in ["ncdu", "tree", "jq", "htop", "btop", "tmux", "unknown-tool"] {
                assert_eq!(package_for(cmd, m), cmd);
            }
            assert_eq!(installed_name("ncdu", m), None);
        }
    }

    // ---- rewriting a command for this machine

    #[test]
    fn a_program_installed_under_another_name_is_rewritten() {
        let only = |have: &'static [&'static str]| move |b: &str| have.contains(&b);
        // fd is missing, fdfind is there (Debian)
        let a = adapt_with("fd -e rs | wc -l", Some(Manager::Apt), only(&["fdfind", "wc"]));
        assert_eq!(a.command, "fdfind -e rs | wc -l");
        assert_eq!(a.renames, vec![("fd".to_string(), "fdfind".to_string())]);
        // already installed under its own name: untouched
        let a = adapt_with("fd x", Some(Manager::Apt), only(&["fd", "fdfind"]));
        assert_eq!((a.command.as_str(), a.renames.len()), ("fd x", 0));
        // missing, and the other name is not there either: nothing to rewrite to
        let a = adapt_with("fd x", Some(Manager::Apt), only(&[]));
        assert_eq!((a.command.as_str(), a.renames.len()), ("fd x", 0));
        // the rename belongs to the distro: on Arch `fd` is `fd`
        let a = adapt_with("fd x", Some(Manager::Pacman), only(&["fdfind"]));
        assert_eq!(a.command, "fd x");
        // no known package manager: leave everything alone
        let a = adapt_with("fd x", None, only(&["fdfind"]));
        assert_eq!(a.command, "fd x");
        // `python` → `python3` on any distro, and wrappers do not get in the way
        let a = adapt_with("sudo python -m venv x", Some(Manager::Dnf), only(&["python3"]));
        assert_eq!(a.command, "sudo python3 -m venv x");
        // shell builtins are never rewritten
        let a = adapt_with("cd /tmp && fd x", Some(Manager::Apt), only(&["fdfind"]));
        assert_eq!(a.command, "cd /tmp && fdfind x");
    }

    // ---- the install plan

    fn plan(programs: &[&str], manager: Manager, have: &'static [&'static str]) -> Option<InstallPlan> {
        let programs: Vec<String> = programs.iter().map(|p| p.to_string()).collect();
        plan_install_for_with(&programs, manager, move |pkg| have.contains(&pkg))
    }

    #[test]
    fn install_plan_for_an_existing_package() {
        let p = plan(&["ncdu"], Manager::Apt, &["ncdu"]).unwrap();
        assert_eq!(p.packages, vec!["ncdu"]);
        assert_eq!(p.display(), "sudo apt install -y ncdu");
        assert_eq!(p.manager, Manager::Apt);
        // the table's package is what gets verified and installed
        let p = plan(&["fd"], Manager::Apt, &["fd-find"]).unwrap();
        assert_eq!(p.display(), "sudo apt install -y fd-find");
        let p = plan(&["fd"], Manager::Pacman, &["fd"]).unwrap();
        assert_eq!(p.display(), "sudo pacman -S --noconfirm fd");
        assert_eq!(
            plan_install_for_with(&[], Manager::Apt, |_| true),
            None,
            "nothing to install"
        );
    }

    #[test]
    fn one_plan_for_several_programs_without_repeats() {
        let p = plan(&["tree", "ncdu"], Manager::Apt, &["tree", "ncdu"]).unwrap();
        assert_eq!(p.display(), "sudo apt install -y tree ncdu");
        // two programs of one package are installed once
        let p = plan(&["ss", "ip"], Manager::Apt, &["iproute2"]).unwrap();
        assert_eq!(p.packages, vec!["iproute2"]);
    }

    #[test]
    fn no_plan_when_the_manager_does_not_have_the_package() {
        assert_eq!(plan(&["nonexistent-tool"], Manager::Apt, &[]), None);
        // asked about the right package, not about the command's name
        assert_eq!(plan(&["fd"], Manager::Apt, &["fd"]), None);
        // all or nothing: a plan that leaves one program missing is not offered
        assert_eq!(plan(&["tree", "nonexistent-tool"], Manager::Apt, &["tree"]), None);
    }

    /// The names come from commands that were read from tldr, cheats and history. They are never
    /// run through a shell, but a name that is not a package name is not a plan either.
    #[test]
    fn odd_names_never_become_a_plan() {
        for odd in [
            "a;b", "a b", "-rf", "--yes", "$(x)", "a&&b", "x\ny", "", ".hidden", "+x", "a|b", "../x",
        ] {
            assert!(!valid_package_name(odd), "{odd:?}");
            assert_eq!(
                plan(&[odd], Manager::Apt, &["a;b", "a b", "-rf", "--yes", "$(x)", ""]),
                None,
                "{odd:?}"
            );
        }
        for fine in [
            "ncdu",
            "fd-find",
            "g++",
            "python3.12",
            "libfoo_bar",
            "docker.io",
            "7zip",
            "lm_sensors",
        ] {
            assert!(valid_package_name(fine), "{fine:?}");
        }
    }
}
