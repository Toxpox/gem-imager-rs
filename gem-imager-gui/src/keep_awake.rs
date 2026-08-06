//! Keep the machine awake while an image is being written.
//!
//! A DFU write is a multi-stage USB conversation with a board that re-enumerates between stages.
//! If the host suspends in the middle of it, the board is left with a half-written eMMC and the
//! only evidence is a device that no longer boots. Idle suspend is the realistic case: the user
//! starts a write and walks away, which is exactly the situation the timer is designed for.
//!
//! The guard is held for the whole write and released by `Drop`, including on cancellation and on
//! failure. It never fails the flash: an inhibit that could not be taken is logged and the write
//! proceeds, because refusing to write at all is the worse outcome.

/// Guard that keeps the system awake for as long as it is alive.
#[derive(Debug)]
pub(crate) struct KeepAwake {
    /// Whether the platform actually took the inhibit, so `Drop` only undoes what was done.
    active: bool,
}

impl KeepAwake {
    /// Ask the platform not to sleep until this value is dropped.
    pub(crate) fn acquire() -> Self {
        Self {
            active: set_inhibit(true),
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        if self.active {
            set_inhibit(false);
        }
    }
}

/// Returns whether the request was honoured.
#[cfg(windows)]
fn set_inhibit(on: bool) -> bool {
    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    // `ES_SYSTEM_REQUIRED` without `ES_DISPLAY_REQUIRED`: the display may still blank, only the
    // machine must not suspend. `ES_CONTINUOUS` makes the state stick until it is cleared, rather
    // than resetting the idle timer once.
    let flags = if on {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };

    // SAFETY: the call takes a plain bitflag and touches no memory the caller owns.
    let previous = unsafe { SetThreadExecutionState(flags) };
    if previous == 0 {
        tracing::warn!(
            "Failed to {} sleep inhibition; the write will continue but the host may suspend",
            if on { "enable" } else { "clear" }
        );
        return false;
    }

    if on {
        tracing::info!("Sleep inhibition enabled for the duration of the write");
    }
    true
}

/// Non-Windows hosts: no inhibit is taken.
///
/// The Linux route is a desktop-portal / logind session inhibit, and which of those is reachable
/// depends on the package (deb, Flatpak, Snap) — that is Faz 9's decision. Claiming an inhibit here
/// that a sandbox may silently refuse would be worse than the current honest state: the log says
/// plainly that the host may suspend.
#[cfg(not(windows))]
fn set_inhibit(on: bool) -> bool {
    if on {
        tracing::warn!(
            "Sleep inhibition is not implemented on this platform; do not let the host suspend \
             while the write is running"
        );
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Acquiring and releasing must be safe to do repeatedly and must never panic, because the
    /// guard wraps every write on every platform.
    #[test]
    fn the_guard_is_reentrant_and_never_panics() {
        let first = KeepAwake::acquire();
        let second = KeepAwake::acquire();
        drop(second);
        drop(first);

        // On Windows the inhibit is expected to be taken; elsewhere it is deliberately absent.
        assert_eq!(KeepAwake::acquire().active, cfg!(windows));
    }
}
