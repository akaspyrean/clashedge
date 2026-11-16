use std::io;

#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use tokio::process::Child;

#[cfg(windows)]
use std::sync::Mutex;

#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

#[cfg(windows)]
pub struct JobHandle {
    _handle: OwnedHandle,
}

#[cfg(windows)]
static JOB_HANDLES: Mutex<Vec<JobHandle>> = Mutex::new(Vec::new());

#[cfg(windows)]
pub fn assign_child_to_job(child: &mut Child) -> io::Result<()> {
    let raw_job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw_job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = unsafe { OwnedHandle::from_raw_handle(raw_job) };

    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let set_ok = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set_ok == 0 {
        let error = io::Error::last_os_error();
        drop(job);
        return Err(error);
    }

    let process_handle = child
        .raw_handle()
        .ok_or_else(|| io::Error::other("mihomo child has no process handle"))?
        as HANDLE;
    if unsafe { AssignProcessToJobObject(job.as_raw_handle() as HANDLE, process_handle) } == 0 {
        let error = io::Error::last_os_error();
        drop(job);
        let _ = child.start_kill();
        return Err(error);
    }

    let mut handles = JOB_HANDLES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    handles.clear();
    handles.push(JobHandle { _handle: job });
    Ok(())
}

#[cfg(not(windows))]
pub fn assign_child_to_job(_child: &Child) -> io::Result<()> {
    Ok(())
}
