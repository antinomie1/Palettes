//! System services over D-Bus, each on its own thread and connection:
//! battery and power profiles (UPower), media players (MPRIS), a
//! notification server, a system tray (StatusNotifierItem and DBusMenu),
//! network (NetworkManager), bluetooth (BlueZ) and the backlight (sysfs,
//! set through logind); and processor and memory use from `/proc`.
//!
//! A service starts with [`System::spawn`], reads what it shows, then blocks
//! on its bus until a signal changes it; it calls back with its whole state
//! only when that changed. Requests go to its thread and never wait for a
//! reply on the caller's. [`Link`] is a plain connection for scripts.

mod battery;
mod bluetooth;
mod brightness;
mod client;
mod link;
mod load;
mod media;
mod network;
mod notifications;
mod objects;
mod pixmap;
mod tray;

pub use aegle_dbus::{Kind, Message, Value};
pub use battery::{Battery, BatteryState, Charge};
pub use bluetooth::{Bluetooth, BluetoothDevice, BluetoothState};
pub use brightness::{BACKLIGHTS, Brightness, BrightnessState};
pub use client::{Bus, CALL_TIMEOUT};
pub use link::Link;
pub use load::{Load, LoadState, cpu_times, memory};
pub use media::{Control, Media, MediaState, Player};
pub use network::{Network, NetworkState, WifiNetwork};
pub use notifications::{Notification, Notifications, NotificationsState};
pub use pixmap::local_path;
pub use tray::{Menu, MenuEntry, Tray, TrayItem, TrayState};

/// What a service calls with its state.
pub type Report<T> = Box<dyn FnMut(&T) + Send>;

/// A system service.
pub trait System: Sized + 'static {
    /// What it reports.
    type State: Clone + Send + 'static;

    /// The bus it is on.
    const BUS: Bus;

    /// Connects to `bus` and starts the service's thread; `changed` runs
    /// there with the state once read and after each change.
    fn spawn(bus: &Bus, changed: Report<Self::State>) -> std::io::Result<Self>;
}
