// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! macOS system info provider using libc

use crate::SystemInfoProvider;

pub struct MacSystemInfo;

impl SystemInfoProvider for MacSystemInfo {
    fn get_memory_usage(&self) -> Option<(u64, Option<u64>)> {
        unsafe {
            // Get current process memory info using task_info
            let mut info: libc::mach_task_basic_info = std::mem::zeroed();
            let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
            
            let result = libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                &mut info as *mut _ as libc::task_info_t,
                &mut count,
            );
            
            if result == libc::KERN_SUCCESS {
                let used = info.resident_size as u64;
                
                // Get total physical memory
                let mut total_memory: u64 = 0;
                let mut size = std::mem::size_of::<u64>();
                let result = libc::sysctlbyname(
                    b"hw.memsize\0".as_ptr() as *const i8,
                    &mut total_memory as *mut _ as *mut libc::c_void,
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                );
                
                let available = if result == 0 {
                    Some(total_memory)
                } else {
                    None
                };
                
                return Some((used, available));
            }
        }
        
        None
    }
    
    fn get_cpu_usage(&self) -> Option<f32> {
        unsafe {
            // Get CPU load using host_statistics
            let mut cpu_info: libc::host_cpu_load_info = std::mem::zeroed();
            let mut count = libc::HOST_CPU_LOAD_INFO_COUNT as u32;
            
            let result = libc::host_statistics(
                libc::mach_host_self(),
                libc::HOST_CPU_LOAD_INFO,
                &mut cpu_info as *mut _ as libc::host_info_t,
                &mut count,
            );
            
            if result == libc::KERN_SUCCESS {
                let user = cpu_info.cpu_ticks[libc::CPU_STATE_USER as usize] as u64;
                let system = cpu_info.cpu_ticks[libc::CPU_STATE_SYSTEM as usize] as u64;
                let idle = cpu_info.cpu_ticks[libc::CPU_STATE_IDLE as usize] as u64;
                let nice = cpu_info.cpu_ticks[libc::CPU_STATE_NICE as usize] as u64;
                
                let total = user + system + idle + nice;
                if total > 0 {
                    let used = user + system + nice;
                    let usage = (used as f64 / total as f64 * 100.0) as f32;
                    return Some(usage);
                }
            }
        }
        
        None
    }
    
    fn platform_name(&self) -> &'static str {
        "macos"
    }
}
