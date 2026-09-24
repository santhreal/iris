//! A Vulkan driver with no devices that connects to the X server `DISPLAY`
//! names when the loader loads it, as the NVIDIA driver does. It appends a
//! line to the file `IRIS_TEST_DRIVER_LOG` names at each load and at each
//! device enumeration: `load` or `devices`, then the `DISPLAY` it saw, or
//! `-` for none.
//!
//! tests/startup.rs builds it into a shared library with rustc.

use std::ffi::{c_char, c_void, CStr};
use std::io::Write as _;
use std::os::linux::net::SocketAddrExt as _;
use std::os::unix::net::{SocketAddr, UnixStream};
use std::ptr::null;

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

#[no_mangle]
pub unsafe extern "C" fn vk_icdNegotiateLoaderICDInterfaceVersion(version: *mut u32) -> i32 {
    // An X client reaches display `:N` or `:N.S` through the abstract
    // socket `/tmp/.X11-unix/XN` first.
    let display = record("load");
    let number = display
        .as_deref()
        .and_then(|name| name.strip_prefix(':'))
        .and_then(|name| name.split('.').next());
    if let Some(number) = number {
        let name = format!("/tmp/.X11-unix/X{number}");
        if let Ok(address) = SocketAddr::from_abstract_name(name.as_bytes()) {
            let _ = UnixStream::connect_addr(&address);
        }
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
