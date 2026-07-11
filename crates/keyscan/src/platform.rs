//! Per-OS process memory access: enumerate readable regions + read at an address.

#[cfg(unix)]
pub use unix::{ProcMem, PERMISSION_HINT};
#[cfg(windows)]
pub use win::{ProcMem, PERMISSION_HINT};

#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::FileExt;

    pub const PERMISSION_HINT: &str =
        " (on Linux the game runs under Proton; if reading its memory is denied, run the editor with elevated permissions)";

    /// Reads another process's memory via `/proc/<pid>/mem`.
    pub struct ProcMem {
        pid: u32,
        mem: fs::File,
    }

    impl ProcMem {
        pub fn open(pid: u32) -> Result<Self, String> {
            let mem = fs::File::open(format!("/proc/{pid}/mem")).map_err(|e| e.to_string())?;
            Ok(Self { pid, mem })
        }

        /// Readable `[start, end)` regions from `/proc/<pid>/maps`.
        pub fn regions(&self) -> Vec<(u64, u64)> {
            let Ok(text) = fs::read_to_string(format!("/proc/{}/maps", self.pid)) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for line in text.lines() {
                let mut it = line.split_whitespace();
                let (Some(range), Some(perms)) = (it.next(), it.next()) else { continue };
                if !perms.starts_with('r') {
                    continue;
                }
                let path = it.nth(3).unwrap_or("");
                if matches!(path, "[vvar]" | "[vsyscall]" | "[vdso]") {
                    continue;
                }
                let Some((s, e)) = range.split_once('-') else { continue };
                if let (Ok(s), Ok(e)) = (u64::from_str_radix(s, 16), u64::from_str_radix(e, 16)) {
                    out.push((s, e));
                }
            }
            out
        }

        /// Read into `buf` at `addr`; returns bytes read (None on failure).
        pub fn read_at(&self, addr: u64, buf: &mut [u8]) -> Option<usize> {
            self.mem.read_at(buf, addr).ok().filter(|&n| n > 0)
        }
    }
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Memory::{
        VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    pub const PERMISSION_HINT: &str =
        " (if reading the game's memory is denied, try running the editor as administrator)";

    /// Reads another process's memory via the Win32 API.
    pub struct ProcMem {
        handle: HANDLE,
    }

    impl ProcMem {
        pub fn open(pid: u32) -> Result<Self, String> {
            let handle = unsafe {
                OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
            }
            .map_err(|e| e.to_string())?;
            Ok(Self { handle })
        }

        /// Committed, readable `[start, end)` regions via VirtualQueryEx.
        pub fn regions(&self) -> Vec<(u64, u64)> {
            let mut out = Vec::new();
            let mut addr: usize = 0;
            loop {
                let mut mbi = MEMORY_BASIC_INFORMATION::default();
                let n = unsafe {
                    VirtualQueryEx(
                        self.handle,
                        Some(addr as *const _),
                        &mut mbi,
                        std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                    )
                };
                if n == 0 {
                    break;
                }
                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                let prot = mbi.Protect;
                let readable = mbi.State == MEM_COMMIT
                    && (prot.0 & PAGE_NOACCESS.0) == 0
                    && (prot.0 & PAGE_GUARD.0) == 0;
                if readable && size > 0 {
                    out.push((base as u64, (base + size) as u64));
                }
                let next = base.checked_add(size);
                match next {
                    Some(a) if a > addr => addr = a,
                    _ => break,
                }
            }
            out
        }

        /// Read into `buf` at `addr`; returns bytes read (None on failure).
        pub fn read_at(&self, addr: u64, buf: &mut [u8]) -> Option<usize> {
            let mut read = 0usize;
            let ok = unsafe {
                ReadProcessMemory(
                    self.handle,
                    addr as *const _,
                    buf.as_mut_ptr() as *mut _,
                    buf.len(),
                    Some(&mut read),
                )
            };
            ok.is_ok().then_some(read).filter(|&n| n > 0)
        }
    }

    impl Drop for ProcMem {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}
