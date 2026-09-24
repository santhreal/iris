//! A Vulkan driver with no devices that makes X connections to the
//! server `DISPLAY` names. When the loader loads it, it sets up a
//! connection and closes it, as Mesa's device selection layer does as it
//! enumerates devices; the NVIDIA driver connects when it is loaded.
//! When the library is unloaded, at a `dlclose` or at the exit of a
//! process that still has it loaded, it sets up a connection and waits
//! for the server's answer without a bound, as the NVIDIA driver's
//! destructor makes a round trip to the X server. It appends a line to
//! the file `IRIS_TEST_DRIVER_LOG` names at each load and at each device
//! enumeration: `load` or `devices`, then the `DISPLAY` it saw, or `-`
//! for none.
//!
//! tests/support/driver.rs builds it into a shared library with rustc.

use std::ffi::{c_char, c_void, CStr};
use std::io::{ErrorKind, Read as _, Write as _};
use std::os::linux::net::SocketAddrExt as _;
use std::os::unix::net::{SocketAddr, UnixStream};
use std::ptr::null;
use std::time::Duration;

const SUCCESS: i32 = 0;
/// The loader-driver interface version this driver implements: its
/// instance accepts every API version.
const INTERFACE_VERSION: u32 = 5;
/// The value a driver stores in the first word of a dispatchable handle.
const LOADER_MAGIC: usize = 0x01CD_C0DE;

/// Append `event` and the `DISPLAY` this process has to the log, and
/// return that `DISPLAY`.
fn record(event: &str) -> Option<String> {
    let display = std::env::var("DISPLAY").ok();
    if let Some(path) = std::env::var_os("IRIS_TEST_DRIVER_LOG") {
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
        {
            let _ = writeln!(log, "{event} {}", display.as_deref().unwrap_or("-"));
        }
    }
    display
}

/// Set up a connection to the X server `display` names, `:N` or `:N.S`,
/// and wait at most `bound` for its answer, or without a bound for
/// `None`. The connection closes on return.
fn set_up(display: &str, bound: Option<Duration>) -> std::io::Result<()> {
    let number = display
        .strip_prefix(':')
        .and_then(|name| name.split('.').next())
        .ok_or(ErrorKind::InvalidInput)?;
    // An X client reaches display `:N` through the abstract socket
    // `/tmp/.X11-unix/XN` first.
    let name = format!("/tmp/.X11-unix/X{number}");
    let mut stream = UnixStream::connect_addr(&SocketAddr::from_abstract_name(name.as_bytes())?)?;
    stream.set_read_timeout(bound)?;
    // The setup request: little-endian byte order, protocol 11.0, and no
    // authorization. The first byte of the answer is its status.
    stream.write_all(&[b'l', 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0])?;
    stream.read_exact(&mut [0u8])
}

/// Runs when the library is unloaded.
#[used]
#[link_section = ".fini_array"]
static UNLOAD: extern "C" fn() = unload;

extern "C" fn unload() {
    if let Ok(display) = std::env::var("DISPLAY") {
        let _ = set_up(&display, None);
    }
}

#[no_mangle]
pub unsafe extern "C" fn vk_icdNegotiateLoaderICDInterfaceVersion(version: *mut u32) -> i32 {
    if let Some(display) = record("load") {
        let _ = set_up(&display, Some(Duration::from_secs(5)));
    }
    *version = (*version).min(INTERFACE_VERSION);
    SUCCESS
}

#[no_mangle]
pub unsafe extern "C" fn vk_icdGetInstanceProcAddr(
    _instance: *mut c_void,
    name: *const c_char,
) -> *const c_void {
    match CStr::from_ptr(name).to_bytes() {
        b"vkGetInstanceProcAddr" => vk_icdGetInstanceProcAddr as *const c_void,
        b"vkCreateInstance" => create_instance as *const c_void,
        b"vkDestroyInstance" => destroy_instance as *const c_void,
        b"vkEnumerateInstanceExtensionProperties" => enumerate_extensions as *const c_void,
        b"vkEnumeratePhysicalDevices" => enumerate_devices as *const c_void,
        b"vkGetPhysicalDeviceFeatures"
        | b"vkGetPhysicalDeviceFormatProperties"
        | b"vkGetPhysicalDeviceImageFormatProperties"
        | b"vkGetPhysicalDeviceProperties"
        | b"vkGetPhysicalDeviceQueueFamilyProperties"
        | b"vkGetPhysicalDeviceMemoryProperties"
        | b"vkGetPhysicalDeviceSparseImageFormatProperties"
        | b"vkEnumerateDeviceExtensionProperties"
        | b"vkCreateDevice"
        | b"vkGetDeviceProcAddr" => no_device as *const c_void,
        _ => null(),
    }
}

unsafe extern "C" fn create_instance(
    _info: *const c_void,
    _allocator: *const c_void,
    instance: *mut *mut c_void,
) -> i32 {
    *instance = Box::into_raw(Box::new(LOADER_MAGIC)).cast();
    SUCCESS
}

unsafe extern "C" fn destroy_instance(instance: *mut c_void, _allocator: *const c_void) {
    if !instance.is_null() {
        drop(Box::from_raw(instance.cast::<usize>()));
    }
}

unsafe extern "C" fn enumerate_extensions(
    _layer: *const c_char,
    count: *mut u32,
    _properties: *mut c_void,
) -> i32 {
    *count = 0;
    SUCCESS
}

unsafe extern "C" fn enumerate_devices(
    _instance: *mut c_void,
    count: *mut u32,
    _devices: *mut *mut c_void,
) -> i32 {
    record("devices");
    *count = 0;
    SUCCESS
}

/// Each core 1.0 function the loader requires of a driver before it
/// enumerates the driver's devices, and calls only with one of them. The
/// driver has none.
unsafe extern "C" fn no_device() {
    std::process::abort();
}
