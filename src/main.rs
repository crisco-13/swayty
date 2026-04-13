use regex::Regex;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread, time,
};
use sysinfo::System;

use swayipc::{Connection, Fallible};

fn main() -> Fallible<()> {
    let ipc = Connection::new()?;
    let ipc = Arc::new(Mutex::new(Some(ipc)));

    let sway_config = fetch_sway_config(&ipc);
    let client_focused_colors = sway_config.and_then(|s| get_client_focused_colors(&s));

    let mut sys = System::new();

    let running = Arc::new(AtomicBool::new(true));

    {
        let running = running.clone();

        let mut signals = Signals::new(&[SIGINT, SIGTERM])?;
        thread::spawn(move || {
            for _sig in signals.forever() {
                running.store(false, Ordering::SeqCst);
                break;
            }
        });
    }

    while running.load(Ordering::SeqCst) {
        sys.refresh_cpu_usage();
        let cpus = sys.cpus();
        let avg_cpu_usage: f32 =
            cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32;

        let (animation_speed, frecuency) = swaytiness_calculator(avg_cpu_usage);

        if let Ok(mut guard) = ipc.lock() {
            if let Some(conn) = guard.as_mut() {
                if let Some(focused_colors) = &client_focused_colors {
                    border_coloring(conn, avg_cpu_usage, focused_colors);
                }

                if animation_speed > 0 {
                    window_breathing(conn, animation_speed);
                }
            }
        }

        std::thread::sleep(time::Duration::from_millis(frecuency));
    }

    if let Ok(mut guard) = ipc.lock() {
        if let Some(mut conn) = guard.take() {
            cleanup(&mut conn, client_focused_colors.as_ref());
        }
    }

    Ok(())
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

#[derive(Debug)]
struct SwayColor(String);

impl SwayColor {
    fn from_cpu_usage(cpu_usage: f32) -> Self {
        if cpu_usage <= 50.0 {
            let green = 255;
            let red = (5.1 * cpu_usage) as u8;
            SwayColor(format!("#{:02x}{:02x}00", red, green))
        } else {
            let green = (510.0 - 5.1 * cpu_usage) as u8;
            let red = 255;
            SwayColor(format!("#{:02x}{:02x}00", red, green))
        }
    }
}

fn border_coloring(ipc: &mut Connection, cpu_usage: f32, focused_colors: &FocusedColors) {
    let border_color = SwayColor::from_cpu_usage(cpu_usage);

    _ = ipc.run_command(format!(
        "client.focused {}99 {}99 {} {}",
        border_color.0, border_color.0, focused_colors.text, focused_colors.indicator
    ));
}

fn fetch_sway_config(ipc: &Arc<Mutex<Option<Connection>>>) -> Option<String> {
    let mut guard = ipc.lock().ok()?;
    let conn = guard.as_mut()?;
    conn.get_config().ok().map(|c| c.config)
}

#[derive(Debug, Clone)]
pub struct FocusedColors {
    pub border: String,
    pub background: String,
    pub text: String,
    pub indicator: String,
}

pub fn get_client_focused_colors(config: &str) -> Option<FocusedColors> {
    let var_pattern = r"set\s+\$([a-zA-Z_][a-zA-Z0-9_-]*)\s+(\S+)";
    let var_regex = Regex::new(var_pattern).ok()?;

    let mut variables: HashMap<String, String> = HashMap::new();

    for caps in var_regex.captures_iter(config) {
        if let (Some(var_name), Some(var_value)) = (caps.get(1), caps.get(2)) {
            variables.insert(
                var_name.as_str().to_string(),
                var_value.as_str().to_string(),
            );
        }
    }

    let client_pattern = r"client\.focused\s+(\S+)\s+(\S+)\s+(\S+)\s+(\S+)";
    let client_regex = Regex::new(client_pattern).ok()?;

    let caps = client_regex.captures(config)?;

    let border = resolve_color(caps.get(1)?.as_str(), &variables)?;
    let background = resolve_color(caps.get(2)?.as_str(), &variables)?;
    let text = resolve_color(caps.get(3)?.as_str(), &variables)?;
    let indicator = resolve_color(caps.get(4)?.as_str(), &variables)?;

    Some(FocusedColors {
        border,
        background,
        text,
        indicator,
    })
}

fn resolve_color(color_ref: &str, variables: &HashMap<String, String>) -> Option<String> {
    if color_ref.starts_with('$') {
        let var_name = &color_ref[1..];
        variables.get(var_name).cloned()
    } else if color_ref.starts_with('#') && (color_ref.len() == 7 || color_ref.len() == 9) {
        Some(color_ref.to_string())
    } else {
        None
    }
}

fn cleanup(ipc: &mut Connection, focused_colors: Option<&FocusedColors>) {
    if let Some(colors) = focused_colors {
        _ = ipc.run_command(format!(
            "client.focused {} {} {} {}",
            colors.border, colors.background, colors.text, colors.indicator
        ));
    }
}
