// NOTICE file corresponding to the section 4 d of the Apache License,
// Version 2.0

// This product includes software developed at
// https://github.com/Kudaes/Bin-Finder

// Copyright [Year] [Kudaes]

// This product includes software licensed under the Apache License, Version 2.0 (the "License").
// You may not use this file except in compliance with the License.
// You may obtain a copy of the License at

// http://www.apache.org/licenses/LICENSE-2.0

// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod cli;

use bindings::Windows::Win32::Foundation::HANDLE;
use bindings::Windows::Win32::System::WindowsProgramming::IO_STATUS_BLOCK;
use clap::Parser;
use cli::Cli;
use data::{CreateFile, FILE_PROCESS_IDS_USING_FILE_INFORMATION, PVOID};
use std::mem::size_of;
use std::path::Path;
use std::process::Command;
use std::{fs, io, ptr};

const FILE_READ_ATTRIBUTES: u32 = 0x80;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE: u32 = 0x00000001 | 0x00000002 | 0x00000004;
const FILE_PROCESS_IDS_INFO: u32 = 47;

#[allow(clippy::missing_safety_doc)]
pub unsafe fn get_pid_from_image_path(path: &str) -> Result<Vec<usize>, String> {
    let mut file: Vec<u16> = path.encode_utf16().collect();
    file.push(0);
    let k32 = dinvoke::get_module_base_address("kernel32.dll");
    let create_file: CreateFile;
    let create_file_r: Option<HANDLE>;

    dinvoke::dynamic_invoke!(
        k32,
        "CreateFileW",
        create_file,
        create_file_r,
        file.as_ptr(),
        FILE_READ_ATTRIBUTES,
        FILE_SHARE,
        ptr::null(),
        OPEN_EXISTING,
        0,
        HANDLE(0)
    );

    let file_handle = match create_file_r {
        Some(handle) => handle,
        None => return Err("Failed to create file handle".to_string()),
    };

    if file_handle.0 == -1 {
        return Err("Invalid file handle".to_string());
    }

    let mut buffer;
    let mut bytes = size_of::<FILE_PROCESS_IDS_USING_FILE_INFORMATION>() as u32;
    for _ in 0..20 {
        buffer = vec![0u8; bytes as usize];
        let ptr: PVOID = std::mem::transmute(buffer.as_ptr());
        let ios: Vec<u8> = vec![0u8; size_of::<IO_STATUS_BLOCK>()];
        let iosb: *mut IO_STATUS_BLOCK = std::mem::transmute(&ios);

        let x = dinvoke::nt_query_information_file(
            file_handle,
            iosb,
            ptr,
            bytes,
            FILE_PROCESS_IDS_INFO,
        );

        if x != 0 {
            bytes *= 2;
        } else {
            let fpi: *mut FILE_PROCESS_IDS_USING_FILE_INFORMATION = std::mem::transmute(ptr);
            let _r = dinvoke::close_handle(file_handle);
            // Access denied error occurs if this pointer is not liberated.
            (*iosb).Anonymous.Pointer = ptr::null_mut();
            return Ok((*fpi)
                .process_id_list
                .into_iter()
                .filter(|x| x > &4)
                .collect());
        }
    }
    Err("Timeout. Call to NtQueryInformationFile failed.".to_string())
}

fn visit<F>(path: &Path, cb: &F) -> std::io::Result<()>
where
    F: Fn(&Path) -> Result<&Path, String>,
{
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            visit(&path, cb)?;
        }
        fs::remove_dir(path)?;
    } else if let Err(e) = cb(path) {
        eprintln!("Error occurs when deleting {}: {}", &path.display(), e);
    }
    Ok(())
}

fn get_user_input() -> String {
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .expect("Failed to read line");
    input.trim().to_lowercase()
}

fn yn_selector(message: &str, default: bool) -> bool {
    println!(
        "{} ({}/{}): ",
        message,
        if default { "Y" } else { "y" },
        if !default { "N" } else { "n" }
    );
    loop {
        let input = get_user_input();
        match input.as_str() {
            "" => return default,
            "y" | "yes" => return true,
            "n" | "no" => return false,
            _ => print!("Invalid input. Please enter 'y' or 'n' :"),
        }
    }
}

fn kill_process(pid: usize) -> std::io::Result<()> {
    let _ = Command::new("taskkill")
        .arg("/F")
        .arg("/PID")
        .arg(pid.to_string())
        .status()?;

    // no need to print status because the `taskkill` will status it as well.
    // if status.success() {
    //     println!("Successfully killed process with pid {}", pid);
    // } else {
    //     eprintln!("Failed to kill process with pid {}", pid);
    // }

    Ok(())
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    visit(cli.path.as_path(), &|path| unsafe {
        if let Err(e) = std::fs::remove_file(path) {
            eprint!("Failed to delete file {:?}: {}, ", path, e);
            let pid = get_pid_from_image_path(path.to_str().ok_or("Not a valid utf-8 filename.")?)?;
            if cli.yes || yn_selector(&format!("Kill process with pid {:?}?", pid), true) {
                for p in pid {
                    kill_process(p)
                        .map_err(|e| format!("Failed to kill process with pid {}: {}", p, e))?;
                }
            }
            let mut removed = Err(format!(
                "Cannot delete file {} even if the occupying process has been killed.",
                path.display()
            ));
            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if std::fs::remove_file(path).is_ok() {
                    removed = Ok(());
                    break;
                }
            }
            removed?;
        }
        println!("Deleted file {:?}", path);
        Ok(path)
    })?;
    Ok(())
}
