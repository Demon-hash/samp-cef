use winapi::shared::minwindef::{BOOL, DWORD, HMODULE};
use winapi::um::errhandlingapi::{
    AddVectoredExceptionHandler, PTOP_LEVEL_EXCEPTION_FILTER, SetUnhandledExceptionFilter,
};
use winapi::um::fileapi::CreateFileA;
use winapi::um::processthreadsapi::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};
use winapi::um::psapi::{
    EnumProcessModules, GetModuleFileNameExA, GetModuleInformation, MODULEINFO,
};
use winapi::um::winnt::{EXCEPTION_POINTERS, FILE_ATTRIBUTE_NORMAL, GENERIC_WRITE, HANDLE, LONG};
use winapi::vc::excpt::EXCEPTION_CONTINUE_SEARCH;

use std::io::Write;
use std::os::raw::c_void;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const CREATE_ALWAYS: DWORD = 2;
const FILE_SHARE_READ: DWORD = 0x1;

// This winapi version (0.3.9) doesn't declare the DbgHelp minidump API at
// all (verified against the crate source - src/um/dbghelp.rs has no
// MiniDump* items), even though "dbghelp" is a valid feature name for the
// rest of that header. MiniDumpWriteDump's signature has been stable since
// Windows XP, so binding it by hand here is safe and avoids pulling in an
// extra crate just for one function.
type MinidumpType = DWORD;
const MINI_DUMP_NORMAL: MinidumpType = 0x0000_0000;
const MINI_DUMP_WITH_DATA_SEGS: MinidumpType = 0x0000_0001;

#[repr(C)]
struct MinidumpExceptionInformation {
    thread_id: DWORD,
    exception_pointers: *mut EXCEPTION_POINTERS,
    client_pointers: BOOL,
}

#[link(name = "dbghelp")]
unsafe extern "system" {
    fn MiniDumpWriteDump(
        h_process: HANDLE,
        process_id: DWORD,
        h_file: HANDLE,
        dump_type: MinidumpType,
        exception_param: *mut MinidumpExceptionInformation,
        user_stream_param: *mut c_void,
        callback_param: *mut c_void,
    ) -> BOOL;
}

static mut EXCEPTION_FILTER: PTOP_LEVEL_EXCEPTION_FILTER = None;
static mut PLAYTIME: Option<Instant> = None;
static mut ALREADY_SENT: bool = false;
static mut DUMP_WRITTEN: bool = false;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CrashReport {
    // player relative
    mem_used: u32,
    mem_available: u32,
    // exception
    base_addr: usize,
    exception_addr: usize,
    exception_code: usize,
    exception_library: String,
    registers: Registers,
    modules: Vec<Module>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Registers {
    eax: DWORD,
    ebx: DWORD,
    ecx: DWORD,
    edx: DWORD,
    esi: DWORD,
    edi: DWORD,
    ebp: DWORD,
    esp: DWORD,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Module {
    name: String,
    addr: usize,
    size: usize,
}

pub fn initialize() {
    unsafe {
        EXCEPTION_FILTER = SetUnhandledExceptionFilter(Some(exception_filter));
        PLAYTIME = Some(Instant::now());

        // SetUnhandledExceptionFilter is a single global slot - whichever
        // module (us, $fastman92limitAdjuster.asi, ...) calls it *last*
        // wins, with no automatic chaining, so relying on it alone is a
        // coin flip on injection order. AddVectoredExceptionHandler has no
        // such problem: every registered handler runs, in registration
        // order, regardless of what else is installed - this is what
        // guarantees we actually get a minidump on a real crash instead of
        // silently losing the race to whichever ASI's filter happens to
        // install after ours.
        AddVectoredExceptionHandler(1, Some(vectored_handler));
    }
}

unsafe extern "system" fn vectored_handler(exception_info: *mut EXCEPTION_POINTERS) -> LONG {
    const EXCEPTION_ACCESS_VIOLATION: DWORD = 0xC0000005;
    const EXCEPTION_ILLEGAL_INSTRUCTION: DWORD = 0xC000001D;
    const EXCEPTION_STACK_OVERFLOW: DWORD = 0xC00000FD;

    let code = (*(*exception_info).ExceptionRecord).ExceptionCode as DWORD;

    // Vectored handlers see *every* exception, including routine
    // first-chance ones (C++ exceptions, etc.) that fire constantly during
    // normal operation - only act on the fault kinds that actually mean
    // "this process is about to die", and only once per session.
    let is_fatal = matches!(
        code,
        EXCEPTION_ACCESS_VIOLATION | EXCEPTION_ILLEGAL_INSTRUCTION | EXCEPTION_STACK_OVERFLOW
    );

    if is_fatal && !DUMP_WRITTEN {
        DUMP_WRITTEN = true;
        write_minidump(exception_info);
    }

    EXCEPTION_CONTINUE_SEARCH
}

unsafe fn write_minidump(exception_info: *mut EXCEPTION_POINTERS) {
    let mut exe_path = [0u8; 260];
    let len = winapi::um::libloaderapi::GetModuleFileNameA(
        std::ptr::null_mut(),
        exe_path.as_mut_ptr() as *mut i8,
        exe_path.len() as u32,
    );

    if len == 0 {
        return;
    }

    let exe_path = String::from_utf8_lossy(&exe_path[..len as usize]).to_string();
    let dir = match exe_path.rfind(['\\', '/']) {
        Some(idx) => &exe_path[..idx],
        None => ".",
    };

    let dump_dir = format!("{dir}\\CrashDumps");
    let _ = std::fs::create_dir_all(&dump_dir);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let pid = GetCurrentProcessId();
    let path = format!("{dump_dir}\\cef-client-{pid}-{timestamp}.dmp\0");

    let file = CreateFileA(
        path.as_ptr() as *const i8,
        GENERIC_WRITE,
        FILE_SHARE_READ,
        std::ptr::null_mut(),
        CREATE_ALWAYS,
        FILE_ATTRIBUTE_NORMAL,
        std::ptr::null_mut(),
    );

    if file.is_null() || file == winapi::um::handleapi::INVALID_HANDLE_VALUE {
        tracing::error!(path, "crash_logger: failed to create minidump file");
        return;
    }

    let mut exception_params = MinidumpExceptionInformation {
        thread_id: GetCurrentThreadId(),
        exception_pointers: exception_info,
        client_pointers: 0,
    };

    // MINI_DUMP_WITH_DATA_SEGS (global/static data, e.g. our own statics
    // and the Manager's state) plus the default thread contexts/stacks is
    // enough to inspect the corrupted state and get a real call stack for
    // every thread, without the size and extra write time of a full
    // full-memory dump while already in a crashed process.
    let written = MiniDumpWriteDump(
        GetCurrentProcess(),
        GetCurrentProcessId(),
        file,
        MINI_DUMP_NORMAL | MINI_DUMP_WITH_DATA_SEGS,
        &mut exception_params,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    );

    winapi::um::handleapi::CloseHandle(file);

    if written == 0 {
        tracing::error!(path, "crash_logger: MiniDumpWriteDump failed");
    } else {
        tracing::error!(path, "crash_logger: minidump written");
    }
}

unsafe extern "system" fn exception_filter(exception_info: *mut EXCEPTION_POINTERS) -> LONG {
    if ALREADY_SENT {
        // EXCEPTION_FILTER is Copy (it's just a function pointer) - read
        // it by value instead of `.as_mut()`, which would create a `&mut`
        // to a `static mut` and is a hard error under edition 2024.
        if let Some(origin) = EXCEPTION_FILTER {
            return origin(exception_info);
        }

        return EXCEPTION_CONTINUE_SEARCH;
    } else {
        ALREADY_SENT = true;
    }

    let info = &mut *exception_info;
    let context = &mut *info.ContextRecord;
    let exception = &mut *info.ExceptionRecord;

    let registers = Registers {
        eax: context.Eax,
        ebx: context.Ebx,
        ecx: context.Ecx,
        edx: context.Edx,
        esi: context.Esi,
        edi: context.Edi,
        ebp: context.Ebp,
        esp: context.Esp,
    };

    let process = GetCurrentProcess();
    let mut module_handles: [HMODULE; 1024] = [0 as *mut _; 1024];
    let mut found = 0;

    EnumProcessModules(
        process,
        module_handles.as_mut_ptr(),
        module_handles.len() as _,
        &mut found,
    );

    let mut bytes = [0i8; 1024];
    let mut modules = Vec::with_capacity((found / 4) as usize);
    let mut module_information = MODULEINFO {
        lpBaseOfDll: std::ptr::null_mut(),
        SizeOfImage: 0,
        EntryPoint: std::ptr::null_mut(),
    };
    let mut exception_library = String::from("Unknown module");

    for i in 0..(found / 4) {
        if GetModuleFileNameExA(
            process,
            module_handles[i as usize],
            bytes.as_mut_ptr(),
            1024,
        ) != 0
            && GetModuleInformation(
                process,
                module_handles[i as usize],
                &mut module_information,
                std::mem::size_of::<MODULEINFO>() as _,
            ) != 0
        {
            let string = std::ffi::CStr::from_ptr(bytes.as_ptr());

            let e_addr = exception.ExceptionAddress as usize;
            let m_addr = module_handles[i as usize] as usize;
            let m_size = module_information.SizeOfImage as usize;

            if e_addr >= m_addr && e_addr < m_addr + m_size {
                exception_library = string.to_string_lossy().to_string();
            }

            modules.push(Module {
                name: string.to_string_lossy().to_string(),
                addr: m_addr,
                size: m_size,
            });
        }
    }

    let report = CrashReport {
        mem_used: *(0x8E4CB4 as *mut u32),
        mem_available: *(0x8A5A80 as *mut u32),
        base_addr: client_api::samp::handle() as usize,
        exception_addr: exception.ExceptionAddress as usize,
        exception_code: exception.ExceptionCode as usize,
        exception_library,
        registers,
        modules,
    };

    tracing::error!(
        report = %serde_json::to_string(&report).unwrap(),
        "client crash captured"
    );

    if let Some(origin) = EXCEPTION_FILTER {
        return origin(exception_info);
    }

    return EXCEPTION_CONTINUE_SEARCH;
}
