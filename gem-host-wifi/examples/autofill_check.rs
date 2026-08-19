//! Reproduces the GUI's exact shape: prime on the main thread, run the event loop, and run the
//! detection on a worker thread — then report what autofill would have produced.
fn main() {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            static kCFRunLoopDefaultMode: *const std::ffi::c_void;
            fn CFRunLoopRunInMode(m: *const std::ffi::c_void, s: f64, r: u8) -> i32;
        }

        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();

        // Exactly what gem-imager-gui's `main` now does, before its event loop starts.
        gem_host_wifi::prime_location_authorization();

        let worker = std::thread::spawn(|| {
            // Stands in for the GUI's `blocking_future(detect_host_wifi)` task.
            std::thread::sleep(std::time::Duration::from_millis(300));
            match gem_host_wifi::detect_current_wifi() {
                Ok(w) => {
                    println!("RESULT ssid={:?}", w.ssid.as_utf8());
                    println!("RESULT security={:?}", w.security);
                    println!("RESULT country={:?}", w.country.map(|c| c.code.to_string()));
                    match gem_host_wifi::read_saved_password(&w.network) {
                        Ok(gem_host_wifi::PasswordOutcome::Found(s)) => {
                            println!("RESULT password=<{} bytes, value not shown>", s.len())
                        }
                        Ok(other) => println!("RESULT password_outcome={other:?}"),
                        Err(e) => println!("RESULT password_error={e}"),
                    }
                }
                Err(e) => println!("RESULT error={e}"),
            }
            println!("DONE");
        });

        // Stands in for winit's main-thread event loop.
        while !worker.is_finished() {
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.25, 0) };
        }
        worker.join().unwrap();
    }
}
