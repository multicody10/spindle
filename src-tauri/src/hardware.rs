use crate::domain::{GpuDevice, HardwareProfile};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
};

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
        if ok != 0 {
            return (status.total_phys, status.avail_phys);
        }
    }
    (0, 0)
}

fn normalize_cpu_name(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('\0');
    if value.is_empty() {
        return None;
    }
    let normalized = value.to_ascii_lowercase();
    if normalized.contains("family")
        && normalized.contains("model")
        && normalized.contains("stepping")
    {
        return None;
    }
    Some(value.to_owned())
}

#[cfg(target_os = "windows")]
fn registry_cpu_name() -> Option<String> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let processor = hklm
        .open_subkey(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
        .ok()?;
    processor
        .get_value::<String, _>("ProcessorNameString")
        .ok()
        .and_then(|value| normalize_cpu_name(&value))
}

fn detect_cpu_name() -> Option<String> {
    #[cfg(target_os = "windows")]
    if let Some(name) = registry_cpu_name() {
        return Some(name);
    }

    std::env::var("PROCESSOR_IDENTIFIER")
        .ok()
        .and_then(|value| normalize_cpu_name(&value))
}

fn parse_nvidia_smi_output(output: &str) -> Vec<GpuDevice> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, ',').map(str::trim);
            let name = fields.next()?.trim();
            let total = fields.next()?.parse::<u64>().ok()?;
            let free = fields.next()?.parse::<u64>().ok()?;
            if name.is_empty() || total == 0 {
                return None;
            }
            Some(GpuDevice {
                vendor: "NVIDIA".into(),
                name: name.to_owned(),
                total_vram_bytes: total.saturating_mul(MIB),
                available_vram_bytes: free.saturating_mul(MIB),
                detection_method: "NVIDIA Management Library CLI (nvidia-smi)".into(),
            })
        })
        .collect()
}

fn nvidia_smi_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |path: PathBuf| {
        if seen.insert(path.clone()) {
            candidates.push(path);
        }
    };

    for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(program_files) = std::env::var_os(variable) {
            add(PathBuf::from(program_files)
                .join("NVIDIA Corporation")
                .join("NVSMI")
                .join("nvidia-smi.exe"));
        }
    }

    if let Some(system_root) = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("WINDIR"))
    {
        add(PathBuf::from(system_root)
            .join("System32")
            .join("nvidia-smi.exe"));
    }

    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            add(directory.join("nvidia-smi.exe"));
        }
    }

    candidates
}

fn query_nvidia_smi(executable: &Path) -> Vec<GpuDevice> {
    let mut command = Command::new(executable);
    crate::child_process::hide_console_window(&mut command);
    let output = command.args([
            "--query-gpu=name,memory.total,memory.free",
            "--format=csv,noheader,nounits",
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            parse_nvidia_smi_output(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

fn nvidia_smi_devices() -> Vec<GpuDevice> {
    for executable in nvidia_smi_candidates() {
        if !executable.is_file() {
            continue;
        }
        let devices = query_nvidia_smi(&executable);
        if !devices.is_empty() {
            return devices;
        }
    }

    // Let Windows resolve nvidia-smi.exe through its normal PATH lookup as a
    // final compatibility path for custom driver installations.
    let devices = query_nvidia_smi(Path::new("nvidia-smi.exe"));
    if !devices.is_empty() {
        return devices;
    }

    Vec::new()
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(file_name: *const u16) -> *mut std::ffi::c_void;
    fn GetProcAddress(module: *mut std::ffi::c_void, name: *const u8) -> *mut std::ffi::c_void;
    fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
}

#[cfg(target_os = "windows")]
type NvmlInit = unsafe extern "C" fn() -> i32;
#[cfg(target_os = "windows")]
type NvmlShutdown = unsafe extern "C" fn() -> i32;
#[cfg(target_os = "windows")]
type NvmlDeviceGetCount = unsafe extern "C" fn(*mut u32) -> i32;
#[cfg(target_os = "windows")]
type NvmlDeviceGetHandleByIndex = unsafe extern "C" fn(u32, *mut *mut std::ffi::c_void) -> i32;
#[cfg(target_os = "windows")]
type NvmlDeviceGetName = unsafe extern "C" fn(*mut std::ffi::c_void, *mut i8, u32) -> i32;
#[cfg(target_os = "windows")]
type NvmlDeviceGetMemoryInfo = unsafe extern "C" fn(*mut std::ffi::c_void, *mut NvmlMemory) -> i32;

#[cfg(target_os = "windows")]
unsafe fn nvml_symbol<T: Copy>(module: *mut std::ffi::c_void, name: &'static [u8]) -> Option<T> {
    // SAFETY: The supplied symbol names are NUL-terminated static strings. The
    // returned address is checked before copying it into the matching function
    // pointer type declared for the NVML API.
    let address = unsafe { GetProcAddress(module, name.as_ptr()) };
    if address.is_null() {
        None
    } else {
        // SAFETY: Each caller requests a function pointer whose ABI and signature
        // match the named NVML export.
        Some(unsafe { std::mem::transmute_copy(&address) })
    }
}

#[cfg(target_os = "windows")]
fn nvml_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(program_files) = std::env::var_os(variable) {
            candidates.push(
                PathBuf::from(program_files)
                    .join("NVIDIA Corporation")
                    .join("NVSMI")
                    .join("nvml.dll"),
            );
        }
    }
    if let Some(system_root) = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("WINDIR"))
    {
        candidates.push(PathBuf::from(system_root).join("System32").join("nvml.dll"));
    }
    candidates
}

#[cfg(target_os = "windows")]
fn nvml_devices() -> Vec<GpuDevice> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    for path in nvml_candidates() {
        if !path.is_file() {
            continue;
        }
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: wide is a valid NUL-terminated UTF-16 path. The returned
        // module handle is released with FreeLibrary below.
        let module = unsafe { LoadLibraryW(wide.as_ptr()) };
        if module.is_null() {
            continue;
        }

        // SAFETY: The symbol names and signatures match the documented NVML API.
        let init = unsafe { nvml_symbol::<NvmlInit>(module, b"nvmlInit_v2\0") };
        let shutdown = unsafe { nvml_symbol::<NvmlShutdown>(module, b"nvmlShutdown\0") };
        let get_count = unsafe {
            nvml_symbol::<NvmlDeviceGetCount>(module, b"nvmlDeviceGetCount_v2\0")
                .or_else(|| nvml_symbol::<NvmlDeviceGetCount>(module, b"nvmlDeviceGetCount\0"))
        };
        let get_handle = unsafe {
            nvml_symbol::<NvmlDeviceGetHandleByIndex>(module, b"nvmlDeviceGetHandleByIndex_v2\0")
                .or_else(|| {
                    nvml_symbol::<NvmlDeviceGetHandleByIndex>(
                        module,
                        b"nvmlDeviceGetHandleByIndex\0",
                    )
                })
        };
        let get_name = unsafe { nvml_symbol::<NvmlDeviceGetName>(module, b"nvmlDeviceGetName\0") };
        let get_memory =
            unsafe { nvml_symbol::<NvmlDeviceGetMemoryInfo>(module, b"nvmlDeviceGetMemoryInfo\0") };

        let (
            Some(init),
            Some(shutdown),
            Some(get_count),
            Some(get_handle),
            Some(get_name),
            Some(get_memory),
        ) = (init, shutdown, get_count, get_handle, get_name, get_memory)
        else {
            // SAFETY: module is a live handle returned by LoadLibraryW.
            unsafe { FreeLibrary(module) };
            continue;
        };

        // SAFETY: NVML functions are called with pointers to initialized storage
        // matching their documented signatures.
        let initialized = unsafe { init() } == 0;
        if !initialized {
            unsafe { FreeLibrary(module) };
            continue;
        }

        let mut count = 0u32;
        let count_ok = unsafe { get_count(&mut count) } == 0;
        let mut devices = Vec::new();
        if count_ok {
            for index in 0..count {
                let mut device = ptr::null_mut();
                if unsafe { get_handle(index, &mut device) } != 0 || device.is_null() {
                    continue;
                }
                let mut name = [0i8; 128];
                if unsafe { get_name(device, name.as_mut_ptr(), name.len() as u32) } != 0 {
                    continue;
                }
                let end = name
                    .iter()
                    .position(|byte| *byte == 0)
                    .unwrap_or(name.len());
                let name = String::from_utf8_lossy(
                    &name[..end]
                        .iter()
                        .map(|byte| *byte as u8)
                        .collect::<Vec<_>>(),
                )
                .trim()
                .to_owned();
                if name.is_empty() {
                    continue;
                }
                let mut memory = NvmlMemory {
                    total: 0,
                    free: 0,
                    used: 0,
                };
                if unsafe { get_memory(device, &mut memory) } != 0 || memory.total == 0 {
                    continue;
                }
                devices.push(GpuDevice {
                    vendor: "NVIDIA".into(),
                    name,
                    total_vram_bytes: memory.total,
                    available_vram_bytes: memory.free,
                    detection_method: "NVIDIA Management Library (NVML)".into(),
                });
            }
        }

        unsafe {
            shutdown();
            FreeLibrary(module);
        }
        if !devices.is_empty() {
            return devices;
        }
    }

    Vec::new()
}

#[cfg(not(target_os = "windows"))]
fn nvml_devices() -> Vec<GpuDevice> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn is_nvidia_adapter(description: &str, matching_device_id: &str) -> bool {
    let description = description.to_ascii_lowercase();
    description.contains("nvidia")
        || description.contains("geforce")
        || description.contains("quadro")
        || description.contains("tesla")
        || matching_device_id.to_ascii_lowercase().contains("ven_10de")
}

#[cfg(target_os = "windows")]
fn registry_nvidia_devices() -> Vec<GpuDevice> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(adapters) = hklm.open_subkey(
        r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}",
    ) else {
        return Vec::new();
    };

    adapters
        .enum_keys()
        .flatten()
        .filter(|name| name.chars().all(|character| character.is_ascii_hexdigit()))
        .filter_map(|subkey| {
            let key = adapters.open_subkey(subkey).ok()?;
            let description = key
                .get_value::<String, _>("DriverDesc")
                .or_else(|_| key.get_value::<String, _>("HardwareInformation.AdapterString"))
                .unwrap_or_default();
            let matching_id = key
                .get_value::<String, _>("MatchingDeviceId")
                .unwrap_or_default();
            if !is_nvidia_adapter(&description, &matching_id) {
                return None;
            }

            let total_vram_bytes = key
                .get_value::<u64, _>("HardwareInformation.qwMemorySize")
                .unwrap_or(0);
            Some(GpuDevice {
                vendor: "NVIDIA".into(),
                name: if description.trim().is_empty() {
                    "NVIDIA graphics adapter".into()
                } else {
                    description.trim().to_owned()
                },
                total_vram_bytes,
                available_vram_bytes: 0,
                detection_method: if total_vram_bytes > 0 {
                    "Windows display-adapter registry (live VRAM telemetry unavailable)".into()
                } else {
                    "Windows display-adapter registry (device detected; VRAM unavailable)".into()
                },
            })
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn registry_nvidia_devices() -> Vec<GpuDevice> {
    Vec::new()
}

fn nvidia_devices() -> Vec<GpuDevice> {
    let devices = nvidia_smi_devices();
    if !devices.is_empty() {
        return devices;
    }
    let devices = nvml_devices();
    if !devices.is_empty() {
        return devices;
    }
    registry_nvidia_devices()
}

pub fn detect() -> HardwareProfile {
    let (ram_total_bytes, ram_available_bytes) = memory_status();
    let logical_threads = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let cpu_name = detect_cpu_name();
    let gpus = nvidia_devices();
    let mut detection_notes = vec![
        "System RAM values are obtained from Windows GlobalMemoryStatusEx; available memory is a point-in-time measurement.".into(),
        "CPU model names are read from the Windows processor-name registry value instead of the low-level PROCESSOR_IDENTIFIER string.".into(),
        "NVIDIA discovery checks nvidia-smi in standard locations and PATH, then NVML, then the Windows display-adapter registry. AMD and Intel adapters are not yet enumerated.".into(),
        "Physical CPU core count is not inferred from logical thread count.".into(),
    ];
    if ram_total_bytes == 0 {
        detection_notes.push("System memory detection is unavailable on this platform; automatic memory budgeting will be conservative.".into());
    }
    if cpu_name.is_none() {
        detection_notes.push("A human-readable CPU model name was not exposed by Windows.".into());
    }
    if gpus.is_empty() {
        detection_notes.push(
            "No NVIDIA GPU was detected through nvidia-smi, NVML, or the Windows display-adapter registry. This does not prove that the machine has no GPU.".into(),
        );
    } else if gpus.iter().any(|gpu| gpu.available_vram_bytes == 0) {
        detection_notes.push(
            "An NVIDIA adapter was found, but live free-VRAM telemetry was unavailable. Automatic GPU-layer budgeting remains conservative until NVML or nvidia-smi can report free memory.".into(),
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_processor_identifier_instead_of_showing_cpuid_gibberish() {
        assert_eq!(
            normalize_cpu_name("Intel64 Family 6 Model 158 Stepping 9, GenuineIntel"),
            None
        );
        assert_eq!(
            normalize_cpu_name("  Intel(R) Core(TM) i7-7700 CPU @ 3.60GHz  ").as_deref(),
            Some("Intel(R) Core(TM) i7-7700 CPU @ 3.60GHz")
        );
    }

    #[test]
    fn parses_nvidia_smi_memory_rows() {
        let devices = parse_nvidia_smi_output(
            "GeForce GTX 1070, 8192, 4096\nGeForce RTX 3060, 12288, 10240\n",
        );
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "GeForce GTX 1070");
        assert_eq!(devices[0].total_vram_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(devices[0].available_vram_bytes, 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn ignores_malformed_nvidia_smi_rows() {
        assert!(parse_nvidia_smi_output("NVIDIA GPU, N/A, N/A\n").is_empty());
        assert!(parse_nvidia_smi_output(",,\n").is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn detects_nvidia_registry_vendor_ids() {
        assert!(is_nvidia_adapter("GeForce GTX 1070", ""));
        assert!(is_nvidia_adapter("", "PCI\\VEN_10DE&DEV_1B81"));
        assert!(!is_nvidia_adapter(
            "Intel UHD Graphics",
            "PCI\\VEN_8086&DEV_5912"
        ));
    }
}
