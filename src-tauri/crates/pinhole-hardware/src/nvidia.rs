//! `nvidia-smi --query-gpu=index,name,memory.total --format=csv,noheader,nounits`

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::{round1, GpuInfo, Vendor};

const QUERY_ARGS: &[&str] = &["--query-gpu=index,name,memory.total", "--format=csv,noheader,nounits"];
const TIMEOUT: Duration = Duration::from_secs(5);
/// Output cap (a line per GPU; anything near this is not nvidia-smi).
const MAX_OUTPUT: u64 = 64 * 1024;

/// Parse `index, name, memory.total [MiB]` lines. Names may contain commas;
/// memory like `[N/A]` becomes 0. Lines without a numeric index are ignored
/// (e.g. the "NVIDIA-SMI has failed…" message).
pub fn parse_nvidia_smi(output: &str) -> Vec<GpuInfo> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(',').map(str::trim).collect();
            if parts.len() < 3 {
                return None;
            }
            let index = parts[0].parse::<usize>().ok()?;
            let mem = parts[parts.len() - 1];
            let name = parts[1..parts.len() - 1].join(", ");
            let name = if name.is_empty() { "NVIDIA GPU".to_string() } else { name };
            let vram_gb = mem.parse::<f64>().ok().filter(|m| m.is_finite() && *m > 0.0).map(|mib| round1(mib / 1024.0)).unwrap_or(0.0);
            Some(GpuInfo { index, vendor: Vendor::Nvidia, name, vram_gb })
        })
        .collect()
}

/// Run nvidia-smi (first candidate that exists). Empty on any failure.
pub(crate) fn query() -> Vec<GpuInfo> {
    for exe in candidates() {
        let mut cmd = Command::new(&exe);
        cmd.args(QUERY_ARGS);
        match run_with_timeout(cmd, TIMEOUT) {
            RunResult::NotFound => continue,
            RunResult::Failed => return Vec::new(),
            RunResult::Ok(out) => return parse_nvidia_smi(&out),
        }
    }
    Vec::new()
}

#[cfg(windows)]
fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    out.push(system_root.join("System32").join("nvidia-smi.exe"));
    out.push(PathBuf::from(r"C:\Windows\System32\nvidia-smi.exe"));
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        out.push(PathBuf::from(pf).join("NVIDIA Corporation").join("NVSMI").join("nvidia-smi.exe"));
    }
    out.push(PathBuf::from("nvidia-smi.exe")); // PATH
    out.dedup();
    out
}

#[cfg(not(windows))]
fn candidates() -> Vec<PathBuf> {
    vec![PathBuf::from("nvidia-smi"), PathBuf::from("/usr/bin/nvidia-smi")]
}

pub(crate) enum RunResult {
    /// The program doesn't exist here: try the next candidate.
    NotFound,
    /// It ran but failed / timed out / couldn't start.
    Failed,
    Ok(String),
}

/// Spawn without a console window (Windows), wait at most `timeout`, kill on
/// timeout. stdout is read on a helper thread so a chatty child can't block.
pub(crate) fn run_with_timeout(mut cmd: Command, timeout: Duration) -> RunResult {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return RunResult::NotFound,
        Err(_) => return RunResult::Failed,
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return RunResult::Failed;
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(MAX_OUTPUT).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().unwrap_or_default();
                return if status.success() { RunResult::Ok(String::from_utf8_lossy(&out).into_owned()) } else { RunResult::Failed };
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return RunResult::Failed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_and_multi_gpu() {
        let out = "0, NVIDIA GeForce RTX 5070 Ti, 16303\n1, NVIDIA GeForce RTX 3060, 12288\n";
        let gpus = parse_nvidia_smi(out);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0], GpuInfo { index: 0, vendor: Vendor::Nvidia, name: "NVIDIA GeForce RTX 5070 Ti".into(), vram_gb: 15.9 });
        assert_eq!(gpus[1].vram_gb, 12.0);
        assert_eq!(gpus[1].index, 1);
    }

    #[test]
    fn rounding_matches_marketing_sizes() {
        let gb = |mib: u32| parse_nvidia_smi(&format!("0, X, {mib}"))[0].vram_gb;
        assert_eq!(gb(8188), 8.0); // RTX 4060
        assert_eq!(gb(24564), 24.0); // RTX 4090
        assert_eq!(gb(32607), 31.8); // RTX 5090
        assert_eq!(gb(6144), 6.0);
    }

    #[test]
    fn tolerates_odd_output() {
        let out = "\r\n  2 , Tesla, weird, name ,  81920 \r\n3, Quadro, [N/A]\nNVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver.\n\n";
        let gpus = parse_nvidia_smi(out);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "Tesla, weird, name");
        assert_eq!(gpus[0].vram_gb, 80.0);
        assert_eq!(gpus[0].index, 2);
        assert_eq!(gpus[1].vram_gb, 0.0);
        assert!(parse_nvidia_smi("").is_empty());
        assert!(parse_nvidia_smi("No devices were found").is_empty());
        assert_eq!(parse_nvidia_smi("0, , 4096")[0].name, "NVIDIA GPU");
    }

    #[test]
    fn missing_program_is_not_found() {
        let cmd = Command::new("pinhole-definitely-not-a-real-program-xyz");
        assert!(matches!(run_with_timeout(cmd, Duration::from_secs(1)), RunResult::NotFound));
    }

    #[cfg(unix)]
    #[test]
    fn runner_captures_output_and_times_out() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo '0, Fake GPU, 8192'"]);
        match run_with_timeout(cmd, Duration::from_secs(5)) {
            RunResult::Ok(out) => assert_eq!(parse_nvidia_smi(&out)[0].vram_gb, 8.0),
            _ => panic!("expected output"),
        }
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 3"]);
        assert!(matches!(run_with_timeout(cmd, Duration::from_secs(5)), RunResult::Failed));
        let mut cmd = Command::new("sleep");
        cmd.arg("10");
        let start = Instant::now();
        assert!(matches!(run_with_timeout(cmd, Duration::from_millis(200)), RunResult::Failed));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
