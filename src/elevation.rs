use std::io;

/// Ensures that only an elevated multimeter instance continues startup.
///
/// Returns `Ok(true)` when an elevated instance was launched and the current
/// unprivileged instance should exit. Returns `Ok(false)` when the current
/// process already has administrator permissions.
pub fn relaunch_if_needed() -> io::Result<bool> {
    relaunch_if_needed_with(platform::is_elevated, platform::launch_elevated)
}

fn relaunch_if_needed_with(
    is_elevated: impl FnOnce() -> io::Result<bool>,
    launch_elevated: impl FnOnce() -> io::Result<()>,
) -> io::Result<bool> {
    if is_elevated()? {
        return Ok(false);
    }

    launch_elevated()?;
    Ok(true)
}

fn error_context(operation: &str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{operation}: {error}"))
}

#[cfg(unix)]
mod platform {
    use std::env;
    use std::io;
    use std::process::Command;

    pub(super) fn is_elevated() -> io::Result<bool> {
        // SAFETY: geteuid has no preconditions and does not access memory.
        Ok(unsafe { libc::geteuid() } == 0)
    }

    pub(super) fn launch_elevated() -> io::Result<()> {
        let executable = env::current_exe()
            .map_err(|error| super::error_context("resolve multimeter executable", error))?;

        let authentication = Command::new("sudo")
            .arg("-v")
            .status()
            .map_err(|error| super::error_context("start sudo authentication", error))?;
        if !authentication.success() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("sudo authentication failed ({authentication})"),
            ));
        }

        Command::new("sudo")
            .arg("-n")
            .arg("--")
            .arg(executable)
            .args(env::args_os().skip(1))
            .spawn()
            .map(|_| ())
            .map_err(|error| super::error_context("launch elevated multimeter instance", error))
    }
}

#[cfg(windows)]
mod platform {
    use std::env;
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, GetLastError, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    pub(super) fn is_elevated() -> io::Result<bool> {
        let mut token: HANDLE = ptr::null_mut();

        // SAFETY: GetCurrentProcess returns the current process pseudo-handle,
        // and token points to valid writable memory.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(super::error_context(
                "open current process token",
                io::Error::last_os_error(),
            ));
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned_size = 0;
        // SAFETY: token was opened for TOKEN_QUERY and elevation is correctly
        // sized and aligned for TOKEN_ELEVATION.
        let status = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&raw mut elevation).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned_size,
            )
        };
        let error = (status == 0).then(io::Error::last_os_error);

        // SAFETY: token is a valid handle returned by OpenProcessToken.
        unsafe {
            CloseHandle(token);
        }

        if let Some(error) = error {
            return Err(super::error_context("read process elevation", error));
        }

        Ok(elevation.TokenIsElevated != 0)
    }

    pub(super) fn launch_elevated() -> io::Result<()> {
        let executable = env::current_exe()
            .map_err(|error| super::error_context("resolve multimeter executable", error))?;
        let current_directory = env::current_dir()
            .map_err(|error| super::error_context("resolve current directory", error))?;
        let operation = wide(OsStr::new("runas"))?;
        let executable = wide(executable.as_os_str())?;
        let current_directory = wide(current_directory.as_os_str())?;
        let parameters = windows_arguments(env::args_os().skip(1))?;
        let parameters = if parameters.is_empty() {
            ptr::null()
        } else {
            parameters.as_ptr()
        };

        // SAFETY: All pointers reference null-terminated UTF-16 buffers that
        // remain alive for the duration of the call. ShellExecuteW has no
        // additional pointer-lifetime requirements.
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                operation.as_ptr(),
                executable.as_ptr(),
                parameters,
                current_directory.as_ptr(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            let code = unsafe { GetLastError() };
            if code == ERROR_CANCELLED {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "administrator elevation was cancelled",
                ));
            }

            let kind = io::Error::from_raw_os_error(code as i32).kind();
            return Err(io::Error::new(
                kind,
                format!(
                    "launch elevated multimeter instance failed with result {} (Windows error {code})",
                    result as isize
                ),
            ));
        }

        Ok(())
    }

    fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
        let mut encoded: Vec<u16> = value.encode_wide().collect();
        if encoded.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains a null character",
            ));
        }
        encoded.push(0);
        Ok(encoded)
    }

    pub(super) fn windows_arguments(
        arguments: impl Iterator<Item = OsString>,
    ) -> io::Result<Vec<u16>> {
        let mut command_line = Vec::new();

        for argument in arguments {
            if !command_line.is_empty() {
                command_line.push(u16::from(b' '));
            }
            append_quoted_argument(&mut command_line, &argument)?;
        }

        if !command_line.is_empty() {
            command_line.push(0);
        }
        Ok(command_line)
    }

    fn append_quoted_argument(command_line: &mut Vec<u16>, argument: &OsStr) -> io::Result<()> {
        let units: Vec<u16> = argument.encode_wide().collect();
        if units.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "argument contains a null character",
            ));
        }

        command_line.push(u16::from(b'"'));
        let mut backslashes = 0;
        for unit in units {
            if unit == u16::from(b'\\') {
                backslashes += 1;
                continue;
            }

            if unit == u16::from(b'"') {
                command_line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
                command_line.push(unit);
            } else {
                command_line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
                command_line.push(unit);
            }
            backslashes = 0;
        }

        command_line.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
        command_line.push(u16::from(b'"'));
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::io;

    pub(super) fn is_elevated() -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "administrator permission detection is not supported on this platform",
        ))
    }

    pub(super) fn launch_elevated() -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "administrator permission elevation is not supported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io;

    use super::relaunch_if_needed_with;

    #[test]
    fn keeps_an_elevated_instance_running() {
        let launched = Cell::new(false);

        let relaunched = relaunch_if_needed_with(
            || Ok(true),
            || {
                launched.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert!(!relaunched);
        assert!(!launched.get());
    }

    #[test]
    fn exits_after_launching_an_elevated_instance() {
        let relaunched = relaunch_if_needed_with(|| Ok(false), || Ok(())).unwrap();

        assert!(relaunched);
    }

    #[test]
    fn does_not_continue_when_elevated_launch_fails() {
        let error = relaunch_if_needed_with(
            || Ok(false),
            || {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "elevation declined",
                ))
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn does_not_continue_when_permission_detection_fails() {
        let error = relaunch_if_needed_with(
            || Err(io::Error::other("cannot inspect permissions")),
            || panic!("elevated instance must not launch after detection failure"),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    #[cfg(windows)]
    #[test]
    fn quotes_windows_arguments() {
        use std::ffi::OsString;

        let arguments = [
            OsString::from("plain"),
            OsString::from("with space"),
            OsString::from(r#"with"quote"#),
            OsString::from(r#"trailing\"#),
        ];

        let encoded = super::platform::windows_arguments(arguments.into_iter()).unwrap();
        let decoded = String::from_utf16(&encoded[..encoded.len() - 1]).unwrap();

        assert_eq!(
            decoded,
            r#""plain" "with space" "with\"quote" "trailing\\""#
        );
    }
}
