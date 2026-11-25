//! Find serial devices (servo buses, USB IMUs, …) by USB serial number, which, unlike the port path,
//! is the same on every OS, USB socket and replug: `/dev/cu.usbmodem…` on macOS, `/dev/ttyACM0`/`ACM1`
//! in plug order on Linux.

use serialport::SerialPortType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortInfo {
    pub path: String,
    pub usb_serial: Option<String>,
    pub description: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("listing serial ports: {0}")]
    List(#[from] serialport::Error),
    #[error("no device with USB serial {serial} connected; connected: {connected:?}")]
    NotFound {
        serial: String,
        connected: Vec<(String, String)>,
    },
}

/// Every serial port the OS reports, sorted by path.
pub fn list() -> Result<Vec<PortInfo>, PortError> {
    let mut ports: Vec<PortInfo> = serialport::available_ports()?
        .into_iter()
        .map(|p| {
            let (usb_serial, description) = match p.port_type {
                SerialPortType::UsbPort(u) => {
                    let name = [u.manufacturer, u.product]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" ");
                    (
                        u.serial_number,
                        format!("USB {:04x}:{:04x} {name}", u.vid, u.pid)
                            .trim_end()
                            .to_string(),
                    )
                }
                SerialPortType::PciPort => (None, "PCI".into()),
                SerialPortType::BluetoothPort => (None, "Bluetooth".into()),
                SerialPortType::Unknown => (None, String::new()),
            };
            PortInfo {
                path: p.port_name,
                usb_serial,
                description,
            }
        })
        .collect();
    ports.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(ports)
}

/// Current port path of the connected device with this USB serial number.
pub fn find_port(usb_serial: &str) -> Result<String, PortError> {
    pick(&list()?, usb_serial)
}

fn pick(ports: &[PortInfo], usb_serial: &str) -> Result<String, PortError> {
    // macOS can list a port as both /dev/cu.* and /dev/tty.*; cu.* (sorts first) doesn't wait for
    // carrier detect.
    ports
        .iter()
        .filter(|p| p.usb_serial.as_deref() == Some(usb_serial))
        .map(|p| p.path.clone())
        .min()
        .ok_or_else(|| PortError::NotFound {
            serial: usb_serial.into(),
            connected: ports
                .iter()
                .filter_map(|p| Some((p.path.clone(), p.usb_serial.clone()?)))
                .collect(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(list: &[(&str, &str)]) -> Vec<PortInfo> {
        list.iter()
            .map(|(path, serial)| PortInfo {
                path: path.to_string(),
                usb_serial: Some(serial.to_string()),
                description: String::new(),
            })
            .collect()
    }

    #[test]
    fn finds_port_by_serial_on_macos() {
        let p = ports(&[
            ("/dev/tty.usbmodem5C931505911", "5C93150591"),
            ("/dev/cu.usbmodem5C931505911", "5C93150591"),
            ("/dev/cu.usbmodemX1", "X"),
        ]);
        assert_eq!(
            pick(&p, "5C93150591").unwrap(),
            "/dev/cu.usbmodem5C931505911"
        );
    }

    #[test]
    fn finds_port_by_serial_on_linux() {
        let p = ports(&[("/dev/ttyACM0", "X"), ("/dev/ttyACM1", "5C93150591")]);
        assert_eq!(pick(&p, "5C93150591").unwrap(), "/dev/ttyACM1");
        assert!(matches!(pick(&p, "NOPE"), Err(PortError::NotFound { .. })));
    }
}
