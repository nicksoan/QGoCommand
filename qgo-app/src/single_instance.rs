//! Port of the single-instance guard in `App.xaml.cs::OnStartup`.
//!
//! The C# takes a named mutex and, if it already exists, walks the process list
//! looking for another QGo with a `MainWindowHandle` to `SetForegroundWindow`.
//! That approach cannot work here: QGo spends most of its life with the window
//! hidden, so there is no main window handle to find.
//!
//! Instead the running instance owns a named auto-reset event and parks a thread
//! on it. A second launch signals that event and exits, and the first instance
//! wakes up and shows itself — the same user-visible behaviour, and it works
//! while the window is hidden.

#[cfg(windows)]
use windows::core::HSTRING;
#[cfg(windows)]
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject,
};

/// Matches the C#'s `QGo_SingleInstance_{MachineName}_{UserName}_{AsmName}` so
/// different users on one machine still get their own instance.
///
/// The `_Rust` suffix is deliberate: the WPF build and this one are separate
/// programs with separate windows, and blocking each other would be surprising
/// while both are installed.
fn base_name() -> String {
    let machine = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "machine".into());
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    format!("QGo_SingleInstance_{machine}_{user}_QGo_Rust")
}

fn mutex_name() -> String {
    base_name()
}

fn event_name() -> String {
    format!("Local\\{}_Show", base_name())
}

/// Held for the lifetime of the process; dropping it releases the mutex.
pub struct InstanceGuard {
    #[cfg(windows)]
    mutex: HANDLE,
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.mutex);
        }
    }
}

/// The outcome of trying to become the single instance.
pub enum Acquisition {
    /// This process owns the instance and should start normally.
    Acquired(InstanceGuard),
    /// Another instance is running; it has been asked to show itself and this
    /// process should exit.
    AlreadyRunning,
}

/// `OnStartup`'s mutex check.
#[cfg(windows)]
pub fn acquire() -> Acquisition {
    let name = HSTRING::from(mutex_name());

    let handle = unsafe { CreateMutexW(None, true, &name) };
    let Ok(handle) = handle else {
        // If the mutex cannot be created at all, start anyway rather than
        // refusing to launch — the C# makes the same call.
        log::warn!("could not create the single-instance mutex; starting anyway");
        return Acquisition::Acquired(InstanceGuard {
            mutex: HANDLE::default(),
        });
    };

    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

    if already_running {
        unsafe {
            let _ = CloseHandle(handle);
        }
        signal_existing_instance();
        return Acquisition::AlreadyRunning;
    }

    Acquisition::Acquired(InstanceGuard { mutex: handle })
}

#[cfg(not(windows))]
pub fn acquire() -> Acquisition {
    Acquisition::Acquired(InstanceGuard {})
}

/// Replaces `TryActivateExistingInstanceWindow()`.
#[cfg(windows)]
fn signal_existing_instance() {
    let name = HSTRING::from(event_name());
    // Auto-reset, initially unset. Creating an existing name opens it.
    if let Ok(event) = unsafe { CreateEventW(None, false, false, &name) } {
        unsafe {
            let _ = SetEvent(event);
            let _ = CloseHandle(event);
        }
        log::info!("another instance is running; asked it to show itself");
    }
}

/// Parks a thread on the named event and runs `on_show` each time a second
/// launch signals it.
///
/// The callback runs on this worker thread, so it must only nudge the UI —
/// `Context::request_repaint` and a channel send — never touch egui state.
#[cfg(windows)]
pub fn listen_for_show_requests<F>(on_show: F)
where
    F: Fn() + Send + 'static,
{
    std::thread::spawn(move || {
        let name = HSTRING::from(event_name());
        let Ok(event) = (unsafe { CreateEventW(None, false, false, &name) }) else {
            log::warn!("could not create the show-request event; second launches will be ignored");
            return;
        };

        loop {
            let result = unsafe { WaitForSingleObject(event, u32::MAX) };
            if result == WAIT_OBJECT_0 {
                on_show();
            } else {
                break;
            }
        }

        unsafe {
            let _ = CloseHandle(event);
        }
    });
}

#[cfg(not(windows))]
pub fn listen_for_show_requests<F>(_on_show: F)
where
    F: Fn() + Send + 'static,
{
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_are_per_user_and_stable() {
        assert_eq!(base_name(), base_name());
        assert!(base_name().starts_with("QGo_SingleInstance_"));
    }

    #[test]
    fn the_event_lives_in_the_local_namespace() {
        // A Global\ name would need elevation and would leak across sessions.
        assert!(event_name().starts_with("Local\\"));
        assert!(event_name().ends_with("_Show"));
    }

    #[test]
    fn the_mutex_and_event_names_differ() {
        assert_ne!(mutex_name(), event_name());
    }
}
