use lazy_static::lazy_static;
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

lazy_static! {
    static ref VAR_REGEX: Regex = Regex::new(r"set\s+\$([a-zA-Z_][a-zA-Z0-9_-]*)\s+(\S+)").unwrap();
    static ref CLIENT_REGEX: Regex =
        Regex::new(r"client\.focused\s+(\S+)\s+(\S+)\s+(\S+)\s+(\S+)").unwrap();
    static ref OUTER_GAP_REGEX: Regex = Regex::new(r"gaps\s+outer\s+(\d+)").unwrap();
}

fn main() -> Fallible<()> {
    let ipc = Connection::new()?;
    let ipc = Arc::new(Mutex::new(Some(ipc)));

    let sway_config = fetch_sway_config(&ipc).expect("Sway config file not found");
    let sway_config_variables =
        SwayConfigVariables::from_config(&sway_config).unwrap_or(SwayConfigVariables::new_empty());

    let user_outer_gap = OuterGap::get_user_outer_gap(&sway_config).unwrap_or(OuterGap(0));

    let client_focused_colors =
        get_client_focused_colors(&sway_config, &sway_config_variables).unwrap_or_default();

    let mut sys = System::new();

    let running = Arc::new(AtomicBool::new(true));

    {
        let running = running.clone();

        let mut signals = Signals::new([SIGINT, SIGTERM])?;
        thread::spawn(move || {
            if let Some(_sig) = signals.forever().next() {
                running.store(false, Ordering::SeqCst);
            }
        });
    }

    while running.load(Ordering::SeqCst) {
        sys.refresh_cpu_usage();
        let cpus = sys.cpus();
        let avg_cpu_usage: f32 =
            cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32;

        let (animation_speed, frequency) = swaytiness_calculator(avg_cpu_usage);

        if let Ok(mut guard) = ipc.lock()
            && let Some(conn) = guard.as_mut()
        {
            border_coloring(conn, avg_cpu_usage, &client_focused_colors)?;

            if animation_speed > 0 {
                window_breathing(conn, &user_outer_gap, animation_speed)?;
            }
        }

        std::thread::sleep(time::Duration::from_millis(frequency));
    }

    if let Ok(mut guard) = ipc.lock()
        && let Some(mut conn) = guard.take()
    {
        cleanup(&mut conn, &client_focused_colors, &user_outer_gap)?;
    }

    Ok(())
}

fn window_breathing(ipc: &mut Connection, outer_gap: &OuterGap, speed: u64) -> Fallible<()> {
    let base_thickness = 5;
    let mut border_thickness = base_thickness;
    let mut current_gap = outer_gap.0.max(6);

    let max_thickness = 11;
    let min_gap = 0u16;

    for breath_cycle in 0..2 {
        let expanding = breath_cycle == 0;

        for _step in 0..6 {
            if expanding {
                border_thickness = (border_thickness + 1).min(max_thickness);
                current_gap = current_gap.saturating_sub(1).max(min_gap);
                ipc.run_command(format!("gaps outer current set {}", current_gap))?;
            } else {
                border_thickness = (border_thickness - 1).max(base_thickness);
                current_gap = current_gap.saturating_add(1);
                ipc.run_command(format!("gaps outer current set {}", current_gap))?;
            }

            ipc.run_command(format!("border pixel {}", border_thickness))?;
            thread::sleep(time::Duration::from_millis(speed));
        }
    }

    Ok(())
}

fn swaytiness_calculator(cpu_usage: f32) -> (u64, u64) {
    if cpu_usage < 50.0 {
        (0, 2000)
    } else {
        let breathing_speed = 150 - cpu_usage as u64;
        let loop_frequency = 3900 - (38.0 * cpu_usage) as u64;
        (breathing_speed, loop_frequency)
    }
}

#[derive(Debug, Clone)]
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

#[derive(Debug)]
enum SwayColorError {
    Format,
    Length,
    HexDigits,
}

impl TryFrom<&str> for SwayColor {
    type Error = SwayColorError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if !value.starts_with('#') {
            return Err(SwayColorError::Format);
        }

        if value.len() != 7 && value.len() != 9 {
            return Err(SwayColorError::Length);
        }

        if !value[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(SwayColorError::HexDigits);
        }

        Ok(SwayColor(value.to_string()))
    }
}

fn border_coloring(
    ipc: &mut Connection,
    cpu_usage: f32,
    focused_colors: &FocusedColors,
) -> Fallible<()> {
    let border_color = SwayColor::from_cpu_usage(cpu_usage);

    ipc.run_command(format!(
        "client.focused {}99 {}99 {} {}",
        border_color.0, border_color.0, focused_colors.text.0, focused_colors.indicator.0
    ))?;

    Ok(())
}

fn fetch_sway_config(ipc: &Arc<Mutex<Option<Connection>>>) -> Option<swayipc::Config> {
    let mut guard = ipc.lock().ok()?;
    let conn = guard.as_mut()?;
    conn.get_config().ok()
}

struct SwayConfigVariables(HashMap<String, String>);

impl SwayConfigVariables {
    fn from_config(config: &swayipc::Config) -> Option<Self> {
        let var_regex = &VAR_REGEX;

        let mut variables: HashMap<String, String> = HashMap::new();

        for caps in var_regex.captures_iter(&config.config) {
            if let (Some(var_name), Some(var_value)) = (caps.get(1), caps.get(2)) {
                variables.insert(
                    var_name.as_str().to_string(),
                    var_value.as_str().to_string(),
                );
            }
        }

        Some(SwayConfigVariables(variables))
    }

    fn new_empty() -> Self {
        SwayConfigVariables(HashMap::new())
    }
}

#[derive(Debug, Clone)]
struct FocusedColors {
    border: SwayColor,
    background: SwayColor,
    text: SwayColor,
    indicator: SwayColor,
}

impl Default for FocusedColors {
    fn default() -> Self {
        FocusedColors {
            border: "#4c7899".try_into().unwrap(),
            background: "#285577".try_into().unwrap(),
            text: "#ffffff".try_into().unwrap(),
            indicator: "#2e9ef4".try_into().unwrap(),
        }
    }
}

fn get_client_focused_colors(
    config: &swayipc::Config,
    variables: &SwayConfigVariables,
) -> Option<FocusedColors> {
    let client_regex = &CLIENT_REGEX;

    let caps = client_regex.captures(&config.config)?;

    let border = resolve_color(caps.get(1)?.as_str(), variables)?;
    let background = resolve_color(caps.get(2)?.as_str(), variables)?;
    let text = resolve_color(caps.get(3)?.as_str(), variables)?;
    let indicator = resolve_color(caps.get(4)?.as_str(), variables)?;

    Some(FocusedColors {
        border,
        background,
        text,
        indicator,
    })
}

fn resolve_color(color_ref: &str, variables: &SwayConfigVariables) -> Option<SwayColor> {
    if let Some(var_name) = color_ref.strip_prefix('$') {
        variables.0.get(var_name)?.as_str().try_into().ok()
    } else if color_ref.starts_with('#') {
        color_ref.try_into().ok()
    } else {
        None
    }
}

#[derive(Debug)]
struct OuterGap(u16);

impl OuterGap {
    fn get_user_outer_gap(config: &swayipc::Config) -> Option<OuterGap> {
        let outer_gap_regex = &OUTER_GAP_REGEX;

        let caps = outer_gap_regex.captures(&config.config)?;

        let gap_value = caps.get(1)?.as_str().parse::<u16>().ok()?;

        Some(OuterGap(gap_value))
    }
}

fn cleanup(
    ipc: &mut Connection,
    focused_colors: &FocusedColors,
    outer_gap: &OuterGap,
) -> Fallible<()> {
    ipc.run_command(format!(
        "client.focused {} {} {} {}",
        focused_colors.border.0,
        focused_colors.background.0,
        focused_colors.text.0,
        focused_colors.indicator.0,
    ))?;

    ipc.run_command(format!("gaps outer all set {}", outer_gap.0))?;

    Ok(())
}
