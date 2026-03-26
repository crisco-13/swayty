use std::{thread, time};

use swayipc::{Connection, Fallible};

fn main() -> Fallible<()> {
    let mut ipc = Connection::new()?;
    window_breathing(&mut ipc);
    Ok(())
}

fn window_breathing(ipc: &mut Connection) {
    let mut counter = 0;
    let mut dir = 2;
    let base_thickness = 5;
    let mut border_thickness = base_thickness;
    loop {
        border_thickness = border_thickness + dir;
        counter += 1;
        if counter % 6 == 0 {
            dir *= -1
        }
        _ = ipc.run_command(format!("border pixel {}", border_thickness));
        _ = ipc.run_command(format!("gaps outer current minus {}", dir));
        thread::sleep(time::Duration::from_millis(100));
        if counter % 12 == 0 {
            thread::sleep(time::Duration::from_secs(2));
        }
    }
}
