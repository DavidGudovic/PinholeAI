//! `nvidia-smi --query-gpu=index,name,memory.total --format=csv,noheader,nounits`
//! (detection) and, right before an engine starts, how much graphics memory is
//! in use and by which compute processes ([`query_vram_usage`]).

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{round1, GpuInfo, Vendor};

const QUERY_ARGS: &[&str] = &["--query-gpu=index,name,memory.total", "--format=csv,noheader,nounits"];
const MEMORY_ARGS: &[&str] = &["--query-gpu=index,memory.total,memory.used,memory.free", "--format=csv,noheader,nounits"];
const APPS_ARGS: &[&str] = &["--query-compute-apps=pid,process_name,used_memory", "--format=csv,noheader,nounits"];
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
    run_smi(QUERY_ARGS).map(|out| parse_nvidia_smi(&out)).unwrap_or_default()
}

/// `nvidia-smi <args>` with the first candidate that exists; `None` on any failure.
fn run_smi(args: &[&str]) -> Option<String> {
    for exe in candidates() {
        let mut cmd = Command::new(&exe);
        cmd.args(args);
        match run_with_timeout(cmd, TIMEOUT) {
            RunResult::NotFound => continue,
            RunResult::Failed => return None,
            RunResult::Ok(out) => return Some(out),
        }
    }
    None
}

/// Memory of one NVIDIA GPU, in MiB (`nvidia-smi` order = CUDA order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuMemory {
    pub index: usize,
    pub total_mib: u64,
    pub used_mib: u64,
    pub free_mib: u64,
}

/// A process with a CUDA context. `used_mib` is `None` where the driver
/// doesn't report it (Windows WDDM shows `[N/A]`). Graphics-only programs
/// (most games on Windows) are not listed by `--query-compute-apps` at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProcess {
    pub pid: u32,
    /// Executable file name (`python.exe`), never a full path.
    pub name: String,
    pub used_mib: Option<u64>,
}

/// Graphics memory in use right now (NVIDIA only).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VramUsage {
    pub gpus: Vec<GpuMemory>,
    pub processes: Vec<GpuProcess>,
}

/// Graphics memory on one GPU that is used by programs other than ours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherGpuUse {
    pub gpu_index: usize,
    pub total_mib: u64,
    /// Used by others (in-use memory minus what our own processes report).
    pub others_mib: u64,
    /// Other compute processes, biggest first (empty when the GPU can't be
    /// told apart from other GPUs, or none are listed).
    pub processes: Vec<GpuProcess>,
}

impl OtherGpuUse {
    /// Worth telling the user: more than 2 GiB or more than a quarter of the card.
    pub fn is_significant(&self) -> bool {
        self.others_mib > 2048.min(self.total_mib / 4)
    }
}

/// Parse `index, memory.total, memory.used, memory.free` lines (MiB). Lines
/// without a numeric index or with `[N/A]` totals are skipped.
pub fn parse_gpu_memory(output: &str) -> Vec<GpuMemory> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(',').map(str::trim).collect();
            if parts.len() != 4 {
                return None;
            }
            let num = |s: &str| s.parse::<u64>().ok();
            let index = parts[0].parse::<usize>().ok()?;
            let total_mib = num(parts[1]).filter(|t| *t > 0)?;
            let used_mib = num(parts[2])?;
            let free_mib = num(parts[3]).unwrap_or_else(|| total_mib.saturating_sub(used_mib));
            Some(GpuMemory { index, total_mib, used_mib, free_mib })
        })
        .collect()
}

/// Parse `pid, process_name, used_memory` lines. Names may contain commas;
/// only the file name of a full path is kept; memory like `[N/A]` → `None`.
pub fn parse_compute_apps(output: &str) -> Vec<GpuProcess> {
    output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(',').map(str::trim).collect();
            if parts.len() < 3 {
                return None;
            }
            let pid = parts[0].parse::<u32>().ok()?;
            let used_mib = parts[parts.len() - 1].parse::<u64>().ok();
            let full = parts[1..parts.len() - 1].join(", ");
            let name = full.rsplit(['/', '\\']).next().unwrap_or("").trim().to_string();
            let name = if name.is_empty() { format!("process {pid}") } else { name };
            Some(GpuProcess { pid, name, used_mib })
        })
        .collect()
}

impl VramUsage {
    /// What programs other than `ours` (our engines, this app) use on GPU
    /// `gpu` (`nvidia-smi` index; `None` = the first). `None` if that GPU
    /// isn't listed. Processes are only named when there is a single GPU:
    /// `--query-compute-apps` doesn't say which GPU a process uses.
    pub fn others(&self, gpu: Option<usize>, ours: &[u32]) -> Option<OtherGpuUse> {
        let g = match gpu {
            Some(i) => self.gpus.iter().find(|g| g.index == i)?,
            None => self.gpus.iter().min_by_key(|g| g.index)?,
        };
        let single = self.gpus.len() == 1;
        let our_mib: u64 = if single { self.processes.iter().filter(|p| ours.contains(&p.pid)).filter_map(|p| p.used_mib).sum() } else { 0 };
        let mut processes: Vec<GpuProcess> = if single { self.processes.iter().filter(|p| !ours.contains(&p.pid)).cloned().collect() } else { Vec::new() };
        processes.sort_by(|a, b| b.used_mib.cmp(&a.used_mib).then_with(|| a.name.cmp(&b.name)));
        Some(OtherGpuUse { gpu_index: g.index, total_mib: g.total_mib, others_mib: g.used_mib.saturating_sub(our_mib), processes })
    }
}

/// Current graphics memory use (runs `nvidia-smi` twice; blocking, ≤ ~10 s).
/// `None` when nvidia-smi is missing or fails. The process list is optional:
/// if that query fails, the memory numbers are still returned.
pub fn query_vram_usage() -> Option<VramUsage> {
    let gpus = parse_gpu_memory(&run_smi(MEMORY_ARGS)?);
    if gpus.is_empty() {
        return None;
    }
    let processes = run_smi(APPS_ARGS).map(|out| parse_compute_apps(&out)).unwrap_or_default();
    Some(VramUsage { gpus, processes })
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
    fn parses_memory_use() {
        let gpus = parse_gpu_memory("0, 16303, 9234, 6726\r\n1, 12288, 0, 12045\nNVIDIA-SMI has failed\n2, [N/A], [N/A], [N/A]\n");
        assert_eq!(gpus, vec![
            GpuMemory { index: 0, total_mib: 16303, used_mib: 9234, free_mib: 6726 },
            GpuMemory { index: 1, total_mib: 12288, used_mib: 0, free_mib: 12045 },
        ]);
        assert_eq!(parse_gpu_memory("0, 8192, 100, [N/A]")[0].free_mib, 8092);
        assert!(parse_gpu_memory("").is_empty());
        assert!(parse_gpu_memory("0, 8192, 100").is_empty(), "wrong field count");
    }

    #[test]
    fn parses_compute_apps_on_linux_and_windows() {
        let linux = "4242, /home/ana/ComfyUI/venv/bin/python3, 9102\n777, /home/ana/Pinhole/Data/engine/sd/v/vulkan/sd-server, 7000\n";
        let apps = parse_compute_apps(linux);
        assert_eq!(apps[0], GpuProcess { pid: 4242, name: "python3".into(), used_mib: Some(9102) });
        assert_eq!(apps[1].name, "sd-server");
        // WDDM: no per-process numbers; names are full Windows paths (may contain commas).
        let windows = "13520, C:\\Users\\Ana\\StabilityMatrix\\Packages\\ComfyUI\\venv\\Scripts\\python.exe, [N/A]\r\n9, C:\\Games\\Big, Game\\game.exe, [N/A]\r\n";
        let apps = parse_compute_apps(windows);
        assert_eq!(apps[0], GpuProcess { pid: 13520, name: "python.exe".into(), used_mib: None });
        assert_eq!(apps[1].name, "game.exe");
        assert!(parse_compute_apps("No running processes found").is_empty());
        assert_eq!(parse_compute_apps("5, , 10")[0].name, "process 5");
    }

    #[test]
    fn others_leave_out_our_processes() {
        let usage = VramUsage {
            gpus: vec![GpuMemory { index: 0, total_mib: 16303, used_mib: 16000, free_mib: 303 }],
            processes: vec![
                GpuProcess { pid: 1, name: "sd-server.exe".into(), used_mib: Some(7000) },
                GpuProcess { pid: 2, name: "python.exe".into(), used_mib: Some(8900) },
                GpuProcess { pid: 3, name: "obs64.exe".into(), used_mib: None },
            ],
        };
        let o = usage.others(Some(0), &[1]).unwrap();
        assert_eq!(o.others_mib, 9000);
        assert_eq!(o.processes.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["python.exe", "obs64.exe"]);
        assert!(o.is_significant());
        assert!(usage.others(Some(3), &[]).is_none());
        // WDDM: nothing to subtract, everything in use counts.
        let wddm = VramUsage { gpus: usage.gpus.clone(), processes: vec![GpuProcess { pid: 2, name: "python.exe".into(), used_mib: None }] };
        assert_eq!(wddm.others(None, &[1]).unwrap().others_mib, 16000);
        // Desktop only (~1 GB on a 16 GB card) is not worth a note; 1.5 GB of a 4 GB card is.
        let quiet = OtherGpuUse { gpu_index: 0, total_mib: 16303, others_mib: 1100, processes: vec![] };
        assert!(!quiet.is_significant());
        assert!(OtherGpuUse { total_mib: 4096, others_mib: 1500, ..quiet.clone() }.is_significant());
        // Two GPUs: processes can't be attributed, so none are named.
        let two = VramUsage { gpus: vec![usage.gpus[0], GpuMemory { index: 1, ..usage.gpus[0] }], processes: usage.processes.clone() };
        let o = two.others(Some(1), &[1]).unwrap();
        assert!(o.processes.is_empty());
        assert_eq!(o.others_mib, 16000);
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
