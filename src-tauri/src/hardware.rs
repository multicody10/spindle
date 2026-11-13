use crate::domain::{GpuDevice, HardwareProfile};
use std::process::Command;

const MIB: u64 = 1024 * 1024;

#[cfg(target_os = "windows")]
#[repr(C)]
struct MemoryStatusEx {
    length: u32,
    memory_load: u32,
    total_phys: u64,
    avail_phys: u64,
    total_page_file: u64,
    avail_page_file: u64,
    total_virtual: u64,
    avail_virtual: u64,
    avail_extended_virtual: u64,
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
}

fn memory_status() -> (u64, u64) {
    #[cfg(target_os = "windows")]
    {
        let mut status = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: GlobalMemoryStatusEx writes into a correctly sized, initialized
        // MEMORYSTATUSEX structure owned by this function.
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        if ok != 0 { return (status.total_phys, status.avail_phys); }
    }
    (0, 0)
}

fn nvidia_devices() -> Vec<GpuDevice> {
    let mut candidates = Vec::new();
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        candidates.push(std::path::PathBuf::from(program_files)
            .join("NVIDIA Corporation").join("NVSMI").join("nvidia-smi.exe"));
    }
    if let Some(program_files) = std::env::var_os("ProgramW6432") {
        candidates.push(std::path::PathBuf::from(program_files)
            .join("NVIDIA Corporation").join("NVSMI").join("nvidia-smi.exe"));
    }

    for executable in candidates {
        if !executable.is_file() { continue; }
        let output = match Command::new(&executable)
            .args(["--query-gpu=name,memory.total,memory.free", "--format=csv,noheader,nounits"])
            .output()
        {
            Ok(output) if output.status.success() => output,
            _ => continue,
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut devices = Vec::new();
        for line in text.lines() {
            let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
            if fields.len() != 3 { continue; }
            let (Ok(total), Ok(free)) = (fields[1].parse::<u64>(), fields[2].parse::<u64>()) else { continue; };
            devices.push(GpuDevice {
                vendor: "NVIDIA".into(),
                name: fields[0].to_string(),
                total_vram_bytes: total.saturating_mul(MIB),
                available_vram_bytes: free.saturating_mul(MIB),
                detection_method: "NVIDIA Management Library CLI (nvidia-smi)".into(),
            });
        }
        if !devices.is_empty() { return devices; }
    }
    Vec::new()
}

pub fn detect() -> HardwareProfile {
    let (ram_total_bytes, ram_available_bytes) = memory_status();
    let logical_threads = std::thread::available_parallelism().map(|v| v.get()).unwrap_or(1);
    let cpu_name = std::env::var("PROCESSOR_IDENTIFIER")
        .ok().filter(|value| !value.trim().is_empty());
    let gpus = nvidia_devices();
    let mut detection_notes = vec![
        "System RAM values are obtained from Windows GlobalMemoryStatusEx; available memory is a point-in-time measurement.".into(),
        "GPU discovery currently supports NVIDIA through the vendor nvidia-smi utility. AMD and Intel adapters are not yet enumerated.".into(),
        "Physical CPU core count is not inferred from logical thread count.".into(),
    ];
    if ram_total_bytes == 0 {
        detection_notes.push("System memory detection is unavailable on this platform; automatic memory budgeting will be conservative.".into());
    }
    if gpus.is_empty() {
        detection_notes.push("No NVIDIA GPU was detected. This does not prove that the machine has no GPU.".into());
    }

    HardwareProfile {
        os: std::env::consts::OS.into(),
        cpu_name,
        logical_threads,
        physical_cores: None,
        ram_total_bytes,
        ram_available_bytes,
        gpus,
        detection_notes,
    }
}
