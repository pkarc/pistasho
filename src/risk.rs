//! How much damage a command can do before it is run.
//!
//! Fixed rules, no model: `ok`, `caution` (it can lose work or change the system, and the user
//! should look first) and `destructive` (it can wipe data that has no way back: a device, a
//! whole tree, a history). pistasho never runs the command either way; the level decides how
//! loudly it is put on the prompt.
//!
//! The rules read what `system::commands` finds (program, arguments, redirects) plus a few
//! patterns in the text that are not shell commands (a fork bomb, SQL).

use crate::devices;
use crate::i18n::tr;
use crate::system::{self, Simple};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Caution,
    Destructive,
}

impl Level {
    /// The stable name, for the protocol between the binary and the zsh widget.
    pub fn name(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Caution => "caution",
            Level::Destructive => "destructive",
        }
    }

    /// The label shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Caution => tr("caution", "cuidado"),
            Level::Destructive => tr("destructive", "destructivo"),
        }
    }
}

/// One rule that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    /// A short stable id: `rm`, `mkfs`, `dd`…
    pub rule: &'static str,
    /// What the command does, in a sentence.
    pub effect: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    /// The worst level of the findings.
    pub level: Level,
    pub findings: Vec<Finding>,
    /// Block devices the command writes to (`/dev/sdb1`), to show what they are.
    pub devices: Vec<String>,
}

/// Adds what a decision model thinks (the probability that the command is destructive) to the
/// fixed rules' verdict. It can only raise the level: at 0.65 or more a command the rules let pass
/// is shown in the red box, at 0.3 or more it gets a caution. The cut-offs come from
/// `scripts/jev_experiment.py risk` and a check on real tldr templates (docs/eval.md): harmless
/// templates scored under 0.21, a plain `rm {{file}}` 0.55 (so it must not get the red box) and
/// `dd`, `mkfs` and `DROP TABLE` 0.70–0.93. Small samples: the margins are not wide.
pub fn with_model(mut assessment: Assessment, destructive: f64) -> Assessment {
    if destructive >= crate::decide::NOUL_DESTRUCTIVE && assessment.level != Level::Destructive {
        assessment.findings.push(Finding {
            level: Level::Destructive,
            rule: "model",
            effect: tr(
                "a decision model flags it as destructive",
                "un modelo de decisión lo marca como destructivo",
            ),
        });
        assessment.level = Level::Destructive;
    } else if destructive >= crate::decide::NOUL_RISKY && assessment.level == Level::Ok {
        assessment.findings.push(Finding {
            level: Level::Caution,
            rule: "model",
            effect: tr(
                "a decision model flags it as risky",
                "un modelo de decisión lo marca como riesgoso",
            ),
        });
        assessment.level = Level::Caution;
    }
    assessment
}

impl Assessment {
    /// The sentences of the findings at the worst level, joined.
    pub fn effect(&self) -> String {
        self.findings
            .iter()
            .filter(|f| f.level == self.level)
            .map(|f| f.effect)
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The ids of the rules that matched, for the `Signals:` line.
    pub fn rules(&self) -> String {
        self.findings
            .iter()
            .map(|f| f.rule)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Top-level directories whose loss breaks the system.
const SYSTEM_DIRS: &[&str] = &[
    "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/opt", "/proc", "/root", "/run", "/sbin",
    "/srv", "/sys", "/usr", "/var",
];

/// `/etc`, `/etc/`, `/etc/*`
fn is_system_dir(target: &str) -> bool {
    let t = target.trim_end_matches('/');
    let t = t.strip_suffix("/*").unwrap_or(t);
    t.is_empty() || SYSTEM_DIRS.contains(&t)
}

/// What `rm -r` must not be pointed at: the root, the system directories, the home directory
/// and everything in the current one.
fn is_rm_catastrophe(target: &str) -> bool {
    let t = target.trim_end_matches('/');
    let bare = t.strip_suffix("/*").unwrap_or(t);
    is_system_dir(target)
        || matches!(
            bare,
            "~" | "$HOME" | "${HOME}" | "*" | ".*" | "." | ".." | "./*" | "../*"
        )
        || matches!(t, "*" | ".*" | "." | "..")
}

#[derive(Default)]
struct Flags<'a> {
    recursive: bool,
    force: bool,
    no_preserve_root: bool,
    /// Arguments that are not options.
    operands: Vec<&'a str>,
}

/// Reads `-rf`, `-r -f`, `--recursive`, `--force`, `--no-preserve-root` and `--`.
fn flags(args: &[String]) -> Flags<'_> {
    let mut f = Flags::default();
    let mut options_ended = false;
    for arg in args {
        let a = arg.as_str();
        if options_ended || !a.starts_with('-') || a == "-" {
            f.operands.push(a);
        } else if a == "--" {
            options_ended = true;
        } else if let Some(long) = a.strip_prefix("--") {
            match long {
                "recursive" => f.recursive = true,
                "force" => f.force = true,
                "no-preserve-root" => f.no_preserve_root = true,
                _ => {}
            }
        } else {
            for c in a[1..].chars() {
                match c {
                    'r' | 'R' => f.recursive = true,
                    'f' => f.force = true,
                    _ => {}
                }
            }
        }
    }
    f
}

struct Found {
    findings: Vec<Finding>,
    devices: Vec<String>,
}

impl Found {
    fn add(&mut self, level: Level, rule: &'static str, effect: &'static str) {
        if !self.findings.iter().any(|f| f.rule == rule && f.level == level) {
            self.findings.push(Finding { level, rule, effect });
        }
    }

    fn device(&mut self, path: &str) {
        if !self.devices.iter().any(|d| d == path) {
            self.devices.push(path.to_string());
        }
    }
}

/// How dangerous `cmd` is, and why.
pub fn assess(cmd: &str) -> Assessment {
    let mut found = Found {
        findings: Vec::new(),
        devices: Vec::new(),
    };
    let commands = system::commands(cmd);
    for c in &commands {
        program_rules(c, &mut found);
        for target in &c.redirects {
            redirect_rules(target, &mut found);
        }
    }
    text_rules(cmd, &commands, &mut found);
    let level = found.findings.iter().map(|f| f.level).max().unwrap_or(Level::Ok);
    Assessment {
        level,
        findings: found.findings,
        devices: found.devices,
    }
}

fn program_rules(c: &Simple, found: &mut Found) {
    let args = &c.args;
    match c.program.as_str() {
        "rm" => {
            let f = flags(args);
            let catastrophe = f.operands.iter().any(|t| is_rm_catastrophe(t));
            if f.recursive && (catastrophe || f.no_preserve_root) {
                found.add(
                    Level::Destructive,
                    "rm",
                    tr(
                        "deletes everything under that path, with no way back",
                        "borra todo lo que hay bajo esa ruta, sin vuelta atrás",
                    ),
                );
            } else if f.recursive {
                found.add(
                    Level::Caution,
                    "rm",
                    tr(
                        "deletes the folder and everything inside it without asking",
                        "borra la carpeta y todo su contenido sin preguntar",
                    ),
                );
            } else if catastrophe {
                found.add(
                    Level::Caution,
                    "rm",
                    tr(
                        "deletes every file in that path",
                        "borra todos los archivos de esa ruta",
                    ),
                );
            }
        }
        "dd" => {
            for arg in args {
                let Some(target) = arg.strip_prefix("of=") else {
                    continue;
                };
                if devices::is_block_device(target) || devices::is_device_placeholder(target) {
                    found.add(
                        Level::Destructive,
                        "dd",
                        tr(
                            "overwrites the whole device: its data is gone",
                            "sobrescribe todo el dispositivo: sus datos se pierden",
                        ),
                    );
                    found.device(target);
                }
            }
        }
        "mkfs" | "mke2fs" | "mkswap" | "mkntfs" | "mkdosfs" => formats(c, found),
        p if p.starts_with("mkfs.") => formats(c, found),
        "wipefs" => device_wipe(
            c,
            found,
            "wipefs",
            tr(
                "erases the filesystem signatures of the device: it stops being readable",
                "borra las firmas del sistema de archivos del dispositivo: deja de poder leerse",
            ),
        ),
        "blkdiscard" => device_wipe(
            c,
            found,
            "blkdiscard",
            tr(
                "discards every block of the device: its data is gone",
                "descarta todos los bloques del dispositivo: sus datos se pierden",
            ),
        ),
        "shred" => device_wipe(
            c,
            found,
            "shred",
            tr(
                "overwrites the file so it can never be recovered",
                "sobrescribe el archivo para que no se pueda recuperar jamás",
            ),
        ),
        "chmod" | "chown" | "chgrp" => {
            let f = flags(args);
            // with no operand at all there is nothing for it to change
            if f.recursive && !f.operands.is_empty() {
                // the first operand is the mode or the owner, the rest are the targets
                let targets = f.operands.iter().skip(1);
                if targets.clone().any(|t| is_system_dir(t)) {
                    found.add(
                        Level::Destructive,
                        "chmod",
                        tr(
                            "changes permissions or owner of the whole system: it can stop booting",
                            "cambia permisos o dueño de todo el sistema: puede dejar de arrancar",
                        ),
                    );
                } else {
                    found.add(
                        Level::Caution,
                        "chmod",
                        tr(
                            "changes permissions or owner of everything under the path",
                            "cambia permisos o dueño de todo lo que hay bajo la ruta",
                        ),
                    );
                }
            }
        }
        "git" => git_rules(args, found),
        "find" => {
            let deletes = args.iter().any(|a| a == "-delete")
                || args.windows(2).any(|w| w[0] == "-exec" && w[1] == "rm");
            if deletes {
                found.add(
                    Level::Caution,
                    "find",
                    tr("deletes the files it finds", "borra los archivos que encuentra"),
                );
            }
        }
        "reboot" | "shutdown" | "poweroff" | "halt" => found.add(
            Level::Caution,
            "shutdown",
            tr("shuts down or reboots the machine", "apaga o reinicia la máquina"),
        ),
        "docker" | "podman" => {
            let prune = args.iter().any(|a| a == "prune");
            let forced_rm =
                args.iter().any(|a| a == "rm") && args.iter().any(|a| a == "-f" || a == "--force");
            if prune || forced_rm {
                found.add(
                    Level::Caution,
                    "docker",
                    tr(
                        "removes containers, images or volumes",
                        "elimina contenedores, imágenes o volúmenes",
                    ),
                );
            }
        }
        "kubectl" if args.iter().any(|a| a == "delete") => found.add(
            Level::Caution,
            "kubectl",
            tr("deletes cluster resources", "borra recursos del clúster"),
        ),
        "terraform" if args.iter().any(|a| a == "destroy") => found.add(
            Level::Caution,
            "terraform",
            tr("destroys the infrastructure", "destruye la infraestructura"),
        ),
        "crontab" if args.iter().any(|a| a == "-r") => found.add(
            Level::Caution,
            "crontab",
            tr("removes all your cron jobs", "elimina todas tus tareas de cron"),
        ),
        "iptables" | "ip6tables" | "nft" => {
            if args.iter().any(|a| a == "-F" || a == "--flush" || a == "flush") {
                found.add(
                    Level::Caution,
                    "firewall",
                    tr(
                        "removes all the firewall rules",
                        "elimina todas las reglas del cortafuegos",
                    ),
                );
            }
        }
        "userdel" => found.add(
            Level::Caution,
            "userdel",
            tr("deletes a user account", "borra una cuenta de usuario"),
        ),
        _ => {}
    }
}

fn formats(c: &Simple, found: &mut Found) {
    found.add(
        Level::Destructive,
        "mkfs",
        tr(
            "formats: erases ALL the contents of the device or partition",
            "formatea: borra TODO el contenido del dispositivo o la partición",
        ),
    );
    for a in &c.args {
        if devices::is_block_device(a) || devices::is_device_placeholder(a) {
            found.device(a);
        }
    }
}

fn device_wipe(c: &Simple, found: &mut Found, rule: &'static str, effect: &'static str) {
    found.add(Level::Destructive, rule, effect);
    for a in &c.args {
        if devices::is_block_device(a) || devices::is_device_placeholder(a) {
            found.device(a);
        }
    }
}

fn git_rules(args: &[String], found: &mut Found) {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    match args.first().map(String::as_str) {
        Some("push") => {
            // `+main` as a refspec forces that update, with or without --force
            let plus = args.iter().any(|a| a.starts_with('+') && a.len() > 1);
            let force = has("--force") || has("-f") || plus;
            let lease = args.iter().any(|a| a.starts_with("--force-with-lease"));
            let main_branch = args.iter().any(|a| {
                let branch = a.rsplit(':').next().unwrap_or(a).trim_start_matches('+');
                matches!(branch, "main" | "master" | "origin/main" | "origin/master")
            });
            if force && main_branch {
                found.add(
                    Level::Destructive,
                    "git",
                    tr(
                        "rewrites the history of main on the remote: other people lose commits",
                        "reescribe el historial de main en el remoto: otras personas pierden commits",
                    ),
                );
            } else if force || lease {
                found.add(
                    Level::Caution,
                    "git",
                    tr(
                        "rewrites history on the remote",
                        "reescribe el historial en el remoto",
                    ),
                );
            }
        }
        Some("reset") if has("--hard") => found.add(
            Level::Caution,
            "git",
            tr(
                "throws away the changes you have not committed",
                "descarta los cambios que no has confirmado",
            ),
        ),
        Some("clean") if args.iter().any(|a| a.starts_with('-') && a.contains('f')) => found.add(
            Level::Caution,
            "git",
            tr(
                "deletes the files git does not track",
                "borra los archivos que git no sigue",
            ),
        ),
        Some("branch") if has("-D") => found.add(
            Level::Caution,
            "git",
            tr(
                "deletes a branch even if it is not merged",
                "borra una rama aunque no esté fusionada",
            ),
        ),
        _ => {}
    }
}

fn redirect_rules(target: &str, found: &mut Found) {
    if devices::is_block_device(target) || devices::is_device_placeholder(target) {
        found.add(
            Level::Destructive,
            "redirect",
            tr(
                "writes straight onto the device: its data is gone",
                "escribe directamente sobre el dispositivo: sus datos se pierden",
            ),
        );
        found.device(target);
    } else if ["/etc/", "/boot/", "/bin/", "/sbin/", "/usr/", "/lib/"]
        .iter()
        .any(|p| target.starts_with(p))
    {
        found.add(
            Level::Caution,
            "redirect",
            tr("overwrites a system file", "sobrescribe un archivo del sistema"),
        );
    }
}

/// What is not a shell command: a fork bomb, SQL given to a client, a download piped to a shell.
fn text_rules(cmd: &str, commands: &[Simple], found: &mut Found) {
    let squeezed: String = cmd.chars().filter(|c| !c.is_whitespace()).collect();
    if squeezed.contains(":(){") {
        found.add(
            Level::Destructive,
            "forkbomb",
            tr(
                "fork bomb: it multiplies processes until the machine locks up",
                "bomba fork: multiplica procesos hasta bloquear la máquina",
            ),
        );
    }

    // SQL is only dangerous when a SQL client is going to read it: `grep 'drop table' notes.txt`
    // is a search, not a statement
    let sql_client = commands.iter().any(|c| {
        matches!(
            c.program.as_str(),
            "psql"
                | "mysql"
                | "mariadb"
                | "sqlite3"
                | "sqlcmd"
                | "clickhouse-client"
                | "duckdb"
                | "usql"
                | "pgcli"
                | "mycli"
        )
    });
    if sql_client {
        sql_rules(cmd, found);
    }

    // `curl … | sh`
    if cmd.contains('|') {
        let first_fetch = commands
            .iter()
            .position(|c| matches!(c.program.as_str(), "curl" | "wget"));
        let shell_after = first_fetch.is_some_and(|i| {
            commands[i + 1..]
                .iter()
                .any(|c| matches!(c.program.as_str(), "sh" | "bash" | "zsh" | "dash" | "ksh"))
        });
        if shell_after {
            found.add(
                Level::Caution,
                "curl-sh",
                tr(
                    "runs code downloaded from the network without reading it",
                    "ejecuta código descargado de la red sin leerlo",
                ),
            );
        }
    }
}

fn sql_rules(cmd: &str, found: &mut Found) {
    let upper = cmd.to_uppercase();
    let words: Vec<&str> = upper
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .collect();
    let after =
        |first: &str, second: &[&str]| words.windows(2).any(|w| w[0] == first && second.contains(&w[1]));
    if after("DROP", &["TABLE", "DATABASE", "SCHEMA"]) {
        found.add(
            Level::Destructive,
            "sql",
            tr(
                "deletes a table or a database with all its data",
                "borra una tabla o una base de datos con todos sus datos",
            ),
        );
    }
    if words.contains(&"TRUNCATE") {
        found.add(
            Level::Destructive,
            "sql",
            tr("empties a table", "vacía una tabla"),
        );
    }
    // `DELETE FROM t` with nothing to say which rows
    if words.windows(2).any(|w| w[0] == "DELETE" && w[1] == "FROM") && !words.contains(&"WHERE") {
        found.add(
            Level::Destructive,
            "sql",
            tr(
                "deletes every row of a table",
                "borra todas las filas de una tabla",
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(cmd: &str) -> Level {
        assess(cmd).level
    }

    fn rules(cmd: &str) -> Vec<&'static str> {
        assess(cmd).findings.iter().map(|f| f.rule).collect()
    }

    use Level::{Caution, Destructive, Ok};

    // ---- the false positives the plan asks for first

    #[test]
    fn a_build_directory_is_caution_not_destructive() {
        assert_eq!(level("rm -rf ./build"), Caution);
        assert_eq!(level("rm -rf build/"), Caution);
        assert_eq!(level("rm -rf node_modules"), Caution);
        assert_eq!(
            level("rm -rf /tmp/pistasho-test"),
            Caution,
            "not a system directory"
        );
        assert_eq!(
            level("rm -rf /var/lib/myapp/cache"),
            Caution,
            "inside /var, not /var itself"
        );
        assert_eq!(level("rm -r src/old"), Caution);
        assert_eq!(level("rm -rf {{path/to/directory}}"), Caution, "unknown target");
    }

    #[test]
    fn plain_commands_are_ok() {
        for cmd in [
            "ls -la",
            "rm file.txt",
            "rm -f file.txt",
            "rm *.o",
            "tar czf a.tgz dir",
            "grep -r foo .",
            "cat /etc/hosts",
            "dd if=/dev/zero of=file.img bs=1M count=10",
            "dd if=a of=/dev/null",
            "echo hi > /dev/null",
            "echo hi > out.txt",
            "git push origin feature",
            "git status",
            "chmod 644 file",
            "chmod +x script.sh",
            "psql -c 'select 1'",
            "docker ps",
            "docker run --rm alpine",
            "mkdir -p a/b",
            "",
        ] {
            assert_eq!(level(cmd), Ok, "{cmd}");
        }
    }

    // ---- destructive, one test per rule

    #[test]
    fn rm_on_the_root_the_home_or_a_glob() {
        for cmd in [
            "rm -rf /",
            "rm -rf /*",
            "sudo rm -rf /",
            "rm -rf ~",
            "rm -rf ~/",
            "rm -rf ~/*",
            "rm -rf $HOME",
            "rm -rf \"$HOME\"",
            "rm -rf ${HOME}/",
            "rm -rf *",
            "rm -rf .*",
            "rm -rf .",
            "rm -rf ..",
            "rm -rf /home",
            "rm -rf /etc",
            "rm -rf /usr/",
            "rm -rf /var",
            "rm -fr /",
            "rm -r -f /",
            "rm --recursive --force /",
            "rm -R /",
            "rm -r /",
            "rm -rf --no-preserve-root /anything",
            "cd /tmp && rm -rf /",
            "find . | xargs rm -rf /",
        ] {
            assert_eq!(level(cmd), Destructive, "{cmd}");
        }
    }

    #[test]
    fn rm_without_recursion_on_a_glob_is_caution() {
        assert_eq!(level("rm /*"), Caution);
        assert_eq!(level("rm -f *"), Caution);
    }

    #[test]
    fn dd_onto_a_device() {
        assert_eq!(level("sudo dd if=disk.iso of=/dev/sdb bs=4M"), Destructive);
        assert_eq!(level("dd if=/dev/zero of=/dev/nvme0n1"), Destructive);
        assert_eq!(
            level("dd bs=4M if=x.iso of={{/dev/usb_device}} status=progress"),
            Destructive
        );
        assert_eq!(level("dd if=x of={{device}}"), Destructive);
        assert_eq!(assess("dd if=a of=/dev/sdb").devices, vec!["/dev/sdb"]);
        assert_eq!(level("dd if=a of=/dev/null"), Ok);
        assert_eq!(level("dd if=a of=/dev/zero"), Ok);
    }

    #[test]
    fn formatting_commands() {
        for cmd in [
            "mkfs.ext4 /dev/sdb1",
            "sudo mkfs -t ext4 /dev/sdb1",
            "mkfs.vfat {{/dev/sdX}}",
            "mkfs.ntfs /dev/sdc1",
            "mke2fs /dev/sdb1",
            "mkswap /dev/sdb2",
            "mkfs.btrfs {{dispositivo}}",
        ] {
            assert_eq!(level(cmd), Destructive, "{cmd}");
        }
        assert_eq!(assess("sudo mkfs.ext4 /dev/sdb1").devices, vec!["/dev/sdb1"]);
    }

    #[test]
    fn wiping_commands() {
        assert_eq!(level("wipefs -a /dev/sdb"), Destructive);
        assert_eq!(level("shred -u secret.txt"), Destructive);
        assert_eq!(level("shred -vz /dev/sdb"), Destructive);
        assert_eq!(level("blkdiscard /dev/nvme1n1"), Destructive);
        assert_eq!(rules("wipefs -a /dev/sdb"), vec!["wipefs"]);
    }

    #[test]
    fn recursive_chmod_and_chown() {
        assert_eq!(level("chmod -R 777 /"), Destructive);
        assert_eq!(level("sudo chown -R me:me /etc"), Destructive);
        assert_eq!(level("chmod -R 755 /usr/*"), Destructive);
        assert_eq!(level("chown --recursive root /"), Destructive);
        // recursive, but not the system
        assert_eq!(level("chmod -R 755 ."), Caution);
        assert_eq!(level("chown -R me:me ~/project"), Caution);
        assert_eq!(level("chmod -R u+rwX {{path}}"), Caution);
        // the mode is not a target: `/` as a MODE does not make it destructive
        assert_eq!(level("chmod 644 /etc/hosts"), Ok);
    }

    #[test]
    fn redirecting_onto_a_device() {
        assert_eq!(level("cat image.iso > /dev/sdb"), Destructive);
        assert_eq!(level("echo x >/dev/sda"), Destructive);
        assert_eq!(level("cmd >> /dev/nvme0n1p2"), Destructive);
        assert_eq!(level("cat x > {{/dev/sdX}}"), Destructive);
        assert_eq!(level("cat x > /dev/null"), Ok);
        assert_eq!(level("cat x > /dev/tty"), Ok);
        assert_eq!(level("echo x > /etc/hosts"), Caution);
        assert_eq!(level("echo x | sudo tee /etc/hosts"), Ok, "tee is not a redirect");
    }

    #[test]
    fn fork_bombs() {
        assert_eq!(level(":(){ :|:& };:"), Destructive);
        assert_eq!(level(":(){:|:&};:"), Destructive);
        assert_eq!(level("bash -c ':(){ :|:& };:'"), Destructive);
        assert_eq!(
            level(":() { echo hi; }"),
            Destructive,
            "the plan flags the opening of a function named colon itself"
        );
    }

    #[test]
    fn git_history_rewrites() {
        assert_eq!(level("git push --force origin main"), Destructive);
        assert_eq!(level("git push -f origin master"), Destructive);
        assert_eq!(level("git push origin +main --force"), Destructive);
        assert_eq!(level("git push --force origin HEAD:main"), Destructive);
        assert_eq!(level("git push -f origin feature/x"), Caution);
        assert_eq!(level("git push --force"), Caution, "no branch named");
        assert_eq!(level("git push --force-with-lease origin main"), Caution);
        assert_eq!(level("git push origin main"), Ok);
        assert_eq!(
            level("git push origin +main"),
            Destructive,
            "a + refspec forces by itself"
        );
        assert_eq!(level("git push origin +feature"), Caution);
        assert_eq!(level("git reset --hard HEAD~1"), Caution);
        assert_eq!(level("git reset --soft HEAD~1"), Ok);
        assert_eq!(level("git clean -fd"), Caution);
        assert_eq!(level("git clean -n"), Ok);
        assert_eq!(level("git branch -D old"), Caution);
        assert_eq!(level("git branch -d old"), Ok);
    }

    #[test]
    fn destructive_sql() {
        assert_eq!(level("psql -c \"DROP TABLE users\""), Destructive);
        assert_eq!(level("mysql -e 'drop database shop'"), Destructive);
        assert_eq!(level("sqlite3 db 'DROP SCHEMA x'"), Destructive);
        assert_eq!(level("psql -c 'TRUNCATE users'"), Destructive);
        assert_eq!(level("mysql -e 'delete from orders'"), Destructive);
        assert_eq!(level("mysql -e 'DELETE FROM orders;'"), Destructive);
        // with a WHERE it is a normal statement
        assert_eq!(level("mysql -e 'delete from orders where id = 4'"), Ok);
        // words that merely contain the keywords
        assert_eq!(level("echo undrop tables"), Ok);
        // a search for SQL text is not SQL: only a SQL client reads it
        assert_eq!(level("grep 'drop table' notes.txt"), Ok);
        assert_eq!(level("echo 'DROP TABLE x' > notes.txt"), Ok);
        assert_eq!(level("echo 'DROP TABLE x' | psql"), Destructive);
    }

    // ---- caution

    #[test]
    fn other_commands_that_deserve_a_look() {
        assert_eq!(level("curl -fsSL https://x.sh | sh"), Caution);
        assert_eq!(level("wget -qO- https://x.sh | sudo bash"), Caution);
        assert_eq!(level("curl https://x.sh -o x.sh"), Ok);
        assert_eq!(level("curl x | jq ."), Ok);
        assert_eq!(level("find . -name '*.tmp' -delete"), Caution);
        assert_eq!(level("find . -exec rm {} +"), Caution);
        assert_eq!(level("find . -name x"), Ok);
        assert_eq!(level("sudo reboot"), Caution);
        assert_eq!(level("shutdown -h now"), Caution);
        assert_eq!(level("docker system prune -a"), Caution);
        assert_eq!(level("docker rm -f web"), Caution);
        assert_eq!(level("kubectl delete pod web"), Caution);
        assert_eq!(level("terraform destroy"), Caution);
        assert_eq!(level("crontab -r"), Caution);
        assert_eq!(level("crontab -l"), Ok);
        assert_eq!(level("sudo iptables -F"), Caution);
        assert_eq!(level("userdel -r bob"), Caution);
    }

    // ---- structure

    #[test]
    fn the_worst_finding_wins_and_findings_are_listed() {
        let a = assess("rm -rf build && mkfs.ext4 /dev/sdb1");
        assert_eq!(a.level, Destructive);
        assert_eq!(a.findings.len(), 2);
        assert_eq!(a.rules(), "rm, mkfs");
        // only the worst level's sentences are the effect
        assert!(a.effect().contains("formats"), "{}", a.effect());
        assert!(!a.effect().contains("deletes the folder"), "{}", a.effect());
    }

    #[test]
    fn wrappers_do_not_hide_the_command() {
        assert_eq!(level("sudo rm -rf /"), Destructive);
        assert_eq!(level("env FOO=1 rm -rf /"), Destructive);
        assert_eq!(level("nohup rm -rf / &"), Destructive);
        assert_eq!(level("echo $(rm -rf /)"), Destructive);
        assert_eq!(level("true && sudo mkfs.ext4 /dev/sdb"), Destructive);
    }

    #[test]
    fn names_and_labels() {
        assert_eq!(Destructive.name(), "destructive");
        assert_eq!(Caution.name(), "caution");
        assert_eq!(Ok.name(), "ok");
        assert!(Ok < Caution && Caution < Destructive);
    }

    #[test]
    fn a_decision_model_can_raise_the_level_but_never_lower_it() {
        let ok = assess("echo hola");
        assert_eq!(
            with_model(ok.clone(), 0.2).level,
            Ok,
            "a low probability changes nothing"
        );
        assert_eq!(with_model(ok.clone(), 0.35).level, Caution);
        let high = with_model(ok, 0.9);
        assert_eq!(high.level, Destructive);
        assert!(high.findings.iter().any(|f| f.rule == "model"));
        // the rules said destructive: a model that disagrees does not soften it
        let rules = assess("mkfs.ext4 /dev/sdb1");
        assert_eq!(rules.level, Destructive);
        assert_eq!(with_model(rules, 0.0).level, Destructive);
        // caution plus a middling probability stays caution; a high one escalates
        let caution = assess("rm -rf ./build");
        assert_eq!(
            with_model(caution.clone(), 0.55).level,
            Caution,
            "a plain rm is 0.55: no red box"
        );
        assert_eq!(with_model(caution, 0.7).level, Destructive);
    }

    #[test]
    fn odd_input_does_not_panic() {
        for cmd in [
            "", "   ", "|", "&&", "rm", "rm -", "rm --", "dd of=", "git", "git push", "chmod", "chmod -R",
            "echo \"", ">",
        ] {
            let _ = assess(cmd);
        }
        assert_eq!(level("rm"), Ok);
        assert_eq!(level("chmod -R"), Ok, "nothing to change");
    }
}
