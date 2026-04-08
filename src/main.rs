use std::{thread, time};
use sysinfo::System;

use swayipc::{Connection, Fallible};

fn main() -> Fallible<()> {
    let mut ipc = Connection::new()?;
    let mut sys = System::new();
    loop {
        sys.refresh_cpu_usage();
        let cpus = sys.cpus();
        let avg_cpu_usage: f32 =
            cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32;
        std::thread::sleep(time::Duration::from_secs(2));
        while avg_cpu_usage > 50.0 {
            window_breathing(&mut ipc);
        }
    }
}

fn window_breathing(ipc: &mut Connection) {
    let mut counter = 0;
    let mut dir = 2;
    let base_thickness = 5;
    let mut border_thickness = base_thickness;
    loop {
        border_thickness = border_thickness + dir;
        counter += 1;
        _ = ipc.run_command(format!("border pixel {}", border_thickness));
        _ = ipc.run_command(format!("gaps outer current minus {}", dir));
        thread::sleep(time::Duration::from_millis(100));
        if counter % 6 == 0 {
            dir *= -1
        }
        if counter % 12 == 0 {
            thread::sleep(time::Duration::from_secs(2));
        }
    }
}
