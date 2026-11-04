//! A mirror of a service's objects through `org.freedesktop.DBus.ObjectManager`,
//! kept current by its signals: NetworkManager and BlueZ publish this way.

use crate::client::{Client, Props, properties_changed};
use aegle_dbus::{Message, Value};
use std::{collections::BTreeMap, io};

const MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// Objects by path, then their interfaces' properties by name.
#[derive(Default)]
pub(crate) struct Objects(BTreeMap<String, BTreeMap<String, Props>>);

impl Objects {
    /// Every object `destination` manages under `path`, and the match rule
    /// for its later changes.
    pub fn load(bus: &mut Client, destination: &str, path: &str) -> io::Result<Self> {
        bus.add_match(&format!("type='signal',sender='{destination}'"))?;
        let reply = bus.method(destination, path, MANAGER, "GetManagedObjects", vec![])?;
        let mut objects = Self::default();
        for item in reply.first().map(Value::items).unwrap_or(&[]) {
            if let Value::Entry(entry) = item
                && let Some(path) = entry.0.as_str()
            {
                objects.add(path, &entry.1);
            }
        }
        Ok(objects)
    }

    /// Takes interfaces as `a{sa{sv}}`.
    fn add(&mut self, path: &str, interfaces: &Value) {
        let object = self.0.entry(path.to_owned()).or_default();
        for item in interfaces.items() {
            if let Value::Entry(entry) = item
                && let Some(name) = entry.0.as_str()
            {
                object.insert(name.to_owned(), Props::from_dict(&entry.1));
            }
        }
    }

    /// Applies an object manager or properties signal; true if it changed
    /// something.
    pub fn apply(&mut self, message: &Message) -> bool {
        if message.is_signal(MANAGER, "InterfacesAdded") {
            if let [path, interfaces] = &message.body[..]
                && let Some(path) = path.as_str()
            {
                self.add(path, interfaces);
                return true;
            }
        } else if message.is_signal(MANAGER, "InterfacesRemoved") {
            if let [path, names] = &message.body[..]
                && let Some(path) = path.as_str()
                && let Some(object) = self.0.get_mut(path)
            {
                for name in names.items().iter().filter_map(Value::as_str) {
                    object.remove(name);
                }
                if object.is_empty() {
                    self.0.remove(path);
                }
                return true;
            }
        } else if let Some((interface, changed, invalidated)) = properties_changed(message)
            && let Some(props) = self
                .0
                .get_mut(&message.path)
                .and_then(|o| o.get_mut(interface))
        {
            props.merge(changed);
            invalidated.iter().for_each(|name| props.remove(name));
            return true;
        }
        false
    }

    /// The properties of `interface` on the object at `path`.
    pub fn get(&self, path: &str, interface: &str) -> Option<&Props> {
        self.0.get(path)?.get(interface)
    }

    /// Every object with `interface`, by path.
    pub fn with<'a>(&'a self, interface: &'a str) -> impl Iterator<Item = (&'a str, &'a Props)> {
        (self.0.iter()).filter_map(move |(path, o)| Some((path.as_str(), o.get(interface)?)))
    }
}
