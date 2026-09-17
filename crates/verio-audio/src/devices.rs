//! cpal device enumeration and lookup.

use cpal::traits::{DeviceTrait, HostTrait};

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
}

fn list(host: &cpal::Host, input: bool) -> Result<Vec<DeviceInfo>, String> {
    let default_name = if input {
        host.default_input_device().and_then(|d| d.name().ok())
    } else {
        host.default_output_device().and_then(|d| d.name().ok())
    };
    let mut out = Vec::new();
    let iter = if input {
        host.input_devices()
    } else {
        host.output_devices()
    }
    .map_err(|e| format!("device enumeration failed: {e}"))?;
    for device in iter {
        if let Ok(name) = device.name() {
            let is_default = default_name.as_deref() == Some(name.as_str());
            out.push(DeviceInfo { name, is_default });
        }
    }
    Ok(out)
}

pub fn list_input_devices() -> Result<Vec<DeviceInfo>, String> {
    list(&cpal::default_host(), true)
}

pub fn list_output_devices() -> Result<Vec<DeviceInfo>, String> {
    list(&cpal::default_host(), false)
}

/// Look up a device by name; `None` selects the system default.
pub fn find_input_device(name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match name {
        None => host
            .default_input_device()
            .ok_or_else(|| "no default input device".to_string()),
        Some(want) => {
            for device in host
                .input_devices()
                .map_err(|e| format!("device enumeration failed: {e}"))?
            {
                if device.name().ok().as_deref() == Some(want) {
                    return Ok(device);
                }
            }
            Err(format!("input device not found: {want}"))
        }
    }
}

pub fn find_output_device(name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match name {
        None => host
            .default_output_device()
            .ok_or_else(|| "no default output device".to_string()),
        Some(want) => {
            for device in host
                .output_devices()
                .map_err(|e| format!("device enumeration failed: {e}"))?
            {
                if device.name().ok().as_deref() == Some(want) {
                    return Ok(device);
                }
            }
            Err(format!("output device not found: {want}"))
        }
    }
}
