use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HWND, LPARAM, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE};
use windows_sys::Win32::System::Threading::{
	GetExitCodeThread, IsWow64Process, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject,
	PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
	PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
	EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

#[derive(Clone)]
pub struct ClientWindow {
	pub pid: u32,
	pub title: String,
}

fn wide(input: &OsStr) -> Vec<u16> {
	input.encode_wide().chain(std::iter::once(0)).collect()
}

pub fn detect() -> Vec<ClientWindow> {
	unsafe extern "system" fn callback(hwnd: HWND, state: LPARAM) -> i32 {
		if IsWindowVisible(hwnd) == 0 {
			return 1;
		}
		let mut buffer = [0u16; 512];
		let count = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
		if count <= 0 {
			return 1;
		}
		let title = String::from_utf16_lossy(&buffer[..count as usize]);
		let lowercase = title.to_ascii_lowercase();
		if !lowercase.contains("minecraft") && !lowercase.contains("lunar client") {
			return 1;
		}
		let mut pid = 0u32;
		GetWindowThreadProcessId(hwnd, &mut pid);
		if pid == 0 {
			return 1;
		}
		let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
		if process == 0 {
			return 1;
		}
		let mut exe = [0u16; 1024];
		let mut len = exe.len() as u32;
		let valid = QueryFullProcessImageNameW(process, 0, exe.as_mut_ptr(), &mut len) != 0;
		CloseHandle(process);
		if !valid {
			return 1;
		}
		let path = String::from_utf16_lossy(&exe[..len as usize]).to_ascii_lowercase();
		if !path.ends_with("javaw.exe") && !path.ends_with("java.exe") {
			return 1;
		}
		let result = &mut *(state as *mut Vec<ClientWindow>);
		if !result.iter().any(|window| window.pid == pid) {
			result.push(ClientWindow { pid, title });
		}
		1
	}
	let mut windows: Vec<ClientWindow> = Vec::new();
	unsafe {
		EnumWindows(Some(callback), &mut windows as *mut _ as LPARAM);
	}
	windows.sort_by_key(|window| window.pid);
	windows
}

type ThreadStart = Option<unsafe extern "system" fn(*mut c_void) -> u32>;
#[link(name = "kernel32")]
extern "system" {
	fn CreateRemoteThread(
		process: isize,
		attributes: *const c_void,
		stack_size: usize,
		start: ThreadStart,
		parameter: *mut c_void,
		flags: u32,
		thread_id: *mut u32,
	) -> isize;
}

pub fn inject(pid: u32, dll: &Path) -> Result<(), String> {
	let path = dll.canonicalize().map_err(|error| format!("Bridge DLL missing: {error}"))?;
	let wide_path = wide(path.as_os_str());
	let bytes = wide_path.len() * std::mem::size_of::<u16>();
	let rights = PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION
		| PROCESS_VM_WRITE | PROCESS_VM_READ;
	let process = unsafe { OpenProcess(rights, 0, pid) };
	if process == 0 {
		return Err(format!("OpenProcess failed ({})", unsafe { GetLastError() }));
	}
	let result = unsafe {
		let mut is_wow64 = 0i32;
		if IsWow64Process(process, &mut is_wow64) == 0 || is_wow64 != 0 {
			CloseHandle(process);
			return Err("Target must be a 64-bit Java process".into());
		}
		let buffer = VirtualAllocEx(process, ptr::null(), bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
		if buffer.is_null() {
			Err(format!("VirtualAllocEx failed ({})", GetLastError()))
		} else {
			let mut release_buffer = true;
			let result = (|| {
				let mut written = 0usize;
				if WriteProcessMemory(process, buffer, wide_path.as_ptr() as *const c_void, bytes, &mut written) == 0
					|| written != bytes {
					return Err(format!("WriteProcessMemory failed ({})", GetLastError()));
				}
				let kernel32 = GetModuleHandleW(wide(OsStr::new("kernel32.dll")).as_ptr());
				if kernel32 == 0 {
					return Err("kernel32.dll not found".into());
				}
				let procedure = GetProcAddress(kernel32, b"LoadLibraryW\0".as_ptr())
					.ok_or("LoadLibraryW not found")?;
				let start = std::mem::transmute::<unsafe extern "system" fn() -> isize,
					unsafe extern "system" fn(*mut c_void) -> u32>(procedure);
				let thread = CreateRemoteThread(process, ptr::null(), 0, Some(start), buffer, 0, ptr::null_mut());
				if thread == 0 {
					return Err(format!("CreateRemoteThread failed ({})", GetLastError()));
				}
				let wait = WaitForSingleObject(thread, 10_000);
				if wait != WAIT_OBJECT_0 {
					release_buffer = false;
					CloseHandle(thread);
					return Err("Injection timed out; check whether the DLL was loaded".into());
				}
				let mut exit_code = 0u32;
				let exit_ok = GetExitCodeThread(thread, &mut exit_code);
				CloseHandle(thread);
				if exit_ok == 0 || exit_code == 0 {
					return Err("LoadLibraryW failed inside the target process".into());
				}
				Ok(())
			})();
			if release_buffer {
				VirtualFreeEx(process, buffer, 0, MEM_RELEASE);
			}
			result
		}
	};
	unsafe { CloseHandle(process) };
	result
}
