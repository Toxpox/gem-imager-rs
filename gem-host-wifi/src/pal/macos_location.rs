//! The CoreLocation authorization gate that macOS puts in front of the SSID.
//!
//! On macOS 14+ every identifying Wi-Fi field — `ssid`, `ssidData`, `bssid`, `countryCode` — reads
//! back `nil` until the process holds Location authorization, so this gate decides whether autofill
//! can work at all. Getting authorized is not a function call; it is a conversation with `locationd`
//! that only completes if four things are true at once:
//!
//! 1. **A delegate is set.** `CLLocationManager` reports its authorization *only* through
//!    `locationManagerDidChangeAuthorization:`. With no delegate the conversation never starts.
//! 2. **The status is read after the callback, not before.** `authorizationStatus` returns
//!    `notDetermined` immediately after `CLLocationManager::new()` even for an app that is already
//!    authorized; the real value is filled in asynchronously.
//! 3. **The manager outlives the request.** Authorization is asynchronous, so a manager that is
//!    dropped at the end of the calling function cancels its own request.
//! 4. **It runs on the main thread's run loop.** The callback is delivered to the run loop the
//!    manager was created on; a worker thread has none, so nothing is ever delivered.
//!
//! Hence the shape here: one process-wide manager, created once on the main queue, kept alive for
//! the life of the process, publishing every status change into a [`Condvar`] that the (blocking,
//! off-main-thread) detection path waits on.
//!
//! `startUpdatingLocation` is what actually makes macOS present the prompt — `requestWhenInUse
//! Authorization` alone leaves the status at `notDetermined` indefinitely. Updates are stopped again
//! the moment a decision arrives: this code wants the authorization, never the user's position.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, MainThreadMarker, define_class, msg_send};
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager, CLLocationManagerDelegate};
use objc2_foundation::{NSObject, NSObjectProtocol};

/// The last status CoreLocation reported, or `None` while no callback has arrived yet.
///
/// `None` and `Some(notDetermined)` are deliberately different: the first means "we have not heard
/// back", the second means "locationd has answered, and the answer is that nobody has decided".
static STATUS: Mutex<Option<i32>> = Mutex::new(None);
static STATUS_CHANGED: Condvar = Condvar::new();

/// Set once the manager has been scheduled onto the main queue, so repeated calls to [`prime`] from
/// the UI do not stack up a second manager and a second prompt.
static PRIMED: AtomicBool = AtomicBool::new(false);

/// How long the detection path waits for a pending decision before giving up for this attempt.
///
/// The app primes at startup, so by the time the user reaches the Wi-Fi form the answer is normally
/// already in and this wait costs nothing. It only bites on the very first run, where it covers the
/// round trip to `locationd` — not the user reading the prompt, which no timeout could cover.
const DECISION_WAIT: Duration = Duration::from_secs(5);

/// Record a status from the delegate and wake everyone blocked in [`wait_for_decision`].
fn publish(status: CLAuthorizationStatus) {
    let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(status.0);
    drop(guard);
    STATUS_CHANGED.notify_all();
}

define_class!(
    /// Exists purely so CoreLocation has somewhere to deliver the authorization result.
    #[unsafe(super(NSObject))]
    #[name = "GemHostWifiLocationDelegate"]
    struct AuthorizationDelegate;

    impl AuthorizationDelegate {}

    unsafe impl NSObjectProtocol for AuthorizationDelegate {}

    unsafe impl CLLocationManagerDelegate for AuthorizationDelegate {
        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn did_change_authorization(&self, manager: &CLLocationManager) {
            // SAFETY: `manager` is the framework's own manager, passed to its delegate on the run
            // loop it was created on; both calls are plain instance methods on it.
            let status = unsafe { manager.authorizationStatus() };
            tracing::info!("CoreLocation authorization status: {}", describe(status));
            publish(status);
            if status != CLAuthorizationStatus::NotDetermined {
                // The prompt has been answered. Location updates were only ever the lever that
                // makes macOS show it, so drop them rather than keep the radio (and the menu bar
                // indicator) alive for a position this crate never reads.
                unsafe { manager.stopUpdatingLocation() };
            }
        }
    }
);

/// Start the authorization conversation, at most once per process.
///
/// Safe to call from any thread and at any time; the work is always performed on the main queue,
/// because that is the only run loop guaranteed to be alive to receive the callback. Returns
/// immediately — [`wait_for_decision`] is what observes the outcome.
pub(crate) fn prime() {
    if PRIMED.swap(true, Ordering::SeqCst) {
        return;
    }

    tracing::debug!("requesting CoreLocation authorization on the main queue");
    DispatchQueue::main().exec_async(|| {
        // SAFETY: this closure runs on the main queue, so the manager is created on the main run
        // loop, which is what makes the delegate callback deliverable.
        unsafe {
            let delegate: Retained<AuthorizationDelegate> =
                msg_send![AuthorizationDelegate::alloc(), init];
            let manager = CLLocationManager::new();
            manager.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            manager.requestWhenInUseAuthorization();
            // The call that actually surfaces the prompt; stopped again in the delegate.
            manager.startUpdatingLocation();

            // Both objects must outlive this closure: the request is asynchronous, and CoreLocation
            // holds the delegate weakly. Leaking them for the life of the process is the point, not
            // an oversight — there is exactly one, and it is needed until the process exits.
            std::mem::forget(delegate);
            std::mem::forget(manager);
        }
    });
}

/// Block until CoreLocation reports a decision, or `DECISION_WAIT` elapses.
///
/// Returns `None` when nothing has been reported yet, which the caller treats as "not authorized
/// *for now*" rather than as a refusal: the user may still be looking at the prompt.
pub(crate) fn wait_for_decision() -> Option<CLAuthorizationStatus> {
    // On the main thread there is no one else to run the run loop, so blocking here would stop the
    // very callback being waited for. Take whatever is already known instead.
    if MainThreadMarker::new().is_some() {
        return current();
    }

    let deadline = Instant::now() + DECISION_WAIT;
    let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if let Some(raw) = *guard
            && raw != CLAuthorizationStatus::NotDetermined.0
        {
            return Some(CLAuthorizationStatus(raw));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return guard.map(CLAuthorizationStatus);
        }
        let (next, _) = STATUS_CHANGED
            .wait_timeout(guard, remaining)
            .unwrap_or_else(|e| e.into_inner());
        guard = next;
    }
}

/// The last reported status without waiting.
fn current() -> Option<CLAuthorizationStatus> {
    let guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    guard.map(CLAuthorizationStatus)
}

/// A human-readable name for a status, for the log line that explains an empty Wi-Fi form.
pub(crate) fn describe(status: CLAuthorizationStatus) -> &'static str {
    match status {
        CLAuthorizationStatus::NotDetermined => "not determined (no decision yet)",
        CLAuthorizationStatus::Restricted => "restricted by policy",
        CLAuthorizationStatus::Denied => "denied (Location Services off, or refused for this app)",
        CLAuthorizationStatus::AuthorizedAlways => "authorized (always)",
        CLAuthorizationStatus::AuthorizedWhenInUse => "authorized (when in use)",
        _ => "unrecognised",
    }
}

/// Whether a reported status permits reading the Location-gated Wi-Fi fields.
pub(crate) fn is_authorized(status: CLAuthorizationStatus) -> bool {
    matches!(
        status,
        CLAuthorizationStatus::AuthorizedWhenInUse | CLAuthorizationStatus::AuthorizedAlways
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_authorized_statuses_open_the_gate() {
        assert!(is_authorized(CLAuthorizationStatus::AuthorizedWhenInUse));
        assert!(is_authorized(CLAuthorizationStatus::AuthorizedAlways));
        assert!(!is_authorized(CLAuthorizationStatus::NotDetermined));
        assert!(!is_authorized(CLAuthorizationStatus::Denied));
        assert!(!is_authorized(CLAuthorizationStatus::Restricted));
    }

    #[test]
    fn a_published_status_is_visible_to_a_later_reader() {
        // `None` and a reported `NotDetermined` must stay distinguishable: the first means locationd
        // has not answered, the second that it answered "nobody has decided".
        publish(CLAuthorizationStatus::NotDetermined);
        assert_eq!(current(), Some(CLAuthorizationStatus::NotDetermined));
        publish(CLAuthorizationStatus::Denied);
        assert_eq!(current(), Some(CLAuthorizationStatus::Denied));
    }
}
