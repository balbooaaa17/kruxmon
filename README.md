# KruxMon

A lightweight, read-only Linux CPU, process, disk and thermal monitoring TUI. It uses a near-black background with muted borders, warm amber accents, green healthy states, and red warnings.

## Current features
- Overall CPU utilization and per-core usage (computed from `/proc/stat` deltas)
- CPU frequency, scaling governor, and boost state when exported by the kernel
- First available non-NVMe hwmon temperature and first NVMe hwmon temperature, when exposed
- RAM, swap, load average, uptime, and battery status
- Failed systemd units
- 60-second CPU sparkline
- One-second refresh, `r` to refresh, `q`/Esc to quit
- No root privileges, daemon, NVIDIA dependency, or configuration required

## Build on EndeavourOS / Arch Linux

Install the Rust toolchain if needed:

```bash
sudo pacman -S --needed rust cargo
```

From the extracted project directory:

```bash
cargo run --release
```

Install the compiled binary for your user:

```bash
install -Dm755 target/release/kruxmon ~/.local/bin/kruxmon
```

Make sure `~/.local/bin` is on your `PATH`, then run `kruxmon`.

## Notes / limitations
- This is an initial implementation. Sensor names and availability vary by kernel and hardware; missing readings show `N/A`.
- The first available hwmon temperature is not guaranteed to be the CPU package temperature on every system.
- Fan RPM and CPU power in watts are not yet implemented because their sysfs exposure depends on hardware and kernel drivers.
- The monitor does not change CPU governor, boost, fan, power, or systemd state.
- `systemctl --failed` output is read-only. The monitor does not use `sudo`.
- The UI uses a fixed near-black background. For best appearance, use a true-color terminal.


## Dashboard features

- Top 10 processes by CPU activity, with PID, CPU percentage and resident RAM.
- Filesystem usage from `df` (mount point, percent used, used/free GiB).
- CPU thermal sensors and other readable hwmon sensors, including labels where Linux exposes them.
- NVMe temperature where exposed by the kernel, per-core CPU load, CPU history, RAM/swap, battery, frequency/governor/boost and failed systemd units.
- Read-only; does not need root. Refreshes about once per second.

## Downloads

Download the latest build from the
[GitHub Releases](../../releases) page.

- **Source ZIP:** source code and build instructions.
- **AppImage:** portable x86_64 Linux build.

### Run the AppImage

```bash
chmod +x KruxMon-x86_64.AppImage
./KruxMon-x86_64.AppImage
```

KruxMon is a terminal application, so run it from a terminal emulator.

### Build from source

Install Rust and Cargo, then run:

```bash
cargo build --release
./target/release/kruxmon
```

