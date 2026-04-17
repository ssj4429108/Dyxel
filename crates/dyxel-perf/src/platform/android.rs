// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Android system info provider using JNI and /proc filesystem

use crate::SystemInfoProvider;
use std::fs;

pub struct AndroidSystemInfo;

impl SystemInfoProvider for AndroidSystemInfo {
    fn get_memory_usage(&self) -> Option<(u64, Option<u64>)> {
        // Try to read from /proc/self/status first
        if let Ok(content) = fs::read_to_string("/proc/self/status") {
            let mut vm_rss = None;
            let mut vm_size = None;

            for line in content.lines() {
                if line.starts_with("VmRSS:") {
                    // Format: "VmRSS:    12345 kB"
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        vm_rss = parts[1].parse::<u64>().ok().map(|v| v * 1024);
                    }
                }
                if line.starts_with("VmSize:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        vm_size = parts[1].parse::<u64>().ok().map(|v| v * 1024);
                    }
                }
            }

            if let Some(used) = vm_rss {
                return Some((used, vm_size));
            }
        }

        // Fallback to /proc/self/statm
        if let Ok(content) = fs::read_to_string("/proc/self/statm") {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() >= 2 {
                // statm format: size resident shared text lib data dt
                // Values are in pages
                if let (Ok(resident), Ok(_)) = (parts[1].parse::<u64>(), parts[0].parse::<u64>()) {
                    let page_size = unsafe { libc::sysconf(libc::_SC_PAGE_SIZE) as u64 };
                    let used = resident * page_size;
                    return Some((used, None));
                }
            }
        }

        None
    }

    fn get_cpu_usage(&self) -> Option<f32> {
        // Read process CPU time from /proc/self/stat
        // Format: pid comm state ppid pgrp session tty_nr tpgid flags minflt cminflt majflt cmajflt utime stime cutime cstime ...
        if let Ok(content) = fs::read_to_string("/proc/self/stat") {
            // Find the closing parenthesis for comm (command name) which can contain spaces
            if let Some(start) = content.find(')') {
                let after_comm = &content[start + 1..];
                let parts: Vec<&str> = after_comm.split_whitespace().collect();

                // utime is at index 11 (after state, ppid, pgrp, session, tty_nr, tpgid, flags, minflt, cminflt, majflt, cmajflt)
                // stime is at index 12
                if parts.len() >= 14 {
                    if let (Ok(utime), Ok(stime)) =
                        (parts[11].parse::<u64>(), parts[12].parse::<u64>())
                    {
                        // CPU time is in clock ticks
                        let ticks_per_sec = unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 };
                        let total_time = (utime + stime) as f64 / ticks_per_sec;

                        // Get process uptime
                        if let Ok(uptime_content) = fs::read_to_string("/proc/uptime") {
                            let uptime_parts: Vec<&str> =
                                uptime_content.split_whitespace().collect();
                            if let Ok(uptime) = uptime_parts.get(0)?.parse::<f64>() {
                                // Calculate CPU percentage
                                // This is a simplified calculation - for accurate measurement
                                // we should track delta over time
                                let cpu_percent = (total_time / uptime * 100.0).min(100.0) as f32;
                                return Some(cpu_percent);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Get CPU temperature in Celsius (if available)
    fn get_temperature(&self) -> Option<f32> {
        // Try to read CPU temperature from thermal zones
        for i in 0..20 {
            let path = format!("/sys/class/thermal/thermal_zone{}/temp", i);
            let type_path = format!("/sys/class/thermal/thermal_zone{}/type", i);

            // Check if this is a CPU thermal zone
            if let Ok(zone_type) = fs::read_to_string(&type_path) {
                let zone_type = zone_type.trim();
                if zone_type.contains("cpu")
                    || zone_type.contains("CPU")
                    || zone_type.contains("tsens")
                {
                    if let Ok(temp_str) = fs::read_to_string(&path) {
                        if let Ok(temp_millidegrees) = temp_str.trim().parse::<i32>() {
                            // Some devices report in millidegrees, others in degrees
                            let temp_celsius = if temp_millidegrees > 1000 {
                                temp_millidegrees as f32 / 1000.0
                            } else {
                                temp_millidegrees as f32
                            };
                            return Some(temp_celsius);
                        }
                    }
                }
            }
        }

        // Fallback: try any available thermal zone
        for i in 0..20 {
            let path = format!("/sys/class/thermal/thermal_zone{}/temp", i);
            if let Ok(temp_str) = fs::read_to_string(&path) {
                if let Ok(temp_millidegrees) = temp_str.trim().parse::<i32>() {
                    let temp_celsius = if temp_millidegrees > 1000 {
                        temp_millidegrees as f32 / 1000.0
                    } else {
                        temp_millidegrees as f32
                    };
                    return Some(temp_celsius);
                }
            }
        }

        None
    }

    fn platform_name(&self) -> &'static str {
        "android"
    }
}
