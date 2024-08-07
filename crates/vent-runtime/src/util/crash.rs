use chrono::{DateTime, Local};
use std::fs::File;
use std::io::Write;
use std::panic::{self, PanicInfo};
use sysinfo::System;

// Crash Handler

#[inline]
pub fn init_panic_hook() {
    panic::set_hook(Box::new(panic_handler));
}

fn panic_handler(pi: &PanicInfo) {
    eprintln!("Crash: {}", pi);
    log_crash(pi).expect("Failed to Log Crash File");
    // show_error_dialog(pi);
}

fn log_crash(pi: &PanicInfo) -> std::io::Result<()> {
    let timestamp: DateTime<Local> = Local::now();

    let sys = System::new_all();
    // let drivers = get_driver_information(); // Implement this function to retrieve driver information

    // Generate log file name based on timestamp
    let log_file_name = format!("crash/crash_log_{}.log", timestamp.format("%Y%m%d%H%M%S"));

    // Create and write crash information to the log file

    let mut f = File::create(log_file_name)?;
    let mut perms = f.metadata()?.permissions();
    perms.set_readonly(true);
    f.set_permissions(perms)?;

    writeln!(&mut f, "--Crash Log--")?;
    writeln!(
        &mut f,
        "System kernel version:   {:?}",
        sysinfo::System::kernel_version()
    )?;
    writeln!(
        &mut f,
        "System OS version:       {:?}",
        sysinfo::System::os_version()
    )?;
    writeln!(
        &mut f,
        "System host name:        {:?}",
        sysinfo::System::host_name()
    )?;

    writeln!(
        &mut f,
        "System Core Count:             {:?}",
        sys.physical_core_count()
    )?;
    writeln!(
        &mut f,
        "System Total Memory:             {:?}",
        sys.total_memory()
    )?;

    //   writeln!(&mut f, "Driver Information: {}", drivers)?;
    writeln!(&mut f, "Crash Info:              {:?}", format!("{:?}", pi))?;
    Ok(())
}
