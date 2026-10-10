//! Read-only launch arguments. Filesystem identity checks belong to Runtime.
use anyhow::{Context as _, Result, bail, ensure};
use disktree_core::scan_threads::ScanThreads;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub(crate) const MAX_ROOT_BYTES: usize = 16_384;
pub(crate) const MAX_ROOT_COMPONENTS: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchOverrides {
    pub root: Option<String>,
    pub apparent_size: Option<bool>,
    pub show_hidden: Option<bool>,
    pub depth: Option<u8>,
    /// 0 means bytes/size; 1 means files (independent of Runtime's metric codes).
    pub metric: Option<u8>,
    pub threads: Option<ThreadPolicy>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadPolicy {
    pub max_threads: u32,
    pub adaptive: bool,
    pub retained_throughput_percent: u8,
    /// Zero disables the governor.
    pub system_cpu_percent: u8,
}

#[derive(Debug, Default)]
pub struct Cli {
    pub data_dir: Option<PathBuf>,
    pub background: bool,
    pub verification: Option<PathBuf>,
    pub help: bool,
    pub launch: LaunchOverrides,
    pub disk: bool,
}

pub const USAGE: &str = "DiskBurrow [OPTIONS] [LOCAL_DIRECTORY]\n\
  -D, --disk                       scan the system volume\n\
  -a, --apparent-size              measure apparent length\n\
  -H, --no-hidden                  hide hidden entries\n\
  -d, --depth N                    draw 1..6 levels\n\
      --metric bytes|size|files    select map metric\n\
  -x, --one-filesystem             stay on the selected volume\n\
      --power-efficiency PRESET    miser|balanced|aggressive|drain-my-battery\n\
      --scan-threads N             positive CPU-capped worker count\n\
      --adaptive-threads           enable adaptive admission\n\
      --fixed-threads              disable adaptive admission and governor\n\
      --thread-throughput-percent N 1..100\n\
      --thread-system-cpu-percent N 0..100 (0 disables governor)\n\
      --scan-root LOCAL_DIRECTORY  alternate launch root\n\
      --data-dir ABSOLUTE_DIRECTORY\n\
      --background\n\
      --verify-runtime NEW_ABSOLUTE_REPORT\n\
  -h, --help\n\
  --                              end options\n\
Following links and crossing filesystems are unsupported.";

/// Parse Unicode arguments after the program name, without filesystem reads.
pub fn parse(args: &[String]) -> Result<Cli> {
    let mut cli = Cli::default();
    let mut seen = HashSet::new();
    let mut positional = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        i += 1;
        if !positional && arg == "--" {
            positional = true;
            continue;
        }
        let key = if positional {
            "root"
        } else {
            match arg {
                "-h" | "--help" => "help",
                "-a" | "--apparent-size" => "apparent",
                "-H" | "--no-hidden" => "hidden",
                "-D" | "--disk" => "disk",
                "-d" | "--depth" => "depth",
                "-x" | "--one-filesystem" => "filesystem",
                "--scan-root" => "root-option",
                "--data-dir" => "data",
                "--background" => "background",
                "--verify-runtime" => "verification",
                "--metric" => "metric",
                "--power-efficiency" => "power",
                "--scan-threads" => "workers",
                "--adaptive-threads" | "--fixed-threads" => "admission",
                "--thread-throughput-percent" => "throughput",
                "--thread-system-cpu-percent" => "cpu",
                "-l" | "--follow-links" => bail!("{arg} is unsupported: scans never follow links"),
                "-X" | "--cross-filesystems" => {
                    bail!("{arg} is unsupported: scans stay on one local volume")
                }
                other if other.starts_with('-') => bail!("Unknown argument: {other}"),
                _ => "root",
            }
        };
        ensure!(seen.insert(key), "Duplicate or conflicting argument: {arg}");
        match key {
            "help" => cli.help = true,
            "apparent" => cli.launch.apparent_size = Some(true),
            "hidden" => cli.launch.show_hidden = Some(false),
            "disk" => cli.disk = true,
            "depth" => cli.launch.depth = Some(percent(value(args, &mut i, arg)?, 1, 6, arg)?),
            "metric" => {
                cli.launch.metric = Some(match value(args, &mut i, arg)? {
                    "bytes" | "size" => 0,
                    "files" => 1,
                    other => bail!("Unknown metric: {other}; use bytes, size or files"),
                })
            }
            "filesystem" => {}
            "root" | "root-option" => {
                ensure!(cli.launch.root.is_none(), "Only one scan root is allowed");
                let root = if key == "root" {
                    arg
                } else {
                    value(args, &mut i, arg)?
                };
                validate_path_text(root)?;
                cli.launch.root = Some(root.to_owned());
            }
            "data" => cli.data_dir = Some(PathBuf::from(value(args, &mut i, arg)?)),
            "background" => cli.background = true,
            "verification" => cli.verification = Some(PathBuf::from(value(args, &mut i, arg)?)),
            "power" => {
                let max_threads = match value(args, &mut i, arg)? {
                    "miser" => 2,
                    "balanced" => 4,
                    "aggressive" => 8,
                    "drain-my-battery" => u32::MAX,
                    other => bail!("Unknown power efficiency preset: {other}"),
                };
                cli.launch.threads = Some(ThreadPolicy {
                    max_threads,
                    ..ThreadPolicy::default()
                });
            }
            "workers" => {
                let count: u64 = value(args, &mut i, arg)?
                    .parse()
                    .context("--scan-threads needs a positive integer")?;
                ensure!(count > 0, "--scan-threads must be positive");
                cli.launch.threads.get_or_insert_default().max_threads =
                    count.min(u64::from(u32::MAX)) as u32;
            }
            "admission" => {
                let policy = cli.launch.threads.get_or_insert_default();
                policy.adaptive = arg == "--adaptive-threads";
                policy.system_cpu_percent = if policy.adaptive { 80 } else { 0 };
            }
            "throughput" => {
                cli.launch
                    .threads
                    .get_or_insert_default()
                    .retained_throughput_percent = percent(value(args, &mut i, arg)?, 1, 100, arg)?
            }
            "cpu" => {
                cli.launch
                    .threads
                    .get_or_insert_default()
                    .system_cpu_percent = percent(value(args, &mut i, arg)?, 0, 100, arg)?
            }
            _ => unreachable!(),
        }
    }
    ensure!(
        !(cli.disk && cli.launch.root.is_some()),
        "--disk conflicts with an explicit scan root"
    );
    Ok(cli)
}
impl Cli {
    /// Resolve in the launching process before IPC; existence/identity remain Runtime checks.
    pub fn resolve(&mut self, cwd: &Path, system_root: &str) -> Result<()> {
        if self.disk {
            self.launch.root = Some(normalize_absolute(system_root)?);
        }
        if let Some(root) = &self.launch.root {
            self.launch.root = Some(if is_absolute(root) {
                normalize_absolute(root)?
            } else {
                let cwd = cwd
                    .to_str()
                    .context("Launching directory must be Unicode")?;
                let cwd = normalize_absolute(cwd)?;
                normalize_absolute(&format!("{cwd}\\{root}"))?
            });
        }
        if let Some(policy) = &mut self.launch.threads {
            let cpus = std::thread::available_parallelism().map_or(1, usize::from);
            policy.max_threads = policy.max_threads.min(cpus.min(u32::MAX as usize) as u32);
        }
        self.launch.validate()
    }
}
impl LaunchOverrides {
    pub fn is_explicit(&self) -> bool {
        self.root.is_some()
            || self.apparent_size.is_some()
            || self.show_hidden.is_some()
            || self.depth.is_some()
            || self.metric.is_some()
            || self.threads.is_some()
    }
    /// Revalidate untrusted IPC values, requiring sender-resolved canonical local paths.
    pub fn validate(&self) -> Result<()> {
        if let Some(root) = &self.root {
            validate_absolute_root(root)?;
        }
        ensure!(
            self.depth.is_none_or(|depth| (1..=6).contains(&depth)),
            "Depth must be 1 to 6"
        );
        ensure!(
            self.metric.is_none_or(|metric| metric <= 1),
            "Metric must be bytes or files"
        );
        if let Some(policy) = self.threads {
            policy.validate()?;
        }
        Ok(())
    }
}
impl Default for ThreadPolicy {
    fn default() -> Self {
        Self {
            max_threads: 4,
            adaptive: false,
            retained_throughput_percent: 80,
            system_cpu_percent: 0,
        }
    }
}
impl ThreadPolicy {
    fn validate(self) -> Result<()> {
        ensure!(self.max_threads > 0, "Worker count must be positive");
        ensure!(
            (1..=100).contains(&self.retained_throughput_percent),
            "Throughput percent must be 1 to 100"
        );
        ensure!(
            self.system_cpu_percent <= 100,
            "CPU percent must be 0 to 100"
        );
        Ok(())
    }
    /// Re-cap to the receiver's CPU count before starting an ordinary scan.
    pub fn to_scan_threads(self, cpus: usize) -> Result<ScanThreads> {
        self.validate()?;
        Ok(ScanThreads {
            max_threads: (self.max_threads as usize).min(cpus.max(1)),
            adaptive: self.adaptive,
            retained_throughput: f64::from(self.retained_throughput_percent) / 100.0,
            system_cpu_limit: (self.system_cpu_percent > 0)
                .then(|| f64::from(self.system_cpu_percent) / 100.0),
        })
    }
}

fn value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str> {
    let value = args
        .get(*i)
        .with_context(|| format!("Missing value for {flag}"))?;
    ensure!(
        !value.is_empty() && !value.starts_with('-'),
        "Missing or invalid value for {flag}"
    );
    *i += 1;
    Ok(value)
}
fn percent(value: &str, min: u8, max: u8, flag: &str) -> Result<u8> {
    let percent: u8 = value
        .parse()
        .with_context(|| format!("{flag} requires {min} to {max}"))?;
    ensure!(
        (min..=max).contains(&percent),
        "{flag} requires {min} to {max}"
    );
    Ok(percent)
}
fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}
fn validate_path_text(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= MAX_ROOT_BYTES && path.encode_utf16().count() <= 32767,
        "Invalid or oversized local path"
    );
    ensure!(
        !path.starts_with(['/', '\\']),
        "Root-relative, UNC and device paths are unsupported"
    );
    let tail = if is_absolute(path) {
        &path[3..]
    } else {
        ensure!(
            !path.contains(':'),
            "Drive-relative paths and streams are unsupported"
        );
        path
    };
    ensure!(
        tail.split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .count()
            <= MAX_ROOT_COMPONENTS,
        "Local paths support at most {MAX_ROOT_COMPONENTS} components"
    );
    for component in tail.split(['/', '\\']) {
        if matches!(component, "" | "." | "..") {
            continue;
        }
        ensure!(
            !component.ends_with(['.', ' ']),
            "Windows path components cannot end in a dot or space"
        );
        ensure!(
            !component
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, ':' | '<' | '>' | '"' | '|' | '?' | '*')),
            "Invalid Windows path component"
        );
        let stem = component
            .split('.')
            .next()
            .unwrap_or_default()
            .to_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        ensure!(!reserved, "Windows device names are unsupported");
    }
    Ok(())
}
/// Bound and validate the canonical root before platform metadata or IPC admission.
pub(crate) fn validate_absolute_root(root: &str) -> Result<()> {
    ensure!(
        normalize_absolute(root)? == root,
        "Launch root must be normalized and absolute"
    );
    Ok(())
}
fn normalize_absolute(path: &str) -> Result<String> {
    validate_path_text(path)?;
    ensure!(
        is_absolute(path),
        "An absolute local drive path is required"
    );
    let mut parts = Vec::new();
    for part in path[3..].split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                ensure!(parts.pop().is_some(), "Path escapes the volume root");
            }
            _ => parts.push(part),
        }
    }
    Ok(format!(
        "{}:\\{}",
        (path.as_bytes()[0] as char).to_ascii_uppercase(),
        parts.join("\\")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }
    fn parsed(values: &[&str]) -> Cli {
        parse(&args(values)).unwrap()
    }

    #[test]
    fn positional_unicode_spaces_resolve_in_sender_directory_without_metadata() {
        let mut cli = parsed(&[r"..\Лес мастера\不存在"]);
        cli.resolve(Path::new(r"E:\owned\work"), r"C:\").unwrap();
        assert_eq!(
            cli.launch.root.as_deref(),
            Some(r"E:\owned\Лес мастера\不存在")
        );
        assert!(cli.launch.is_explicit());
        cli.launch.validate().unwrap();
    }
    #[test]
    fn absolute_paths_normalize_separator_dots_and_drive_case() {
        let mut cli = parsed(&["e:/owned/./one/../two/"]);
        cli.resolve(Path::new(r"C:\elsewhere"), r"C:\").unwrap();
        assert_eq!(cli.launch.root.as_deref(), Some(r"E:\owned\two"));
    }
    #[test]
    fn root_depth_and_utf8_bounds_apply_before_resolution_and_ipc() {
        let at_limit = format!("E:\\{}", vec!["a"; 64].join("\\"));
        let beyond_limit = format!("{at_limit}\\a");
        let bytes_at_limit = format!("E:\\{}", "a".repeat(16381));
        let unicode_over_limit = format!("E:\\{}", "界".repeat(5461));
        assert_eq!(bytes_at_limit.len(), 16384);
        assert!(unicode_over_limit.encode_utf16().count() < 32767);
        for root in [&at_limit, &bytes_at_limit] {
            let mut cli = parsed(&[root]);
            cli.resolve(Path::new(r"E:\owned"), r"C:\").unwrap();
            cli.launch.validate().unwrap();
        }
        for root in [&beyond_limit, &unicode_over_limit] {
            assert!(parse(&args(&[root])).is_err());
            assert!(
                LaunchOverrides {
                    root: Some(root.clone()),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        let mut relative = parsed(&[&vec!["a"; 64].join("\\")]);
        assert!(relative.resolve(Path::new(r"E:\owned"), r"C:\").is_err());
    }
    #[test]
    fn separator_allows_dash_directory_and_disk_resolves_system_volume() {
        let mut cli = parsed(&["--", "-folder"]);
        cli.resolve(Path::new(r"E:\owned"), r"C:\").unwrap();
        assert_eq!(cli.launch.root.as_deref(), Some(r"E:\owned\-folder"));
        let mut disk = parsed(&["-D"]);
        disk.resolve(Path::new(r"E:\owned"), r"c:/").unwrap();
        assert_eq!(disk.launch.root.as_deref(), Some(r"C:\"));
    }
    #[test]
    fn empty_launch_preserves_saved_settings_and_legacy_fields_are_local_only() {
        assert!(!parsed(&[]).launch.is_explicit());
        let cli = parsed(&[
            "--data-dir",
            r"E:\settings",
            "--background",
            "--verify-runtime",
            r"E:\proof.json",
            "--scan-root",
            r"E:\fixture",
        ]);
        assert_eq!(cli.data_dir, Some(PathBuf::from(r"E:\settings")));
        assert_eq!(cli.verification, Some(PathBuf::from(r"E:\proof.json")));
        assert!(cli.background);
        assert_eq!(cli.launch.root.as_deref(), Some(r"E:\fixture"));
        let json = serde_json::to_string(&cli.launch).unwrap();
        assert!(!json.contains("settings"));
        assert!(!json.contains("proof.json"));
    }
    #[test]
    fn display_options_accept_aliases_and_range_boundaries() {
        let cli = parsed(&["-a", "-H", "-d", "1", "--metric", "files", "-x"]);
        assert_eq!(cli.launch.apparent_size, Some(true));
        assert_eq!(cli.launch.show_hidden, Some(false));
        assert_eq!(cli.launch.depth, Some(1));
        assert_eq!(cli.launch.metric, Some(1));
        assert_eq!(
            parsed(&["--depth", "6", "--metric", "bytes"]).launch.depth,
            Some(6)
        );
        assert_eq!(parsed(&["--metric", "size"]).launch.metric, Some(0));
        assert!(parsed(&["--apparent-size"]).launch.is_explicit());
        assert!(parsed(&["--no-hidden"]).launch.is_explicit());
        assert!(parsed(&["-h"]).help);
        assert!(parsed(&["--help"]).help);
    }
    #[test]
    fn malformed_conflicting_duplicate_and_unsafe_arguments_fail() {
        for values in [
            vec!["--unknown"],
            vec!["--depth"],
            vec!["--depth", "--help"],
            vec!["--depth", "0"],
            vec!["--depth", "7"],
            vec!["--depth", "-1"],
            vec!["--metric", "age"],
            vec!["--scan-root"],
            vec!["--data-dir"],
            vec!["--verify-runtime"],
            vec!["one", "two"],
            vec!["one", "--disk"],
            vec!["--disk", "--scan-root", "one"],
            vec!["--scan-root", "one", "two"],
            vec!["-a", "--apparent-size"],
            vec!["-d", "1", "--depth", "2"],
            vec!["--background", "--background"],
            vec!["--adaptive-threads", "--fixed-threads"],
            vec!["-l"],
            vec!["--follow-links"],
            vec!["-X"],
            vec!["--cross-filesystems"],
            vec!["--mft-helper"],
            vec!["--scan-threads", "0"],
            vec!["--scan-threads", "-1"],
            vec!["--power-efficiency", "unknown"],
            vec!["--thread-throughput-percent", "0"],
            vec!["--thread-throughput-percent", "101"],
            vec!["--thread-system-cpu-percent", "101"],
        ] {
            assert!(parse(&args(&values)).is_err(), "accepted {values:?}");
        }
    }
    #[test]
    fn unsafe_windows_roots_are_rejected_before_forwarding() {
        for root in [
            r"E:relative",
            r"\owned",
            r"\\server\share",
            r"\\?\E:\owned",
            r"\\.\E:\",
            "E:/owned/name:stream",
            "E:/owned/space ",
            "E:/owned/dot.",
            "E:/owned/NUL",
            "E:/owned/con.txt",
            "E:/owned/COM1",
            "E:/owned/a*",
            "",
            "E:/../../outside",
            "E:/owned/\0name",
        ] {
            let result = parse(&args(&[root]))
                .and_then(|mut cli| cli.resolve(Path::new(r"E:\owned"), r"C:\"));
            assert!(result.is_err(), "accepted {root:?}");
        }
    }
    #[test]
    fn presets_and_explicit_worker_counts_are_bounded_on_receiver() {
        for (name, want) in [
            ("miser", 2),
            ("balanced", 4),
            ("aggressive", 8),
            ("drain-my-battery", 16),
        ] {
            let policy = parsed(&["--power-efficiency", name])
                .launch
                .threads
                .unwrap();
            let threads = policy.to_scan_threads(16).unwrap();
            assert_eq!(threads.max_threads, want);
            assert!(!threads.adaptive);
            assert_eq!(threads.system_cpu_limit, None);
            assert_eq!(policy.to_scan_threads(1).unwrap().max_threads, 1);
        }
        let threads = parsed(&[
            "--scan-threads",
            "999",
            "--adaptive-threads",
            "--thread-throughput-percent",
            "1",
            "--thread-system-cpu-percent",
            "100",
        ])
        .launch
        .threads
        .unwrap()
        .to_scan_threads(12)
        .unwrap();
        assert_eq!(threads.max_threads, 12);
        assert!(threads.adaptive);
        assert_eq!(threads.retained_throughput, 0.01);
        assert_eq!(threads.system_cpu_limit, Some(1.0));
        let fixed = parsed(&[
            "--fixed-threads",
            "--thread-throughput-percent",
            "100",
            "--thread-system-cpu-percent",
            "0",
        ])
        .launch
        .threads
        .unwrap()
        .to_scan_threads(0)
        .unwrap();
        assert_eq!(fixed.max_threads, 1);
        assert!(!fixed.adaptive);
        assert_eq!(fixed.retained_throughput, 1.0);
        assert_eq!(fixed.system_cpu_limit, None);
    }
    #[test]
    fn worker_presets_follow_upstream_argument_order() {
        let first = parsed(&["--power-efficiency", "miser", "--scan-threads", "6"])
            .launch
            .threads
            .unwrap()
            .to_scan_threads(16)
            .unwrap();
        assert_eq!(first.max_threads, 6);
        let last = parsed(&["--scan-threads", "6", "--power-efficiency", "miser"])
            .launch
            .threads
            .unwrap()
            .to_scan_threads(16)
            .unwrap();
        assert_eq!(last.max_threads, 2);
    }
    #[test]
    fn ipc_schema_rejects_authority_fields_and_invalid_unresolved_values() {
        for json in [
            r#"{"command":"delete"}"#,
            r#"{"data_dir":"E:\\settings"}"#,
            r#"{"threads":{"max_threads":1,"adaptive":false,"retained_throughput_percent":80,"system_cpu_percent":0,"follow_links":true}}"#,
        ] {
            assert!(serde_json::from_str::<LaunchOverrides>(json).is_err());
        }
        for launch in [
            LaunchOverrides {
                root: Some("relative".into()),
                ..Default::default()
            },
            LaunchOverrides {
                root: Some("e:/owned/../owned".into()),
                ..Default::default()
            },
            LaunchOverrides {
                depth: Some(0),
                ..Default::default()
            },
            LaunchOverrides {
                depth: Some(7),
                ..Default::default()
            },
            LaunchOverrides {
                metric: Some(2),
                ..Default::default()
            },
            LaunchOverrides {
                threads: Some(ThreadPolicy {
                    max_threads: 0,
                    adaptive: false,
                    retained_throughput_percent: 80,
                    system_cpu_percent: 0,
                }),
                ..Default::default()
            },
        ] {
            assert!(launch.validate().is_err(), "accepted {launch:?}");
        }
        let cli = parsed(&[
            "--scan-root",
            r"E:\owned",
            "--depth",
            "6",
            "--adaptive-threads",
        ]);
        let roundtrip: LaunchOverrides =
            serde_json::from_str(&serde_json::to_string(&cli.launch).unwrap()).unwrap();
        roundtrip.validate().unwrap();
        assert_eq!(roundtrip, cli.launch);
    }
}
