//! The PipeWire thread: registry, default metadata, node props, commands,
//! and the peak capture once asked for.
//!
//! Volume and mute are read from a node's `Props`. They are written through
//! the device's active `Route` when the node has one, as mixers do, so the
//! hardware route keeps and saves them; virtual nodes take `Props` directly.

use crate::{AudioState, Command, Endpoint, Node as Which, Stream, Target, default_name};
use pipewire::{
    self as pw,
    device::{Device, DeviceListener},
    metadata::{Metadata, MetadataListener},
    node::{Node, NodeListener},
    spa::{
        self,
        param::ParamType,
        pod::{
            Object, Pod, Property, PropertyFlags, Value, ValueArray, deserialize::PodDeserializer,
            serialize::PodSerializer,
        },
    },
    types::ObjectType,
};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::mpsc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Sink,
    Source,
    Stream,
}

struct Tracked {
    node: Node,
    _listener: NodeListener,
    name: String,
    description: String,
    class: Class,
    /// A stream's application, its icon and what it plays.
    app: String,
    icon: String,
    title: String,
    volumes: Vec<f32>,
    muted: bool,
    /// The device's global id and the route device (`card.profile.device`).
    route: Option<(u32, i32)>,
}

struct Routes {
    device: Device,
    _listener: DeviceListener,
    /// Active routes: route device → route index.
    active: HashMap<i32, i32>,
}

#[derive(Default)]
struct Graph {
    nodes: HashMap<u32, Tracked>,
    devices: HashMap<u32, Routes>,
    metadata: Vec<(Metadata, MetadataListener)>,
    default_sink: String,
    default_source: String,
    pub peak: f32,
    last: Option<AudioState>,
}

fn average(volumes: &[f32]) -> f32 {
    (volumes.iter().sum::<f32>() / volumes.len().max(1) as f32).cbrt()
}

impl Graph {
    /// Nodes of `class` whose props arrived: no made-up volumes.
    fn known(&self, class: Class) -> impl Iterator<Item = (&u32, &Tracked)> {
        (self.nodes.iter()).filter(move |(_, n)| n.class == class && !n.volumes.is_empty())
    }

    fn endpoints(&self, class: Class) -> Vec<Endpoint> {
        let mut found: Vec<Endpoint> = self
            .known(class)
            .map(|(_, node)| Endpoint {
                name: node.name.clone(),
                description: node.description.clone(),
                volume: average(&node.volumes),
                muted: node.muted,
            })
            .collect();
        found.sort_by(|a, b| a.description.cmp(&b.description).then(a.name.cmp(&b.name)));
        found
    }

    /// The state if it changed since the last report.
    fn changed(&mut self) -> Option<AudioState> {
        let (sinks, sources) = (self.endpoints(Class::Sink), self.endpoints(Class::Source));
        let pick = |list: &[Endpoint], name: &str| list.iter().find(|e| e.name == name).cloned();
        let mut streams: Vec<Stream> = self
            .known(Class::Stream)
            .map(|(&id, node)| Stream {
                id,
                app: node.app.clone(),
                icon: node.icon.clone(),
                title: node.title.clone(),
                volume: average(&node.volumes),
                muted: node.muted,
            })
            .collect();
        streams.sort_by(|a, b| a.app.cmp(&b.app).then(a.id.cmp(&b.id)));
        let state = AudioState {
            sink: pick(&sinks, &self.default_sink),
            source: pick(&sources, &self.default_source),
            sinks,
            sources,
            streams,
            peak: self.peak,
        };
        (self.last.as_ref() != Some(&state)).then(|| {
            self.last = Some(state.clone());
            state
        })
    }

    fn target(&self, which: Which) -> Option<&Tracked> {
        let (name, class) = match which {
            Which::Default(Target::Sink) => (&self.default_sink, Class::Sink),
            Which::Default(Target::Source) => (&self.default_source, Class::Source),
            Which::Stream(id) => return self.nodes.get(&id).filter(|n| n.class == Class::Stream),
        };
        self.nodes
            .values()
            .find(|n| n.class == class && &n.name == name)
    }
}

/// Reads `channelVolumes` and `mute` from a `Props` object.
fn read_props(pod: &Pod, node: &mut Tracked) {
    let Ok((_, Value::Object(object))) = PodDeserializer::deserialize_any_from(pod.as_bytes())
    else {
        return;
    };
    for property in object.properties {
        match (property.key, property.value) {
            (spa::sys::SPA_PROP_channelVolumes, Value::ValueArray(ValueArray::Float(volumes))) => {
                node.volumes = volumes;
            }
            (spa::sys::SPA_PROP_mute, Value::Bool(muted)) => node.muted = muted,
            _ => {}
        }
    }
}

/// Records an active route's index under its route device.
fn read_route(pod: &Pod, routes: &mut Routes) {
    let Ok((_, Value::Object(object))) = PodDeserializer::deserialize_any_from(pod.as_bytes())
    else {
        return;
    };
    let (mut index, mut device) = (None, None);
    for property in object.properties {
        match (property.key, property.value) {
            (spa::sys::SPA_PARAM_ROUTE_index, Value::Int(i)) => index = Some(i),
            (spa::sys::SPA_PARAM_ROUTE_device, Value::Int(d)) => device = Some(d),
            _ => {}
        }
    }
    if let (Some(index), Some(device)) = (index, device) {
        routes.active.insert(device, index);
    }
}

fn pod_bytes(value: &Value) -> Option<Vec<u8>> {
    let (cursor, _) = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), value).ok()?;
    Some(cursor.into_inner())
}

fn props(properties: Vec<Property>) -> Value {
    Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Props,
        id: spa::sys::SPA_PARAM_Props,
        properties,
    })
}

/// Writes `properties` through the node's device route, or the node.
fn set_props(graph: &Graph, node: &Tracked, properties: Vec<Property>) {
    let route = node.route.and_then(|(device, route_device)| {
        let routes = graph.devices.get(&device)?;
        Some((routes, route_device, *routes.active.get(&route_device)?))
    });
    match route {
        Some((routes, route_device, index)) => {
            let route = Value::Object(Object {
                type_: spa::sys::SPA_TYPE_OBJECT_ParamRoute,
                id: spa::sys::SPA_PARAM_Route,
                properties: vec![
                    Property::new(spa::sys::SPA_PARAM_ROUTE_index, Value::Int(index)),
                    Property::new(spa::sys::SPA_PARAM_ROUTE_device, Value::Int(route_device)),
                    Property::new(spa::sys::SPA_PARAM_ROUTE_props, props(properties)),
                    Property::new(spa::sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
                ],
            });
            if let Some(bytes) = pod_bytes(&route)
                && let Some(pod) = Pod::from_bytes(&bytes)
            {
                routes.device.set_param(ParamType::Route, 0, pod);
            }
        }
        None => {
            if let Some(bytes) = pod_bytes(&props(properties))
                && let Some(pod) = Pod::from_bytes(&bytes)
            {
                node.node.set_param(ParamType::Props, 0, pod);
            }
        }
    }
}

fn command(graph: &Graph, command: Command) {
    let target = match command {
        Command::Volume(target, _) | Command::Mute(target, _) => target,
        Command::Default(target, name) => {
            let key = match target {
                Target::Sink => "default.configured.audio.sink",
                Target::Source => "default.configured.audio.source",
            };
            // The session manager saves the choice and moves streams to it.
            let value = format!("{{\"name\":{}}}", json_string(&name));
            for (metadata, _) in &graph.metadata {
                metadata.set_property(0, key, Some("Spa:String:JSON"), Some(&value));
            }
            return;
        }
        Command::Peak => return,
    };
    let Some(node) = graph.target(target) else {
        return;
    };
    let property = match command {
        Command::Volume(_, volume) => {
            // Mixers show the cube root of the linear channel volume.
            let linear = volume * volume * volume;
            let channels = node.volumes.len().max(1);
            Property::new(
                spa::sys::SPA_PROP_channelVolumes,
                Value::ValueArray(ValueArray::Float(vec![linear; channels])),
            )
        }
        Command::Mute(_, muted) => Property::new(
            spa::sys::SPA_PROP_mute,
            Value::Bool(muted.unwrap_or(!node.muted)),
        ),
        Command::Default(..) | Command::Peak => return,
    };
    set_props(
        graph,
        node,
        vec![Property {
            flags: PropertyFlags::empty(),
            ..property
        }],
    );
}

/// `text` as a JSON string.
fn json_string(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn run(
    commands: pw::channel::Receiver<Command>,
    changed: impl FnMut(&AudioState) + 'static,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let setup = || -> Result<_, pw::Error> {
        let main_loop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&main_loop, None)?;
        let core = context.connect_rc(None)?;
        let registry = core.get_registry_rc()?;
        Ok((main_loop, context, core, registry))
    };
    let (main_loop, _context, core, registry) = match setup() {
        Ok(parts) => parts,
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return;
        }
    };
    let graph = Rc::new(RefCell::new(Graph::default()));
    let changed = Rc::new(RefCell::new(changed));
    let report = {
        let (graph, changed) = (graph.clone(), changed.clone());
        Rc::new(move || {
            let state = graph.borrow_mut().changed();
            if let Some(state) = state {
                (changed.borrow_mut())(&state);
            }
        })
    };

    let weak_registry = registry.downgrade();
    let (global_graph, global_report) = (graph.clone(), report.clone());
    let _registry_listener = registry
        .add_listener_local()
        .global(move |object| {
            let Some(registry) = weak_registry.upgrade() else {
                return;
            };
            let props = object.props.as_ref();
            let get = |key| props.and_then(|p| p.get(key)).unwrap_or("").to_owned();
            match object.type_ {
                ObjectType::Node => {
                    let class = match get("media.class").as_str() {
                        "Audio/Sink" => Class::Sink,
                        "Audio/Source" => Class::Source,
                        "Stream/Output/Audio" => Class::Stream,
                        _ => return,
                    };
                    let Ok(node) = registry.bind::<Node, _>(object) else {
                        return;
                    };
                    let (graph, report, id) =
                        (global_graph.clone(), global_report.clone(), object.id);
                    let (info_graph, info_report) = (global_graph.clone(), global_report.clone());
                    let listener = node
                        .add_listener_local()
                        // The registry's properties are a subset; the route is only here.
                        .info(move |info| {
                            let Some(props) = info.props() else { return };
                            let device = props.get("device.id").and_then(|v| v.parse::<u32>().ok());
                            let route = props
                                .get("card.profile.device")
                                .and_then(|v| v.parse::<i32>().ok());
                            let text = |key| props.get(key).unwrap_or("").to_owned();
                            if let Some(node) = info_graph.borrow_mut().nodes.get_mut(&id) {
                                node.route = device.zip(route);
                                node.app = text("application.name");
                                node.icon = text("application.icon-name");
                                node.title = text("media.name");
                            }
                            info_report();
                        })
                        .param(move |_, kind, _, _, pod| {
                            if kind != ParamType::Props {
                                return;
                            }
                            let Some(pod) = pod else { return };
                            if let Some(node) = graph.borrow_mut().nodes.get_mut(&id) {
                                read_props(pod, node);
                            }
                            report();
                        })
                        .register();
                    node.subscribe_params(&[ParamType::Props]);
                    let description = Some(get("node.description"))
                        .filter(|d| !d.is_empty())
                        .unwrap_or_else(|| get("node.nick"));
                    global_graph.borrow_mut().nodes.insert(
                        object.id,
                        Tracked {
                            node,
                            _listener: listener,
                            name: get("node.name"),
                            description,
                            class,
                            app: get("application.name"),
                            icon: String::new(),
                            title: String::new(),
                            volumes: Vec::new(),
                            muted: false,
                            route: None,
                        },
                    );
                }
                ObjectType::Device if get("media.class") == "Audio/Device" => {
                    let Ok(device) = registry.bind::<Device, _>(object) else {
                        return;
                    };
                    let (graph, id) = (global_graph.clone(), object.id);
                    let listener = device
                        .add_listener_local()
                        .param(move |_, kind, _, _, pod| {
                            if let (ParamType::Route, Some(pod)) = (kind, pod)
                                && let Some(routes) = graph.borrow_mut().devices.get_mut(&id)
                            {
                                read_route(pod, routes);
                            }
                        })
                        .register();
                    device.subscribe_params(&[ParamType::Route]);
                    let routes = Routes {
                        device,
                        _listener: listener,
                        active: HashMap::new(),
                    };
                    global_graph.borrow_mut().devices.insert(object.id, routes);
                }
                ObjectType::Metadata if get("metadata.name") == "default" => {
                    let Ok(metadata) = registry.bind::<Metadata, _>(object) else {
                        return;
                    };
                    let (graph, report) = (global_graph.clone(), global_report.clone());
                    let listener = metadata
                        .add_listener_local()
                        .property(move |_, key, _, value| {
                            let name = value.and_then(default_name).unwrap_or("").to_owned();
                            {
                                let mut graph = graph.borrow_mut();
                                match key {
                                    Some("default.audio.sink") => graph.default_sink = name,
                                    Some("default.audio.source") => graph.default_source = name,
                                    _ => return 0,
                                }
                            }
                            report();
                            0
                        })
                        .register();
                    global_graph
                        .borrow_mut()
                        .metadata
                        .push((metadata, listener));
                }
                _ => {}
            }
        })
        .global_remove({
            let (graph, report) = (graph.clone(), report.clone());
            move |id| {
                graph.borrow_mut().devices.remove(&id);
                let removed = graph.borrow_mut().nodes.remove(&id).is_some();
                if removed {
                    report();
                }
            }
        })
        .register();

    let command_graph = graph.clone();
    let (peak_core, peak_report) = (core.clone(), report.clone());
    let peak = RefCell::new(None);
    let _commands = commands.attach(main_loop.loop_(), move |request| {
        if matches!(request, Command::Peak) {
            if peak.borrow().is_none() {
                let (graph, report) = (command_graph.clone(), peak_report.clone());
                let measured = crate::peak::capture(peak_core.clone(), move |level| {
                    graph.borrow_mut().peak = level;
                    report();
                });
                match measured {
                    Ok(capture) => *peak.borrow_mut() = Some(capture),
                    Err(e) => eprintln!("palettes: audio peaks: {e}"),
                }
            }
            return;
        }
        command(&command_graph.borrow(), request);
    });
    // The connection dropping (PipeWire restarting) ends the thread; the
    // shell keeps the last state it was told.
    let weak_loop = main_loop.downgrade();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _, _, _| {
            if id == pw::core::PW_ID_CORE
                && let Some(main_loop) = weak_loop.upgrade()
            {
                main_loop.quit();
            }
        })
        .register();
    let _ = ready.send(Ok(()));
    main_loop.run();
}
