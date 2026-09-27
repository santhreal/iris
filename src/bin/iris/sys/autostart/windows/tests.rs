use windows_sys::Win32::System::Registry::{RegDeleteTreeW, REG_BINARY};

use super::*;

/// A key of this test's own under HKEY_CURRENT_USER\Software\iris-tests
/// with Run and StartupApproved subkeys, deleted when dropped.
struct Scratch {
    root: String,
    run: String,
    approved: String,
}

impl Scratch {
    fn new(test: &str) -> Self {
        let root = format!(
            r"Software\iris-tests\autostart-{}-{test}",
            std::process::id()
        );
        Scratch {
            run: format!(r"{root}\Run"),
            approved: format!(r"{root}\StartupApproved"),
            root,
        }
    }

    fn keys(&self) -> Keys<'_> {
        Keys {
            run: &self.run,
            approved: &self.approved,
        }
    }

    /// Write `data` as the REG_BINARY `iris` value of `subkey`.
    fn write_binary(&self, subkey: &str, data: &[u8]) {
        let (subkey, name) = (wide(OsStr::new(subkey)), wide(OsStr::new(VALUE)));
        let mut key: HKEY = null_mut();
        // SAFETY: NUL-terminated names; `key` is closed below.
        unsafe {
            assert_eq!(
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    0,
                    null(),
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    null(),
                    &mut key,
                    null_mut(),
                ),
                ERROR_SUCCESS
            );
            let rc = RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_BINARY,
                data.as_ptr(),
                data.len() as u32,
            );
            RegCloseKey(key);
            assert_eq!(rc, ERROR_SUCCESS);
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let root = wide(OsStr::new(&self.root));
        // SAFETY: a NUL-terminated name under this test's own key.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, root.as_ptr()) };
    }
}

const EXE: &str = r"C:\Users\a b\AppData\Local\Programs\iris\iris.exe";

fn this() -> OsString {
    command_for(Path::new(EXE))
}

#[test]
fn the_command_is_the_one_the_installer_writes() {
    assert_eq!(this(), OsString::from(format!("\"{EXE}\" --daemon")));
}

#[test]
fn a_run_value_starts_this_iris_once_turned_on() {
    let scratch = Scratch::new("on-off");
    let keys = scratch.keys();
    assert!(!enabled_in(&keys, &this()));
    set_in(&keys, &this(), true).expect("on");
    assert!(enabled_in(&keys, &this()));
    assert_eq!(
        read(keys.run, RRF_RT_REG_SZ),
        Some(wide(&this())),
        "REG_SZ data ends in one NUL"
    );
    // Windows paths ignore case; another iris.exe is not this one.
    let upper = OsString::from(this().to_string_lossy().to_uppercase());
    assert!(enabled_in(&keys, &upper));
    assert!(!enabled_in(
        &keys,
        &command_for(Path::new(r"D:\iris\iris.exe"))
    ));
    set_in(&keys, &this(), false).expect("off");
    assert!(!enabled_in(&keys, &this()));
    assert_eq!(read(keys.run, RRF_RT_REG_SZ), None);
    set_in(&keys, &this(), false).expect("off again");
}

/// Task Manager writes 02 as the first byte for on and 03 for off; the
/// low bit is the switch.
#[test]
fn task_manager_turns_it_off_and_on_turns_it_back_on() {
    let scratch = Scratch::new("approved");
    let keys = scratch.keys();
    set_in(&keys, &this(), true).expect("on");
    for (first, on) in [(0x02, true), (0x03, false), (0x06, true), (0x07, false)] {
        scratch.write_binary(&scratch.approved, &[first, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(enabled_in(&keys, &this()), on, "{first:#04x}");
    }
    set_in(&keys, &this(), true).expect("on over Task Manager's off");
    assert!(enabled_in(&keys, &this()));
    assert_eq!(read(keys.approved, RRF_RT_REG_BINARY), None);
    scratch.write_binary(&scratch.approved, &[0x03; 12]);
    set_in(&keys, &this(), false).expect("off");
    assert_eq!(read(keys.approved, RRF_RT_REG_BINARY), None);
}
