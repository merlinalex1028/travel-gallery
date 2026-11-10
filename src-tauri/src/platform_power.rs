/// Read-only power-source inspection. Unknown status never implies battery power.
#[cfg(target_os = "macos")]
pub fn on_battery() -> Option<bool> {
    let mut command = std::process::Command::new("/usr/bin/pmset");
    command.args(["-g", "batt"]).env("LC_ALL", "C");
    let output = crate::media::run(command, 2).ok()?;
    if !output.status.success() {
        return None;
    }
    parse_pmset(&String::from_utf8_lossy(&output.stdout))
}
#[cfg(windows)]
pub fn on_battery() -> Option<bool> {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    // The API writes exactly one initialized SYSTEM_POWER_STATUS structure.
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return None;
    }
    match status.ACLineStatus {
        0 if status.BatteryFlag != 255 && status.BatteryFlag & 128 == 0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn on_battery() -> Option<bool> {
    None
}

#[cfg(any(target_os = "macos", test))]
fn parse_pmset(text: &str) -> Option<bool> {
    match text.lines().next()?.trim() {
        "Now drawing from 'Battery Power'" => Some(true),
        "Now drawing from 'AC Power'" => Some(false),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn unknown_or_failed_status_is_not_assumed_to_be_battery() {
        assert_eq!(
            super::parse_pmset(
                "Now drawing from 'Battery Power'\n -InternalBattery-0\t50%; discharging"
            ),
            Some(true)
        );
        assert_eq!(
            super::parse_pmset("Now drawing from 'AC Power'\n"),
            Some(false)
        );
        assert_eq!(super::parse_pmset(""), None);
        assert_eq!(super::parse_pmset("Permission denied"), None);
    }
}
