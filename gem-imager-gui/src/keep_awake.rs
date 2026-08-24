
#[derive(Debug)]
pub(crate) struct KeepAwake {
    active: bool,
}

impl KeepAwake {
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

#[cfg(windows)]
fn set_inhibit(on: bool) -> bool {
    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

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

    #[test]
    fn the_guard_is_reentrant_and_never_panics() {
        let first = KeepAwake::acquire();
        let second = KeepAwake::acquire();
        drop(second);
        drop(first);

        assert_eq!(KeepAwake::acquire().active, cfg!(windows));
    }
}
