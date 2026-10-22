use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table, Wrap},
    Terminal,
};
use std::{
    collections::HashMap,
    fs,
    io::{self, stdout},
    process::Command,
    time::{Duration, Instant},
};

const BG: Color = Color::Rgb(5, 7, 6);
const FG: Color = Color::Rgb(220, 222, 210);
const MUTED: Color = Color::Rgb(126, 132, 120);
const ACCENT: Color = Color::Rgb(219, 185, 92);
const GREEN: Color = Color::Rgb(150, 190, 125);
const RED: Color = Color::Rgb(232, 91, 128);
const BLUE: Color = Color::Rgb(125, 175, 190);
const PANEL: Color = Color::Rgb(12, 15, 12);

#[derive(Default, Clone)]
struct CpuTimes {
    total: u64,
    idle: u64,
}
#[derive(Default, Clone)]
struct Core {
    name: String,
    usage: u16,
}
#[derive(Default, Clone)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu: f64,
    mem_mb: f64,
}
#[derive(Default, Clone)]
struct Thermal {
    label: String,
    temp: f64,
    device: String,
}
#[derive(Default, Clone)]
struct Disk {
    mount: String,
    used: String,
    avail: String,
    pct: u16,
}
#[derive(Default)]
struct Snapshot {
    cpu: u16,
    cores: Vec<Core>,
    thermals: Vec<Thermal>,
    freq_mhz: Option<u64>,
    governor: String,
    boost: String,
    mem_total: u64,
    mem_available: u64,
    swap_total: u64,
    swap_free: u64,
    load: String,
    uptime: String,
    failed_units: Vec<String>,
    battery: Option<String>,
    disks: Vec<Disk>,
    processes: Vec<ProcessInfo>,
}
struct App {
    previous_cpu: Vec<CpuTimes>,
    previous_processes: HashMap<u32, u64>,
    cpu_history: Vec<u64>,
    snap: Snapshot,
    last_sample: Instant,
}
impl App {
    fn new() -> Self {
        let mut a = Self {
            previous_cpu: Vec::new(),
            previous_processes: HashMap::new(),
            cpu_history: vec![0; 60],
            snap: Snapshot::default(),
            last_sample: Instant::now(),
        };
        a.sample();
        a
    }
    fn sample(&mut self) {
        let current = read_cpu_times();
        let total_delta = current
            .first()
            .zip(self.previous_cpu.first())
            .map(|(n, o)| n.total.saturating_sub(o.total))
            .unwrap_or(0);
        if !self.previous_cpu.is_empty() && current.len() == self.previous_cpu.len() {
            let mut usages = Vec::new();
            for (i, (now, old)) in current.iter().zip(self.previous_cpu.iter()).enumerate() {
                let dt = now.total.saturating_sub(old.total);
                let di = now.idle.saturating_sub(old.idle);
                let pct = if dt == 0 {
                    0
                } else {
                    ((dt.saturating_sub(di)) * 100 / dt).min(100) as u16
                };
                if i == 0 {
                    self.snap.cpu = pct;
                } else {
                    usages.push(Core {
                        name: format!("CPU{}", i - 1),
                        usage: pct,
                    });
                }
            }
            self.snap.cores = usages;
        }
        self.previous_cpu = current;
        self.cpu_history.push(self.snap.cpu as u64);
        if self.cpu_history.len() > 60 {
            self.cpu_history.remove(0);
        }

        self.snap.thermals = read_thermals();
        self.snap.freq_mhz =
            read_number("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq").map(|v| v / 1000);
        self.snap.governor = read_text("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
            .unwrap_or_else(|| "n/a".into());
        self.snap.boost = read_text("/sys/devices/system/cpu/cpufreq/boost")
            .map(|v| if v.trim() == "1" { "on" } else { "off" }.into())
            .unwrap_or_else(|| "n/a".into());
        self.read_memory();
        self.snap.load = read_text("/proc/loadavg")
            .unwrap_or_default()
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join("  ");
        self.snap.uptime = read_text("/proc/uptime")
            .and_then(|s| {
                s.split_whitespace()
                    .next()
                    .and_then(|v| v.parse::<f64>().ok())
            })
            .map(format_duration)
            .unwrap_or_else(|| "n/a".into());
        self.snap.failed_units = failed_units();
        self.snap.battery = battery_status();
        self.snap.disks = read_disks();
        self.snap.processes = read_processes(&self.previous_processes, total_delta);
        self.previous_processes = read_process_ticks();
        self.last_sample = Instant::now();
    }
    fn read_memory(&mut self) {
        if let Some(s) = read_text("/proc/meminfo") {
            for line in s.lines() {
                let mut p = line.split_whitespace();
                let key = p.next().unwrap_or("");
                let val = p.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                match key {
                    "MemTotal:" => self.snap.mem_total = val,
                    "MemAvailable:" => self.snap.mem_available = val,
                    "SwapTotal:" => self.snap.swap_total = val,
                    "SwapFree:" => self.snap.swap_free = val,
                    _ => {}
                }
            }
        }
    }
}
fn read_cpu_times() -> Vec<CpuTimes> {
    read_text("/proc/stat")
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            l.starts_with("cpu")
                && l.as_bytes()
                    .get(3)
                    .map(|b| b.is_ascii_digit() || *b == b' ')
                    .unwrap_or(false)
        })
        .filter_map(|line| {
            let mut p = line.split_whitespace();
            let name = p.next()?;
            if name != "cpu" && !name.starts_with("cpu") {
                return None;
            }
            let vals: Vec<u64> = p.filter_map(|x| x.parse().ok()).collect();
            if vals.len() < 4 {
                return None;
            }
            let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
            Some(CpuTimes {
                total: vals.iter().sum(),
                idle,
            })
        })
        .collect()
}
fn read_text(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}
fn read_number(path: &str) -> Option<u64> {
    read_text(path)?.parse().ok()
}

fn read_thermals() -> Vec<Thermal> {
    let mut out = Vec::new();
    let entries = match fs::read_dir("/sys/class/hwmon") {
        Ok(v) => v,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let device =
            read_text(path.join("name").to_str().unwrap_or("")).unwrap_or_else(|| "sensor".into());
        if let Ok(files) = fs::read_dir(&path) {
            for file in files.flatten() {
                let p = file.path();
                let n = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if !n.starts_with("temp") || !n.ends_with("_input") {
                    continue;
                }
                let raw =
                    match read_text(p.to_str().unwrap_or("")).and_then(|v| v.parse::<f64>().ok()) {
                        Some(v) => v,
                        None => continue,
                    };
                let temp = raw / 1000.0;
                if !(-20.0..150.0).contains(&temp) {
                    continue;
                }
                let stem = n.trim_end_matches("_input");
                let label = read_text(path.join(format!("{stem}_label")).to_str().unwrap_or(""))
                    .unwrap_or_else(|| stem.to_string());
                out.push(Thermal {
                    label,
                    temp,
                    device: device.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        let a_cpu = is_cpu_sensor(&a.device, &a.label);
        let b_cpu = is_cpu_sensor(&b.device, &b.label);
        b_cpu
            .cmp(&a_cpu)
            .then_with(|| a.device.cmp(&b.device))
            .then_with(|| a.label.cmp(&b.label))
    });
    out.dedup_by(|a, b| {
        a.device == b.device && a.label == b.label && (a.temp - b.temp).abs() < 0.1
    });
    out
}
fn is_cpu_sensor(device: &str, label: &str) -> bool {
    let s = format!("{device} {label}").to_lowercase();
    [
        "k10temp", "coretemp", "cpu", "tctl", "tdie", "package", "zenpower",
    ]
    .iter()
    .any(|x| s.contains(x))
}
fn read_process_ticks() -> HashMap<u32, u64> {
    let mut out = HashMap::new();
    let entries = match fs::read_dir("/proc") {
        Ok(v) => v,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let pid = match name.parse::<u32>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(t) = process_ticks(&e.path().join("stat")) {
            out.insert(pid, t);
        }
    }
    out
}
fn process_ticks(path: &std::path::Path) -> Option<u64> {
    let s = fs::read_to_string(path).ok()?;
    let close = s.rfind(')')?;
    let fields: Vec<&str> = s.get(close + 1..)?.split_whitespace().collect();
    // After comm: state is field 3; utime/stime are fields 14/15.
    Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
}
fn read_processes(previous: &HashMap<u32, u64>, total_delta: u64) -> Vec<ProcessInfo> {
    let ticks_now = read_process_ticks();
    let page_kb = 4.0; // Linux base pages are normally 4 KiB; RSS is read from statm.
    let mut out = Vec::new();
    let entries = match fs::read_dir("/proc") {
        Ok(v) => v,
        Err(_) => return out,
    };
    let cpus = read_cpu_times().len().saturating_sub(1).max(1) as f64;
    for e in entries.flatten() {
        let pid = match e.file_name().to_string_lossy().parse::<u32>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let base = e.path();
        let name =
            read_text(base.join("comm").to_str().unwrap_or("")).unwrap_or_else(|| "?".into());
        let ticks = ticks_now.get(&pid).copied().unwrap_or(0);
        let old = previous.get(&pid).copied().unwrap_or(ticks);
        let cpu = if total_delta > 0 {
            ticks.saturating_sub(old) as f64 / total_delta as f64 * 100.0 * cpus
        } else {
            0.0
        };
        let rss_pages = read_text(base.join("statm").to_str().unwrap_or(""))
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
            .unwrap_or(0);
        out.push(ProcessInfo {
            pid,
            name: name.chars().take(22).collect(),
            cpu,
            mem_mb: rss_pages as f64 * page_kb / 1024.0,
        });
    }
    out.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(10);
    out
}
fn read_disks() -> Vec<Disk> {
    let out = Command::new("df")
        .args(["-P", "-k", "-x", "tmpfs", "-x", "devtmpfs"])
        .output();
    let text = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return vec![],
    };
    let mut disks = Vec::new();
    for line in text.lines().skip(1) {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() < 6 {
            continue;
        }
        let pct = p[4].trim_end_matches('%').parse::<u16>().unwrap_or(0);
        let avail = p[3].parse::<f64>().unwrap_or(0.0) / 1024.0 / 1024.0;
        let used = p[2].parse::<f64>().unwrap_or(0.0) / 1024.0 / 1024.0;
        let mount = p[5].to_string();
        if !mount.starts_with('/') {
            continue;
        }
        disks.push(Disk {
            mount,
            used: format!("{used:.1}G"),
            avail: format!("{avail:.1}G"),
            pct,
        });
    }
    disks.sort_by(|a, b| b.pct.cmp(&a.pct));
    disks.truncate(8);
    disks
}
fn failed_units() -> Vec<String> {
    let out = Command::new("systemctl")
        .args(["--failed", "--no-legend", "--plain"])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| {
                let s = l.split_whitespace().next()?.to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            })
            .collect(),
        _ => vec![],
    }
}
fn battery_status() -> Option<String> {
    for e in fs::read_dir("/sys/class/power_supply").ok()? {
        let p = e.ok()?.path();
        let typ = fs::read_to_string(p.join("type")).unwrap_or_default();
        if typ.trim() != "Battery" {
            continue;
        }
        let status = fs::read_to_string(p.join("status"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let cap = fs::read_to_string(p.join("capacity"))
            .ok()
            .and_then(|v| v.trim().parse::<u8>().ok());
        return Some(match cap {
            Some(c) => format!("{} · {}%", status, c),
            None => status,
        });
    }
    None
}
fn format_duration(s: f64) -> String {
    let n = s as u64;
    let d = n / 86400;
    let h = (n % 86400) / 3600;
    let m = (n % 3600) / 60;
    if d > 0 {
        format!("{}d {}h {}m", d, h, m)
    } else {
        format!("{}h {}m", h, m)
    }
}
fn box_block(title: &'static str) -> Block<'static> {
    Block::default()
        .title(Span::styled(
            format!(" :: {title} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(54, 61, 50)))
        .style(Style::default().bg(PANEL).fg(FG))
}
fn draw(f: &mut ratatui::Frame, app: &App) {
    f.render_widget(
        Block::default().style(Style::default().bg(BG).fg(FG)),
        f.area(),
    );
    let area = f.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(6),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " KRUXMON ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("· Linux system cockpit · ", Style::default().fg(FG)),
        Span::styled("LIVE", Style::default().fg(GREEN)),
        Span::styled(
            format!("   load {}   up {}", app.snap.load, app.snap.uptime),
            Style::default().fg(MUTED),
        ),
    ]))
    .style(Style::default().bg(BG));
    f.render_widget(header, rows[0]);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(rows[1]);
    let cpu_color = if app.snap.cpu >= 90 {
        RED
    } else if app.snap.cpu >= 70 {
        ACCENT
    } else {
        GREEN
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(" CPU  {:>3}%", app.snap.cpu),
                Style::default().fg(cpu_color).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(
                    " {} MHz",
                    app.snap
                        .freq_mhz
                        .map(|v| v.to_string())
                        .unwrap_or("n/a".into())
                ),
                Style::default().fg(MUTED),
            )),
            Line::from(Span::styled(
                format!(" {} cores", app.snap.cores.len()),
                Style::default().fg(MUTED),
            )),
        ])
        .block(box_block("PROCESSOR")),
        top[0],
    );
    let hottest = app
        .snap
        .thermals
        .iter()
        .filter(|t| is_cpu_sensor(&t.device, &t.label))
        .map(|t| t.temp)
        .fold(None, |acc: Option<f64>, v| {
            Some(acc.map_or(v, |a| a.max(v)))
        });
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(
                    " CPU  {}",
                    hottest.map(|v| format!("{v:.1}°C")).unwrap_or("n/a".into())
                ),
                Style::default()
                    .fg(hottest.map(temp_color).unwrap_or(MUTED))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(
                    " NVMe {}",
                    app.snap
                        .thermals
                        .iter()
                        .find(|t| t.device.to_lowercase().contains("nvme"))
                        .map(|t| format!("{:.1}°C", t.temp))
                        .unwrap_or("n/a".into())
                ),
                Style::default().fg(MUTED),
            )),
            Line::from(Span::styled(
                format!(" {} sensors", app.snap.thermals.len()),
                Style::default().fg(MUTED),
            )),
        ])
        .block(box_block("THERMALS")),
        top[1],
    );
    let mem_used = app.snap.mem_total.saturating_sub(app.snap.mem_available);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(
                    " RAM  {:.1}/{:.1} GiB",
                    mem_used as f64 / 1048576.0,
                    app.snap.mem_total as f64 / 1048576.0
                ),
                Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(
                    " Swap {:.1}/{:.1} GiB",
                    app.snap.swap_total.saturating_sub(app.snap.swap_free) as f64 / 1048576.0,
                    app.snap.swap_total as f64 / 1048576.0
                ),
                Style::default().fg(MUTED),
            )),
            Line::from(Span::styled(
                format!(" Battery {}", app.snap.battery.as_deref().unwrap_or("n/a")),
                Style::default().fg(MUTED),
            )),
        ])
        .block(box_block("MEMORY")),
        top[2],
    );
    let root_disk = app.snap.disks.iter().find(|d| d.mount == "/");
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!(
                    " Root {}",
                    root_disk
                        .map(|d| format!("{} free · {} used", d.avail, d.used))
                        .unwrap_or("n/a".into())
                ),
                Style::default()
                    .fg(if root_disk.map(|d| d.pct >= 90).unwrap_or(false) {
                        RED
                    } else {
                        GREEN
                    })
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(" Governor {}", app.snap.governor),
                Style::default().fg(MUTED),
            )),
            Line::from(Span::styled(
                format!(" Boost {}", app.snap.boost),
                Style::default().fg(MUTED),
            )),
        ])
        .block(box_block("STORAGE & POLICY")),
        top[3],
    );

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(57), Constraint::Percentage(43)])
        .split(rows[2]);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(7)])
        .split(middle[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Length(6),
            Constraint::Length(5),
        ])
        .split(middle[1]);

    let mut core_lines = Vec::new();
    for chunk in app.snap.cores.chunks(4) {
        core_lines.push(Line::from(
            chunk
                .iter()
                .flat_map(|c| {
                    let color = if c.usage >= 90 {
                        RED
                    } else if c.usage >= 70 {
                        ACCENT
                    } else {
                        GREEN
                    };
                    [
                        Span::styled(format!("{:<4}", c.name), Style::default().fg(MUTED)),
                        Span::styled(format!("{:>3}%  ", c.usage), Style::default().fg(color)),
                    ]
                })
                .collect::<Vec<_>>(),
        ));
    }
    if core_lines.is_empty() {
        core_lines.push(Line::from(Span::styled(
            "Sampling CPU counters…",
            Style::default().fg(MUTED),
        )));
    }
    f.render_widget(
        Paragraph::new(core_lines).block(box_block("PER-CORE LOAD")),
        left[0],
    );
    let process_rows = app.snap.processes.iter().map(|p| {
        Row::new(vec![
            p.pid.to_string(),
            p.name.clone(),
            format!("{:>4.1}%", p.cpu),
            format!("{:>5.0}M", p.mem_mb),
        ])
        .style(Style::default().fg(if p.cpu >= 50.0 { RED } else { FG }))
    });
    let proc_table = Table::new(
        process_rows,
        [
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(7),
            Constraint::Length(7),
        ],
    )
    .header(
        Row::new(vec!["PID", "PROCESS", "CPU", "RAM"])
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
    )
    .column_spacing(1)
    .block(box_block("TOP PROCESSES · CPU"));
    f.render_widget(proc_table, left[1]);

    let disk_rows = app.snap.disks.iter().map(|d| {
        let c = if d.pct >= 90 {
            RED
        } else if d.pct >= 75 {
            ACCENT
        } else {
            GREEN
        };
        Row::new(vec![
            d.mount.clone(),
            format!("{}%", d.pct),
            d.used.clone(),
            d.avail.clone(),
        ])
        .style(Style::default().fg(c))
    });
    let disk_table = Table::new(
        disk_rows,
        [
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(6),
        ],
    )
    .header(
        Row::new(vec!["MOUNT", "USE", "USED", "FREE"])
            .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
    )
    .column_spacing(1)
    .block(box_block("FILESYSTEMS"));
    f.render_widget(disk_table, right[0]);

    let thermal_lines: Vec<Line> = if app.snap.thermals.is_empty() {
        vec![Line::from(Span::styled(
            "No readable hwmon sensors",
            Style::default().fg(MUTED),
        ))]
    } else {
        app.snap
            .thermals
            .iter()
            .take(4)
            .map(|t| {
                Line::from(vec![
                    Span::styled(
                        format!(
                            "{:<10}",
                            format!("{} {}", t.device, t.label)
                                .chars()
                                .take(10)
                                .collect::<String>()
                        ),
                        Style::default().fg(MUTED),
                    ),
                    Span::styled(
                        format!("{:>5.1} °C", t.temp),
                        Style::default()
                            .fg(temp_color(t.temp))
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            })
            .collect()
    };
    f.render_widget(
        Paragraph::new(thermal_lines).block(box_block("THERMAL SENSORS")),
        right[1],
    );

    let mut health = vec![Line::from(vec![
        Span::styled("Failed units ", Style::default().fg(MUTED)),
        Span::styled(
            format!("{}", app.snap.failed_units.len()),
            Style::default()
                .fg(if app.snap.failed_units.is_empty() {
                    GREEN
                } else {
                    RED
                })
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    if app.snap.failed_units.is_empty() {
        health.push(Line::from(Span::styled(
            "Systemd OK",
            Style::default().fg(GREEN),
        )));
    } else {
        for unit in app.snap.failed_units.iter().take(2) {
            health.push(Line::from(Span::styled(
                format!("! {unit}"),
                Style::default().fg(RED),
            )));
        }
    }
    f.render_widget(
        Paragraph::new(health)
            .wrap(Wrap { trim: true })
            .block(box_block("SYSTEM HEALTH")),
        right[2],
    );

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            " q ",
            Style::default()
                .fg(BG)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" quit  ", Style::default().fg(MUTED)),
        Span::styled(
            "r ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" refresh  ", Style::default().fg(MUTED)),
        Span::styled(
            "1s refresh · read-only · no root",
            Style::default().fg(MUTED),
        ),
    ]))
    .style(Style::default().bg(BG));
    f.render_widget(footer, rows[3]);
}
fn temp_color(v: f64) -> Color {
    if v >= 90.0 {
        RED
    } else if v >= 75.0 {
        ACCENT
    } else {
        GREEN
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let result = run_app(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    if let Err(e) = result {
        eprintln!("kruxmon: {e}");
    }
    Ok(())
}
fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut app = App::new();
    loop {
        terminal.draw(|f| draw(f, &app))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('r') => app.sample(),
                        _ => {}
                    }
                }
            }
        }
        if app.last_sample.elapsed() >= Duration::from_secs(1) {
            app.sample();
        }
    }
}
