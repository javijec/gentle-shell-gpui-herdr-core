//! The machine's power state, so animated theme backdrops can do less when
//! it matters: on battery, and in Low Power Mode or battery saver.

/// How the machine is powered now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Power {
    /// Plugged in, or unknown.
    #[default]
    Plugged,
    /// Running on its battery.
    Battery,
    /// The person asked to save power: Low Power Mode (macOS), battery saver
    /// (Windows) or the power-saver profile (Linux).
    Saving,
}

/// The power state now. Cheap enough to poll every few seconds.
pub fn current() -> Power {
    imp::current()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Power;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        /// Seconds of battery left, -2 (`kIOPSTimeRemainingUnlimited`) on
        /// external power, -1 while it's still estimating.
        fn IOPSGetTimeRemainingEstimate() -> f64;
    }

    pub fn current() -> Power {
        if objc2_foundation::NSProcessInfo::processInfo().isLowPowerModeEnabled() {
            return Power::Saving;
        }
        // SAFETY: a plain IOKit query with no arguments or ownership.
        let remaining = unsafe { IOPSGetTimeRemainingEstimate() };
        if remaining == -2. {
            Power::Plugged
        } else {
            Power::Battery
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::Power;
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    pub fn current() -> Power {
        let mut status = SYSTEM_POWER_STATUS::default();
        // SAFETY: `status` is a valid, writable SYSTEM_POWER_STATUS.
        if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
            return Power::Plugged;
        }
        match (status.SystemStatusFlag, status.ACLineStatus) {
            (1, _) => Power::Saving,
            (_, 0) => Power::Battery,
            _ => Power::Plugged,
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::Power;

    /// The firmware's power-saver profile (what GNOME and KDE's power
    /// profiles set) means saving power; a battery that's discharging means
    /// no mains power.
    pub fn current() -> Power {
        let profile = std::fs::read_to_string("/sys/firmware/acpi/platform_profile");
        if profile.is_ok_and(|p| p.trim() == "low-power") {
            return Power::Saving;
        }
        let Ok(supplies) = std::fs::read_dir("/sys/class/power_supply") else {
            return Power::Plugged;
        };
        let read = |dir: &std::path::Path, file: &str| {
            std::fs::read_to_string(dir.join(file)).map(|s| s.trim().to_string())
        };
        let discharging = supplies.flatten().any(|supply| {
            let dir = supply.path();
            read(&dir, "type").is_ok_and(|t| t == "Battery")
                && read(&dir, "status").is_ok_and(|s| s == "Discharging")
        });
        if discharging {
            Power::Battery
        } else {
            Power::Plugged
        }
    }
}
