
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, MainThreadMarker, define_class, msg_send};
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager, CLLocationManagerDelegate};
use objc2_foundation::{NSObject, NSObjectProtocol};

static STATUS: Mutex<Option<i32>> = Mutex::new(None);
static STATUS_CHANGED: Condvar = Condvar::new();

static PRIMED: AtomicBool = AtomicBool::new(false);

const DECISION_WAIT: Duration = Duration::from_secs(5);

fn publish(status: CLAuthorizationStatus) {
    let mut guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(status.0);
    drop(guard);
    STATUS_CHANGED.notify_all();
}

define_class!(
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
                unsafe { manager.stopUpdatingLocation() };
            }
        }
    }
);

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
            manager.startUpdatingLocation();

            std::mem::forget(delegate);
            std::mem::forget(manager);
        }
    });
}

pub(crate) fn wait_for_decision() -> Option<CLAuthorizationStatus> {
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

fn current() -> Option<CLAuthorizationStatus> {
    let guard = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    guard.map(CLAuthorizationStatus)
}

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
        publish(CLAuthorizationStatus::NotDetermined);
        assert_eq!(current(), Some(CLAuthorizationStatus::NotDetermined));
        publish(CLAuthorizationStatus::Denied);
        assert_eq!(current(), Some(CLAuthorizationStatus::Denied));
    }
}
