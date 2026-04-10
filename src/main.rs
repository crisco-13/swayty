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

        let (animation_speed, frecuency) = swaytiness_calculator(avg_cpu_usage);

        if animation_speed > 0 {
            window_breathing(&mut ipc, animation_speed);
        }

        std::thread::sleep(time::Duration::from_millis(frecuency));
    }
}

fn window_breathing(ipc: &mut Connection, speed: u64) {
    let mut counter = 0;
    let mut dir = 2;
    let base_thickness = 5;
    let mut border_thickness = base_thickness;

    while counter < 12 {
        border_thickness = border_thickness + dir;
        counter += 1;
        _ = ipc.run_command(format!("border pixel {}", border_thickness));
        _ = ipc.run_command(format!("gaps outer current minus {}", dir));
        thread::sleep(time::Duration::from_millis(speed));
        if counter % 6 == 0 {
            dir *= -1
        }
    }
}

fn swaytiness_calculator(cpu_usage: f32) -> (u64, u64) {
    if cpu_usage < 20.0 {
        (0, 2000)
    } else {
        let breathing_speed = 150 - cpu_usage as u64;
        let loop_frecuency = 3900 - (38.0 * cpu_usage) as u64;
        (breathing_speed, loop_frecuency)
    }
}
