//! Kill all processes occupying a file/folder, and enforce deletion.
//! [Inspired by](https://t.me/withabsolutex/1537)

#![warn(clippy::nursery, clippy::cargo)]
mod cli;

use std::{
    env, fs, io,
    mem::{offset_of, size_of},
    path::Path,
    process::Command,
    ptr,
    thread::sleep,
    time::Duration,
};

use clap::Parser;
use cli::Cli;
use inquire::Confirm;
use windows::{
    core::PCWSTR,
    Wdk::Storage::FileSystem::{FileProcessIdsUsingFileInformation, NtQueryInformationFile},
    Win32::{
        Foundation::{CloseHandle, HANDLE, STATUS_SUCCESS},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_CREATION_DISPOSITION, FILE_SHARE_MODE,
        },
        System::{
            ProcessStatus::{EnumProcessModules, GetModuleBaseNameW},
            Threading::{
                OpenProcess, TerminateProcess, PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE,
                PROCESS_VM_READ,
            },
            IO::IO_STATUS_BLOCK,
        },
    },
};
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_WRITE},
    RegKey,
};

const FILE_READ_ATTRIBUTES: u32 = 0x80;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;

/// Variable-length struct: `process_id_list` actually holds
/// `number_of_process_ids_in_list` entries.
#[repr(C)]
struct FileProcessIdsUsingFileInformation {
    number_of_process_ids_in_list: u32,
    process_id_list: [usize; 1],
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
}

impl std::fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

fn remove_any(path: &Path) -> io::Result<()> {
    if path.is_file() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
}

/// 通过进程ID获取进程名称
#[allow(clippy::missing_safety_doc)]
unsafe fn get_process_name_by_pid(pid: u32) -> Option<String> {
    // 打开目标进程
    let process_handle = match OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
    {
        Ok(handle) => handle,
        Err(_) => return None,
    };

    if process_handle.is_invalid() {
        return None;
    }

    let mut module_handles: [windows::Win32::Foundation::HMODULE; 1024] = std::mem::zeroed();
    let mut cb_needed = 0;

    // 枚举进程模块
    if EnumProcessModules(
        process_handle,
        module_handles.as_mut_ptr(),
        size_of::<[windows::Win32::Foundation::HMODULE; 1024]>() as u32,
        &mut cb_needed,
    )
    .is_err()
    {
        CloseHandle(process_handle).ok();
        return None;
    }

    let mut module_name_buffer: [u16; 256] = [0; 256];
    // 获取第一个模块（即可执行文件）的名称
    let name_len = GetModuleBaseNameW(
        process_handle,
        Some(module_handles[0]),
        &mut module_name_buffer,
    );

    CloseHandle(process_handle).ok();

    if name_len == 0 {
        return None;
    }

    Some(String::from_utf16_lossy(
        &module_name_buffer[..name_len as usize],
    ))
}

#[allow(clippy::missing_safety_doc)]
pub unsafe fn get_process_info_from_file_path(path: &str) -> Result<Vec<ProcessInfo>, String> {
    let mut file: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let handle = CreateFileW(
        PCWSTR(file.as_mut_ptr()),
        FILE_READ_ATTRIBUTES,
        FILE_SHARE_MODE(FILE_SHARE),
        Some(std::ptr::null_mut()),
        FILE_CREATION_DISPOSITION(OPEN_EXISTING),
        FILE_ATTRIBUTE_NORMAL,
        None,
    )
    .map_err(|e| e.to_string())?;

    let result = query_processes_using_file(handle);
    // Close in every path so the handle is not leaked when the query fails.
    CloseHandle(handle).map_err(|e| e.to_string())?;
    result
}

/// Upper bound for the query buffer; the pid list never needs more than this.
const MAX_QUERY_BUFFER: u32 = 1 << 20;

#[allow(clippy::missing_safety_doc)]
unsafe fn query_processes_using_file(handle: HANDLE) -> Result<Vec<ProcessInfo>, String> {
    let mut iosb: IO_STATUS_BLOCK = std::mem::zeroed();
    let mut bytes = size_of::<FileProcessIdsUsingFileInformation>() as u32;
    loop {
        let mut buffer = vec![0u8; bytes as usize];
        let status = NtQueryInformationFile(
            handle,
            &mut iosb,
            buffer.as_mut_ptr().cast(),
            bytes,
            FileProcessIdsUsingFileInformation,
        );
        // NtQueryInformationFile leaves the transfer size in the iosb union
        // (Pointer field); Clear it, or CloseHandle fails with access denied.
        iosb.Anonymous.Pointer = ptr::null_mut();

        if status != STATUS_SUCCESS {
            if bytes >= MAX_QUERY_BUFFER {
                return Err(format!(
                    "NtQueryInformationFile failed with status: {:X}",
                    status.0 as u32
                ));
            }
            bytes *= 2;
            continue;
        }

        let pids = parse_process_ids(&buffer)
            .ok_or("Pid count reported by the query exceeds the buffer size")?;
        return Ok(pids
            .into_iter()
            .map(|pid| ProcessInfo {
                pid,
                name: get_process_name_by_pid(pid).unwrap_or_else(|| "<Unknown>".to_string()),
            })
            .collect());
    }
}

/// Extract the pid list from a `FileProcessIdsUsingFileInformation` buffer.
/// Returns `None` if the reported count does not fit in the buffer.
fn parse_process_ids(buffer: &[u8]) -> Option<Vec<u32>> {
    let offset = offset_of!(FileProcessIdsUsingFileInformation, process_id_list);
    // SAFETY: the struct is trivially readable from any sufficiently long,
    // aligned-free byte buffer (plain integers, no invalid representations).
    let info = unsafe { &*(buffer.as_ptr() as *const FileProcessIdsUsingFileInformation) };
    let count = info.number_of_process_ids_in_list as usize;
    if buffer.len() < offset + count * size_of::<usize>() {
        return None;
    }
    // SAFETY: `count` entries were just verified to fit in the buffer.
    let list = unsafe { std::slice::from_raw_parts(info.process_id_list.as_ptr(), count) };
    Some(list.iter().map(|&pid| pid as u32).collect())
}

/// delete the given file/folder.
fn visit<F>(path: &Path, remove_fun: &F) -> std::io::Result<()>
where
    F: Fn(&Path) -> Result<&Path, String>,
{
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            visit(&path, remove_fun)?;
        }
    }
    if let Err(reason) = remove_fun(path) {
        eprintln!("{reason}\nError occurs when deleting {}.", &path.display());
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn kill_process(pid: u32) -> std::io::Result<()> {
    // 1. 获取目标进程的句柄。
    let process_handle = match OpenProcess(PROCESS_TERMINATE, false, pid) {
        Ok(handle) => {
            // 检查句柄是否有效。虽然 `Ok`
            // 通常意味着有效，但多一层保险没有坏处。
            if handle.is_invalid() {
                return Err(io::Error::last_os_error());
            }
            handle
        }
        Err(e) => {
            return Err(io::Error::other(e));
        }
    };

    // 2. 使用获取到的句柄来终止进程。
    let result = TerminateProcess(process_handle, 1);

    CloseHandle(process_handle)?;
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(io::Error::other(e)),
    }
}

fn add_context_menu_entry() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let current_exe = env::current_exe()?;
    let exe_path = current_exe.to_str().ok_or("Invalid executable path")?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes_key = hkcu
        .open_subkey_with_flags("Software\\Classes", KEY_WRITE)
        .or_else(|_| hkcu.create_subkey("Software\\Classes").map(|x| x.0))?;

    // For files (only for current user)
    let (key, _) = classes_key.create_subkey("*\\shell\\FUCK, DELETE IT!\\command")?;
    key.set_value("", &format!("\"{}\" \"%1\"", exe_path))?;
    println!("Added context menu entry for files.");

    // For folders (only for current user)
    let (key, _) = classes_key.create_subkey("Directory\\shell\\FUCK, DELETE IT!\\command")?;
    key.set_value("", &format!("\"{}\" \"%1\"", exe_path))?;
    println!("Added context menu entry for directories.");

    Ok(())
}

fn remove_context_menu_entry() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let classes_key = hkcu.open_subkey("Software\\Classes")?;

    let key_paths = ["*\\shell", "Directory\\shell"];
    for key_path in key_paths {
        match classes_key.open_subkey_with_flags(key_path, KEY_ALL_ACCESS) {
            Ok(shell_key) => match shell_key.delete_subkey_all("FUCK, DELETE IT!") {
                Ok(_) => {
                    println!("Removed context menu entry for files.");
                }
                Err(e) => {
                    if e.raw_os_error() == Some(2) {
                        println!("Context menu entry for files not found (already removed or never existed).");
                    } else {
                        eprintln!("Error removing file context menu entry: {}", e);
                    }
                }
            },
            Err(e) => {
                if e.raw_os_error() == Some(2) {
                    println!(
                        "Parent key `{}` for files not found, entry likely doesn't exist.",
                        key_path
                    );
                } else {
                    eprintln!("Error opening {} key: {}", key_path, e);
                }
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    if cli.uninstall {
        remove_context_menu_entry()?;
        println!("Context menu entries removed.");
        let _ = Command::new("cmd").args(["/C", "pause"]).status();
        return Ok(());
    }
    if cli.path.is_empty() {
        // No arguments provided, add context menu entry
        add_context_menu_entry()?;
        println!("Context menu entries added. You can now right-click on files/folders to use the program.");
        let _ = Command::new("cmd").args(["/C", "pause"]).status();
    }
    for path in cli.path {
        assert!(path.exists(), "Path does not exist");
        let confirm = cli.yes
            || Confirm::new(&format!(
                "Are you sure you want to delete {path:?}? The {} cannot be recovered.",
                if path.is_dir() { "folder" } else { "file" }
            ))
            .with_default(true)
            .prompt()?;
        if !confirm {
            return Ok(());
        }
        visit(path.as_path(), &|path| unsafe {
            if let Err(e) = remove_any(path) {
                eprintln!("Failed to delete file {path:?}: {e} ");
                if e.kind() == io::ErrorKind::PermissionDenied {
                    eprintln!("Permission denied, skipping...");
                    return Ok(path);
                }
                let pinfos = get_process_info_from_file_path(
                    path.to_str().ok_or("Not a valid utf-8 filename.")?,
                )?;
                if cli.yes
                    || Confirm::new(&format!("Kill processes:\n{pinfos:#?}?"))
                        .with_default(true)
                        .prompt()
                        .map_err(|e| e.to_string())?
                {
                    for pinfo in pinfos {
                        kill_process(pinfo.pid).map_err(|e| {
                            format!(
                                "Failed to kill process `{}` [{}]: {e}",
                                pinfo.name, pinfo.pid
                            )
                        })?;
                    }
                    sleep(Duration::from_millis(20)); // wait for the process to
                                                      // be killed.
                    remove_any(path).map_err(|_| {
                        format!(
                            "Cannot delete file {} even if the occupying process has been killed.",
                            path.display()
                        )
                    })?;
                    println!("Deleted file {path:?}");
                } else {
                    eprintln!("Skipped deleting file {path:?}");
                }
            }
            Ok(path)
        })?;
    }
    Ok(())
}
