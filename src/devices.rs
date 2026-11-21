//! Block devices: telling them apart in a command, and what `lsblk` says about them.
//!
//! Two uses: the risk rules need to know that `/dev/sdb1` is a disk and `/dev/null` is not, and
//! the destructive-command box shows what the device is (model, size, where it is mounted).
//! When a command asks for a device, only removable ones are offered, so a typo or a hasty Enter
//! cannot point `mkfs` at the system disk.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::{Command, Stdio};

/// `/dev/sdb`, `/dev/sdb1`, `/dev/nvme0n1p2`, `/dev/mmcblk0`, `/dev/vda`, `/dev/md0`,
/// `/dev/loop0`, `/dev/dm-0`, `/dev/disk/by-id/…`, `/dev/mapper/…`; also the `/dev/sdX` of
/// documentation. Not `/dev/null`, `/dev/zero` or the terminals.
pub fn is_block_device(path: &str) -> bool {
    let Some(name) = path.strip_prefix("/dev/") else {
        return false;
    };
    if name.starts_with("disk/") || name.starts_with("mapper/") {
        return true;
    }
    let all = |s: &str, f: fn(char) -> bool| !s.is_empty() && s.chars().all(f);
    let letters_then_digits = |rest: &str| {
        let letters = rest.trim_end_matches(|c: char| c.is_ascii_digit());
        all(letters, |c| c.is_ascii_alphabetic())
    };
    for prefix in ["sd", "hd", "vd", "xvd"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return letters_then_digits(rest);
        }
    }
    if let Some(rest) = name.strip_prefix("nvme") {
        // nvme0n1, nvme0n1p2
        let (ctrl, ns) = rest.split_once('n').unwrap_or(("", ""));
        let (ns, part) = ns.split_once('p').map_or((ns, "1"), |(n, p)| (n, p));
        return all(ctrl, |c| c.is_ascii_digit())
            && all(ns, |c| c.is_ascii_digit())
            && all(part, |c| c.is_ascii_digit());
    }
    if let Some(rest) = name.strip_prefix("mmcblk") {
        let (disk, part) = rest.split_once('p').map_or((rest, "1"), |(d, p)| (d, p));
        return all(disk, |c| c.is_ascii_digit()) && all(part, |c| c.is_ascii_digit());
    }
    for prefix in ["md", "loop", "dm-"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return all(rest, |c| c.is_ascii_digit());
        }
    }
    false
}

/// A `{{placeholder}}` that stands for a device: `{{dispositivo}}`, `{{device}}`,
/// `{{/dev/sdX}}`, `{{usb_device}}`, `{{partition}}`…
pub fn is_device_placeholder(s: &str) -> bool {
    let Some(inner) = s.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")) else {
        return false;
    };
    placeholder_names_a_device(inner)
}

/// The name inside a placeholder says it is a device.
pub fn placeholder_names_a_device(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("/dev/")
        || [
            "device",
            "dispositivo",
            "disk",
            "disco",
            "partition",
            "partici",
            "usb",
            "drive",
            "unidad",
            "block_dev",
        ]
        .iter()
        .any(|k| n.contains(k))
}

// ---------------------------------------------------------------- lsblk

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// `/dev/sdb1`
    pub path: String,
    /// `sdb1`
    pub name: String,
    pub model: Option<String>,
    pub size: String,
    pub fstype: Option<String>,
    pub mountpoint: Option<String>,
    pub removable: bool,
    /// A partition of another device (its model and removability come from that one).
    pub is_partition: bool,
}

impl Device {
    /// One line to recognise it by: `/dev/sdb1  SanDisk Ultra  29.8G  vfat  /media/pk/USB`.
    pub fn describe(&self) -> String {
        let mut parts = vec![self.path.clone()];
        parts.extend(self.model.clone());
        parts.push(self.size.clone());
        parts.extend(self.fstype.clone());
        parts.extend(self.mountpoint.clone());
        parts.join("  ")
    }
}

/// `lsblk -J` prints `rm` as a boolean in recent versions and as `"0"` / `"1"` in older ones.
#[derive(Deserialize)]
#[serde(untagged)]
enum Flag {
    Bool(bool),
    Num(u8),
    Text(String),
}

impl Flag {
    fn on(&self) -> bool {
        match self {
            Flag::Bool(b) => *b,
            Flag::Num(n) => *n != 0,
            Flag::Text(t) => t == "1" || t.eq_ignore_ascii_case("true"),
        }
    }
}

#[derive(Deserialize)]
struct Node {
    name: String,
    path: Option<String>,
    model: Option<String>,
    size: Option<String>,
    fstype: Option<String>,
    mountpoint: Option<String>,
    rm: Option<Flag>,
    #[serde(default)]
    children: Vec<Node>,
}

#[derive(Deserialize)]
struct Tree {
    blockdevices: Vec<Node>,
}

/// Reads the JSON of `lsblk -J -o NAME,MODEL,SIZE,FSTYPE,MOUNTPOINT,RM,PATH` into a flat list,
/// disks first and then their partitions. Partitions inherit the model and the `removable`
/// flag of their disk when lsblk leaves them blank.
pub fn parse_lsblk(json: &str) -> Result<Vec<Device>> {
    let tree: Tree = serde_json::from_str(json).context("lsblk output")?;
    let mut out = Vec::new();
    for disk in &tree.blockdevices {
        let removable = disk.rm.as_ref().is_some_and(Flag::on);
        out.push(device(disk, removable, None, false));
        flatten(disk, removable, disk.model.as_deref(), &mut out);
    }
    Ok(out)
}

fn flatten(parent: &Node, removable: bool, model: Option<&str>, out: &mut Vec<Device>) {
    for child in &parent.children {
        out.push(device(child, removable, model, true));
        flatten(child, removable, model, out);
    }
}

fn device(n: &Node, removable: bool, inherited_model: Option<&str>, is_partition: bool) -> Device {
    let clean = |s: &Option<String>| s.as_ref().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    Device {
        path: n.path.clone().unwrap_or_else(|| format!("/dev/{}", n.name)),
        name: n.name.clone(),
        model: clean(&n.model).or_else(|| {
            inherited_model
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
        }),
        size: n.size.clone().unwrap_or_default(),
        fstype: clean(&n.fstype),
        mountpoint: clean(&n.mountpoint),
        removable,
        is_partition,
    }
}

/// Asks `lsblk`. Read-only, no root needed.
pub fn lsblk() -> Result<Vec<Device>> {
    let out = Command::new("lsblk")
        .args(["-J", "-o", "NAME,MODEL,SIZE,FSTYPE,MOUNTPOINT,RM,PATH"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .context("could not run lsblk")?;
    parse_lsblk(&String::from_utf8_lossy(&out.stdout))
}

/// The removable devices worth offering: USB sticks, SD cards. Whole disks are listed only when
/// they have no partitions (a blank stick); otherwise their partitions are. Loop and RAM devices
/// are not devices anyone means.
pub fn removable(all: &[Device]) -> Vec<&Device> {
    let has_partitions = |disk: &Device| {
        all.iter()
            .any(|d| d.is_partition && d.path != disk.path && d.path.starts_with(&disk.path))
    };
    all.iter()
        .filter(|d| d.removable)
        .filter(|d| !d.name.starts_with("loop") && !d.name.starts_with("ram") && !d.name.starts_with("zram"))
        .filter(|d| d.is_partition || !has_partitions(d))
        .collect()
}

/// Names of the whole disks that are not offered because they are not removable
/// (`sda`, `nvme0n1`), for the note under the list.
pub fn hidden_disks(all: &[Device]) -> Vec<&str> {
    all.iter()
        .filter(|d| !d.is_partition && !d.removable)
        .filter(|d| !d.name.starts_with("loop") && !d.name.starts_with("ram") && !d.name.starts_with("zram"))
        .map(|d| d.name.as_str())
        .collect()
}

/// The device at `path`, if lsblk knows it.
pub fn find<'a>(all: &'a [Device], path: &str) -> Option<&'a Device> {
    all.iter().find(|d| d.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_block_devices() {
        for yes in [
            "/dev/sda",
            "/dev/sdb1",
            "/dev/sdz",
            "/dev/hda",
            "/dev/vda2",
            "/dev/xvda",
            "/dev/nvme0n1",
            "/dev/nvme0n1p2",
            "/dev/nvme12n3p10",
            "/dev/mmcblk0",
            "/dev/mmcblk0p1",
            "/dev/md0",
            "/dev/loop3",
            "/dev/dm-0",
            "/dev/disk/by-id/usb-x",
            "/dev/mapper/vg-root",
            "/dev/sdX",
            "/dev/sdX1",
        ] {
            assert!(is_block_device(yes), "{yes}");
        }
        for no in [
            "/dev/null",
            "/dev/zero",
            "/dev/tty",
            "/dev/stdout",
            "/dev/random",
            "/dev/pts/0",
            "/dev/",
            "/dev",
            "sda",
            "/tmp/sda",
            "/dev/sd",
            "/dev/nvme",
            "/dev/nvme0",
            "/dev/mmcblk",
            "/dev/md",
            "/dev/sd1",
            "",
        ] {
            assert!(!is_block_device(no), "{no}");
        }
    }

    #[test]
    fn recognises_device_placeholders() {
        for yes in [
            "{{dispositivo}}",
            "{{device}}",
            "{{/dev/sdX}}",
            "{{usb_device}}",
            "{{partition}}",
            "{{disco}}",
            "{{particion}}",
            "{{/dev/usb_device}}",
            "{{target_drive}}",
            "{{unidad}}",
        ] {
            assert!(is_device_placeholder(yes), "{yes}");
        }
        for no in [
            "{{file}}",
            "{{path/to/file}}",
            "{{archivo}}",
            "dispositivo",
            "{{host}}",
            "{{dir}}",
            "{{puerto}}",
        ] {
            assert!(!is_device_placeholder(no), "{no}");
        }
    }

    const NEW_FORMAT: &str = r#"{"blockdevices":[
      {"name":"loop0","model":null,"size":"74M","fstype":"squashfs","mountpoint":"/snap/core22/1","rm":false,"path":"/dev/loop0"},
      {"name":"sda","model":"Samsung SSD 860","size":"500G","fstype":null,"mountpoint":null,"rm":false,"path":"/dev/sda",
       "children":[{"name":"sda1","model":null,"size":"500G","fstype":"ext4","mountpoint":"/","rm":false,"path":"/dev/sda1"}]},
      {"name":"sdb","model":"SanDisk Ultra  ","size":"29.8G","fstype":null,"mountpoint":null,"rm":true,"path":"/dev/sdb",
       "children":[{"name":"sdb1","model":null,"size":"29.8G","fstype":"vfat","mountpoint":"/media/pk/USB","rm":true,"path":"/dev/sdb1"}]},
      {"name":"sdc","model":"Blank Stick","size":"16G","fstype":null,"mountpoint":null,"rm":true,"path":"/dev/sdc"},
      {"name":"nvme0n1","model":"WD Black","size":"1T","fstype":null,"mountpoint":null,"rm":false,"path":"/dev/nvme0n1",
       "children":[{"name":"nvme0n1p1","model":null,"size":"512M","fstype":"vfat","mountpoint":"/boot/efi","rm":false,"path":"/dev/nvme0n1p1"}]}
    ]}"#;

    /// Older util-linux prints `rm` as "0" / "1" and may leave out `path`.
    const OLD_FORMAT: &str = r#"{"blockdevices":[
      {"name":"sdb","model":"Cruzer","size":"7.5G","fstype":null,"mountpoint":null,"rm":"1",
       "children":[{"name":"sdb1","model":null,"size":"7.5G","fstype":"vfat","mountpoint":null,"rm":"1"}]},
      {"name":"sda","model":"HDD","size":"1T","fstype":null,"mountpoint":null,"rm":"0"}
    ]}"#;

    #[test]
    fn parses_lsblk_and_partitions_inherit_from_their_disk() {
        let all = parse_lsblk(NEW_FORMAT).unwrap();
        assert_eq!(all.len(), 8);
        let sdb1 = find(&all, "/dev/sdb1").unwrap();
        assert!(sdb1.is_partition && sdb1.removable);
        assert_eq!(
            sdb1.model.as_deref(),
            Some("SanDisk Ultra"),
            "the disk's model, trimmed"
        );
        assert_eq!(sdb1.fstype.as_deref(), Some("vfat"));
        assert_eq!(sdb1.mountpoint.as_deref(), Some("/media/pk/USB"));
        assert_eq!(
            sdb1.describe(),
            "/dev/sdb1  SanDisk Ultra  29.8G  vfat  /media/pk/USB"
        );
        assert!(!find(&all, "/dev/sda1").unwrap().removable);
        assert_eq!(find(&all, "/dev/nope"), None);
    }

    #[test]
    fn parses_the_older_format() {
        let all = parse_lsblk(OLD_FORMAT).unwrap();
        let sdb1 = find(&all, "/dev/sdb1").expect("path built from the name");
        assert!(sdb1.removable, "rm \"1\" means removable");
        assert!(!find(&all, "/dev/sda").unwrap().removable);
    }

    /// The GIF case: of everything on the machine only the USB stick is offered.
    #[test]
    fn only_removable_devices_are_offered() {
        let all = parse_lsblk(NEW_FORMAT).unwrap();
        let offered: Vec<&str> = removable(&all).iter().map(|d| d.path.as_str()).collect();
        // the stick's partition (not the disk that has partitions) and the blank stick
        assert_eq!(offered, vec!["/dev/sdb1", "/dev/sdc"]);
        assert!(!offered
            .iter()
            .any(|p| p.contains("sda") || p.contains("nvme") || p.contains("loop")));
    }

    #[test]
    fn the_hidden_disks_are_named_for_the_note() {
        let all = parse_lsblk(NEW_FORMAT).unwrap();
        assert_eq!(hidden_disks(&all), vec!["sda", "nvme0n1"]);
    }

    #[test]
    fn nothing_removable_means_nothing_offered() {
        let json = r#"{"blockdevices":[{"name":"sda","model":"x","size":"1T","fstype":null,"mountpoint":null,"rm":false,"path":"/dev/sda"}]}"#;
        let all = parse_lsblk(json).unwrap();
        assert!(removable(&all).is_empty());
        assert_eq!(hidden_disks(&all), vec!["sda"]);
    }

    #[test]
    fn bad_json_is_an_error_not_a_panic() {
        assert!(parse_lsblk("").is_err());
        assert!(parse_lsblk("not json").is_err());
        assert!(parse_lsblk("{}").is_err());
        assert_eq!(parse_lsblk(r#"{"blockdevices":[]}"#).unwrap(), vec![]);
    }
}
