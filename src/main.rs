use regex::Regex;
use signal_hook::{
    consts::{SIGINT, SIGTERM},
    iterator::Signals,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    thread, time,
};
use sysinfo::System;

use swayipc::{Connection, Fallible};

const BASE_BORDER_THICKNESS: u16 = 12;
const MIN_BORDER_THICKNESS: u16 = 5;
const BREATHING_STEPS: u16 = 7;
const BREATHING_CYCLES: u64 = 2;
const BASE_BREATHING_SPEED: u64 = 150;
const BASE_LOOP_FREQUENCY: u64 = 3900;
const CPU_TO_FREQUENCY_RATIO: f32 = 38.0;
const CPU_COLOR_SCALE: f32 = 255.0 / 50.0;

const DEFAULT_BORDER_COLOR: &str = "#4c7899";
const DEFAULT_BACKGROUND_COLOR: &str = "#285577";
const DEFAULT_TEXT_COLOR: &str = "#ffffff";
const DEFAULT_INDICATOR_COLOR: &str = "#2e9ef4";
const DEFAULT_CHILD_BORDER_COLOR: &str = "#285577";

static VAR_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"set\s+\$([a-zA-Z_][a-zA-Z0-9_-]*)\s+(\S+)").unwrap());
static CLIENT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)(?i)^\s*client\.focused[ \t]+(\S+)[ \t]+(\S+)[ \t]+(\S+)(?:[ \t]+(\S+))?(?:[ \t]+(\S+))?",
    )
    .unwrap()
});
static INNER_GAP_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"gaps\s+inner\s+(\d+)").unwrap());

fn main() -> Fallible<()> {
    let mut ipc = Connection::new()?;

    let sway_config = ipc.get_config()?;
    let sway_config_variables = SwayConfigVariables::from_config(&sway_config);

    let user_inner_gap = InnerGap::get_user_inner_gap(&sway_config).unwrap_or_else(|| {
        eprintln!("Warning: could not parse user's default inner gap, defaulting to 0");
        InnerGap(0)
    });

    let client_focused_colors =
        get_client_focused_colors(&sway_config, &sway_config_variables).unwrap_or_else(|| {
            eprintln!("Warning: could not parse user's default colors for focused windows, using sway's default values");
            FocusedColors::default()
        });

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

    let mut sys = System::new();
    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);

    while running.load(Ordering::SeqCst) {
        sys.refresh_cpu_usage();
        let cpus = sys.cpus();
        let avg_cpu_usage: f32 =
            cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32;

        let (animation_speed, frequency) = swaytiness_calculator(avg_cpu_usage);

        border_coloring(&mut ipc, avg_cpu_usage, &client_focused_colors)
            .inspect_err(|e| eprintln!("Error coloring border: {}", e))?;

        window_breathing(&mut ipc, &user_inner_gap, animation_speed)
            .inspect_err(|e| eprintln!("Error resizing border/gap: {}", e))?;

        std::thread::sleep(time::Duration::from_millis(frequency));
    }

    cleanup(&mut ipc, &client_focused_colors, &user_inner_gap)?;

    Ok(())
}

fn window_breathing(ipc: &mut Connection, inner_gap: &InnerGap, speed: u64) -> Fallible<()> {
    if speed == 0 {
        return Ok(());
    }

    let base_thickness = BASE_BORDER_THICKNESS;
    let mut border_thickness = base_thickness;
    let mut current_gap = inner_gap.0;

    let min_thickness = MIN_BORDER_THICKNESS;

    for breath_cycle in 0..BREATHING_CYCLES {
        let inhaling = breath_cycle == 0;

        for _step in 0..BREATHING_STEPS {
            if inhaling {
                border_thickness = (border_thickness - 1).max(min_thickness);
                current_gap = current_gap.saturating_add(1);
                ipc.run_command(format!("gaps inner current set {}", current_gap))?;
            } else {
                border_thickness = (border_thickness + 1).min(base_thickness);
                current_gap = current_gap.saturating_sub(1);
                ipc.run_command(format!("gaps inner current set {}", current_gap))?;
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
        let breathing_speed = BASE_BREATHING_SPEED
            .saturating_sub(cpu_usage.round() as u64)
            .max(50);
        let loop_frequency = BASE_LOOP_FREQUENCY
            .saturating_sub((CPU_TO_FREQUENCY_RATIO * cpu_usage).round().max(100.0) as u64);
        (breathing_speed, loop_frequency)
    }
}

#[derive(Debug, Clone)]
struct SwayColor(String);

impl SwayColor {
    fn from_cpu_usage(cpu_usage: f32) -> Self {
        let cpu_usage = cpu_usage.clamp(0.0, 100.0);
        if cpu_usage <= 50.0 {
            let green = 255;
            let red = (CPU_COLOR_SCALE * cpu_usage).round() as u8;
            SwayColor(format!("#{:02x}{:02x}00", red, green))
        } else {
            let green = (CPU_COLOR_SCALE * (100.0 - cpu_usage).round()) as u8;
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

    let focused_colors = FocusedColors {
        border: border_color.clone(),
        background: focused_colors.background.clone(),
        text: focused_colors.text.clone(),
        indicator: focused_colors.indicator.clone(),
        child_border: Some(border_color).clone(),
    };

    ipc.run_command(format!("client.focused {}", focused_colors))?;

    Ok(())
}

struct SwayConfigVariables(HashMap<String, String>);

impl SwayConfigVariables {
    fn from_config(config: &swayipc::Config) -> Self {
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

        SwayConfigVariables(variables)
    }
}

#[derive(Debug, Clone)]
struct FocusedColors {
    border: SwayColor,
    background: SwayColor,
    text: SwayColor,
    indicator: Option<SwayColor>,
    child_border: Option<SwayColor>,
}

impl Default for FocusedColors {
    fn default() -> Self {
        FocusedColors {
            border: DEFAULT_BORDER_COLOR.try_into().unwrap(),
            background: DEFAULT_BACKGROUND_COLOR.try_into().unwrap(),
            text: DEFAULT_TEXT_COLOR.try_into().unwrap(),
            indicator: DEFAULT_INDICATOR_COLOR.try_into().ok(),
            child_border: DEFAULT_CHILD_BORDER_COLOR.try_into().ok(),
        }
    }
}

impl std::fmt::Display for FocusedColors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let indicator = if let Some(indicator) = self.indicator.clone() {
            indicator.0
        } else {
            DEFAULT_INDICATOR_COLOR.to_string()
        };

        let child_border = if let Some(child_border) = self.child_border.clone() {
            child_border.0
        } else {
            self.border.0.clone()
        };

        write!(
            f,
            "{} {} {} {} {}",
            self.border.0, self.background.0, self.text.0, indicator, child_border
        )
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
    let indicator = if let Some(cap) = caps.get(4) {
        resolve_color(cap.as_str(), variables)
    } else {
        None
    };
    let child_border = if let Some(cap) = caps.get(5) {
        resolve_color(cap.as_str(), variables)
    } else {
        None
    };

    Some(FocusedColors {
        border,
        background,
        text,
        indicator,
        child_border,
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
struct InnerGap(u16);

impl InnerGap {
    fn get_user_inner_gap(config: &swayipc::Config) -> Option<InnerGap> {
        let inner_gap_regex = &INNER_GAP_REGEX;

        let caps = inner_gap_regex.captures(&config.config)?;

        let gap_value = caps.get(1)?.as_str().parse::<u16>().ok()?;

        Some(InnerGap(gap_value))
    }
}

fn cleanup(
    ipc: &mut Connection,
    focused_colors: &FocusedColors,
    inner_gap: &InnerGap,
) -> Fallible<()> {
    ipc.run_command(format!("client.focused {}", focused_colors))?;

    ipc.run_command(format!("gaps inner all set {}", inner_gap.0))?;

    Ok(())
}
