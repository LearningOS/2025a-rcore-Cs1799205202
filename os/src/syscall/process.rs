//! Process management syscalls
use crate::config::PAGE_SIZE;
use crate::mm::{PageTable, VirtAddr, PTEFlags};
use crate::task::{
    change_program_brk, current_user_token, exit_current_and_run_next, get_syscall_times,
    mmap_current, munmap_current, suspend_current_and_run_next,
};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let ts_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };

    let token = current_user_token();
    let page_table = PageTable::from_token(token);

    let start_va = VirtAddr::from(ts as usize);
    let end_va = VirtAddr::from(ts as usize + core::mem::size_of::<TimeVal>());

    let bytes = unsafe {
        core::slice::from_raw_parts(
            &ts_val as *const _ as *const u8,
            core::mem::size_of::<TimeVal>(),
        )
    };

    let mut start = start_va.0;
    let end = end_va.0;
    let mut offset = 0;

    while start < end {
        let va = VirtAddr::from(start);
        let vpn = va.floor();
        let offset_in_page = va.page_offset();

        match page_table.translate(vpn) {
            Some(pte) => {
                if !pte.writable() || !pte.is_valid() || !pte.flags().contains(PTEFlags::U) {
                    return -1;
                }
                let ppn = pte.ppn();
                let len = core::cmp::min(PAGE_SIZE - offset_in_page, end - start);
                let dst = &mut ppn.get_bytes_array()[offset_in_page..offset_in_page + len];
                dst.copy_from_slice(&bytes[offset..offset + len]);

                start += len;
                offset += len;
            }
            None => return -1,
        }
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = current_user_token();
    let page_table = PageTable::from_token(token);

    match trace_request {
        0 => {
            // Read byte from id
            let va = VirtAddr::from(id);
            let vpn = va.floor();
            let offset = va.page_offset();
            match page_table.translate(vpn) {
                Some(pte) => {
                    if !pte.readable() || !pte.is_valid() || !pte.flags().contains(PTEFlags::U) {
                        return -1;
                    }
                    let ppn = pte.ppn();
                    let val = ppn.get_bytes_array()[offset];
                    val as isize
                }
                None => -1,
            }
        }
        1 => {
            // Write data to id
            let va = VirtAddr::from(id);
            let vpn = va.floor();
            let offset = va.page_offset();
            match page_table.translate(vpn) {
                Some(pte) => {
                    if !pte.writable() || !pte.is_valid() || !pte.flags().contains(PTEFlags::U) {
                        return -1;
                    }
                    let ppn = pte.ppn();
                    ppn.get_bytes_array()[offset] = data as u8;
                    0
                }
                None => -1,
            }
        }
        2 => get_syscall_times(id) as isize,
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    mmap_current(start, len, prot)
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    munmap_current(start, len)
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
