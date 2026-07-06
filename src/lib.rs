//! yeli: a pure-Rust LV2 plugin host library
//!
//! Components:
//!   * `oxttl` + `oxrdf`   : Turtle parsing / RDF graph (bundle discovery)
//!   * `lv2-sys`           : the LV2 C ABI (descriptor, features, URID, options)
//!   * `libloading`        : loading plugin shared objects
//!
//! Supported host features:
//!   * http://lv2plug.in/ns/ext/urid#map
//!   * http://lv2plug.in/ns/ext/urid#unmap
//!   * http://lv2plug.in/ns/ext/options#options
//!   * http://lv2plug.in/ns/ext/buf-size#boundedBlockLength

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_void};
use std::fmt;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use oxrdf::{Graph, NamedNode, NamedNodeRef, NamedOrBlankNodeRef, TermRef};
use oxttl::TurtleParser;

pub mod uris {
    pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    pub const RDFS_SEE_ALSO: &str = "http://www.w3.org/2000/01/rdf-schema#seeAlso";
    pub const DOAP_NAME: &str = "http://usefulinc.com/ns/doap#name";

    pub const LV2_PLUGIN: &str = "http://lv2plug.in/ns/lv2core#Plugin";
    pub const LV2_BINARY: &str = "http://lv2plug.in/ns/lv2core#binary";
    pub const LV2_PORT: &str = "http://lv2plug.in/ns/lv2core#port";
    pub const LV2_INPUT_PORT: &str = "http://lv2plug.in/ns/lv2core#InputPort";
    pub const LV2_OUTPUT_PORT: &str = "http://lv2plug.in/ns/lv2core#OutputPort";
    pub const LV2_AUDIO_PORT: &str = "http://lv2plug.in/ns/lv2core#AudioPort";
    pub const LV2_CONTROL_PORT: &str = "http://lv2plug.in/ns/lv2core#ControlPort";
    pub const LV2_CV_PORT: &str = "http://lv2plug.in/ns/lv2core#CVPort";
    pub const LV2_INDEX: &str = "http://lv2plug.in/ns/lv2core#index";
    pub const LV2_SYMBOL: &str = "http://lv2plug.in/ns/lv2core#symbol";
    pub const LV2_NAME: &str = "http://lv2plug.in/ns/lv2core#name";
    pub const LV2_DEFAULT: &str = "http://lv2plug.in/ns/lv2core#default";
    pub const LV2_MINIMUM: &str = "http://lv2plug.in/ns/lv2core#minimum";
    pub const LV2_MAXIMUM: &str = "http://lv2plug.in/ns/lv2core#maximum";
    pub const LV2_REQUIRED_FEATURE: &str = "http://lv2plug.in/ns/lv2core#requiredFeature";
    pub const LV2_PORT_PROPERTY: &str = "http://lv2plug.in/ns/lv2core#portProperty";
    pub const LV2_CONNECTION_OPTIONAL: &str = "http://lv2plug.in/ns/lv2core#connectionOptional";

    pub const ATOM_PORT: &str = "http://lv2plug.in/ns/ext/atom#AtomPort";
    pub const ATOM_SEQUENCE: &str = "http://lv2plug.in/ns/ext/atom#Sequence";
    pub const ATOM_CHUNK: &str = "http://lv2plug.in/ns/ext/atom#Chunk";
    pub const ATOM_INT: &str = "http://lv2plug.in/ns/ext/atom#Int";

    pub const MIDI_EVENT: &str = "http://lv2plug.in/ns/ext/midi#MidiEvent";

    pub const URID_MAP: &str = "http://lv2plug.in/ns/ext/urid#map";
    pub const URID_UNMAP: &str = "http://lv2plug.in/ns/ext/urid#unmap";
    pub const OPTIONS_OPTIONS: &str = "http://lv2plug.in/ns/ext/options#options";
    pub const BUF_BOUNDED: &str = "http://lv2plug.in/ns/ext/buf-size#boundedBlockLength";
    pub const BUF_MIN_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#minBlockLength";
    pub const BUF_MAX_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#maxBlockLength";
    pub const BUF_NOMINAL_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#nominalBlockLength";
    pub const BUF_SEQ_SIZE: &str = "http://lv2plug.in/ns/ext/buf-size#sequenceSize";

    pub const UI_UI: &str = "http://lv2plug.in/ns/extensions/ui#ui";
    pub const UI_BINARY: &str = "http://lv2plug.in/ns/extensions/ui#binary";
    pub const UI_SHOWN_BY_DEFAULT: &str = "http://lv2plug.in/ns/extensions/ui#shownByDefault";
    pub const UI_X11_UI: &str = "http://lv2plug.in/ns/extensions/ui#X11UI";
    pub const UI_GTK_UI: &str = "http://lv2plug.in/ns/extensions/ui#GtkUI";
    pub const UI_GTK3_UI: &str = "http://lv2plug.in/ns/extensions/ui#Gtk3UI";
    pub const UI_SHOW_INTERFACE: &str = "http://lv2plug.in/ns/extensions/ui#showInterface";
    pub const UI_WINDOW_ID: &str = "http://lv2plug.in/ns/extensions/ui#windowId";
    pub const UI_PARENT: &str = "http://lv2plug.in/ns/extensions/ui#parent";
    pub const UI_IDLE_INTERFACE: &str = "http://lv2plug.in/ns/extensions/ui#idleInterface";

    pub const PARAM_SAMPLE_RATE: &str = "http://lv2plug.in/ns/ext/parameters#sampleRate";
    pub const ATOM_DOUBLE: &str = "http://lv2plug.in/ns/ext/atom#Double";
    pub const ATOM_FLOAT: &str = "http://lv2plug.in/ns/ext/atom#Float";

    pub const WORKER_SCHEDULE: &str = "http://lv2plug.in/ns/ext/worker#schedule";
    pub const WORKER_INTERFACE: &str = "http://lv2plug.in/ns/ext/worker#interface";
}

/// Features this host can supply to plugins.
pub const SUPPORTED_FEATURES: [&str; 6] = [
    uris::URID_MAP,
    uris::URID_UNMAP,
    uris::OPTIONS_OPTIONS,
    uris::BUF_BOUNDED,
    uris::WORKER_SCHEDULE,
    uris::WORKER_INTERFACE,
];

/// Capacity (bytes) of atom sequence port buffers.
pub const ATOM_SEQUENCE_CAPACITY: usize = 8192;

/// The number of ports by type.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PortCounts {
    pub control_inputs: usize,
    pub control_outputs: usize,
    pub audio_inputs: usize,
    pub audio_outputs: usize,
    pub atom_sequence_inputs: usize,
    pub atom_sequence_outputs: usize,
    pub cv_inputs: usize,
    pub cv_outputs: usize,
}

/// Combined port type (direction + data kind).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PortType {
    ControlInput,
    ControlOutput,
    AudioInput,
    AudioOutput,
    AtomSequenceInput,
    AtomSequenceOutput,
    CVInput,
    CVOutput,
}

/// The index of a port within a plugin.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortIndex(pub u32);

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse(String),
    MissingBinary(String),
    MissingFeature(String),
    PluginNotFound(String),
    Library(String),
    UnsupportedPort(String),
    Instantiation(String),
    BufferTooSmall,
    BlockTooLarge { requested: usize, max: usize },
    BlockTooSmall { requested: usize, min: usize },
    PortCountMismatch { expected: usize, actual: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Parse(e) => write!(f, "Turtle parse error: {e}"),
            Error::MissingBinary(u) => write!(f, "plugin {u} has no usable lv2:binary"),
            Error::MissingFeature(u) => write!(f, "plugin requires unsupported feature {u}"),
            Error::PluginNotFound(u) => write!(f, "plugin {u} not found"),
            Error::Library(e) => write!(f, "shared library error: {e}"),
            Error::UnsupportedPort(s) => write!(f, "unsupported port: {s}"),
            Error::Instantiation(u) => write!(f, "failed to instantiate {u}"),
            Error::BufferTooSmall => write!(f, "atom buffer too small"),
            Error::BlockTooLarge { requested, max } => {
                write!(f, "block of {requested} frames exceeds maximum {max}")
            }
            Error::BlockTooSmall { requested, min } => {
                write!(f, "block of {requested} frames is below minimum {min}")
            }
            Error::PortCountMismatch { expected, actual } => {
                write!(f, "expected {expected} ports but got {actual}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

fn path_to_file_uri(path: &Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = String::from("file://");
    for b in abs.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let raw = rest.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Builder for a shareable `Features` object.
#[derive(Clone, Debug)]
pub struct FeaturesBuilder {
    /// Minimum block size. `run()` calls with fewer samples will error.
    pub min_block_length: usize,
    /// Maximum block size. `run()` calls with more samples will error.
    pub max_block_length: usize,
}

impl Default for FeaturesBuilder {
    fn default() -> Self {
        FeaturesBuilder {
            min_block_length: 1,
            max_block_length: 4096,
        }
    }
}

/// Host features shared across plugin instances.
///
/// Provides URID mapping, block size constraints, and pre-resolved URIDs
/// for atom sequences and MIDI events.
pub struct Features {
    urid: Arc<UridMap>,
    min_block_length: usize,
    max_block_length: usize,
    atom_seq_urid: u32,
    atom_chunk_urid: u32,
    midi_urid: u32,
}

impl Features {
    /// The minimum allowed block length.
    pub fn min_block_length(&self) -> usize {
        self.min_block_length
    }

    /// The maximum allowed block length.
    pub fn max_block_length(&self) -> usize {
        self.max_block_length
    }

    /// The URID for the `midi:MidiEvent` URI.
    pub fn midi_urid(&self) -> u32 {
        self.midi_urid
    }

    /// Map a URI string to a URID.
    pub fn urid(&self, uri: &str) -> u32 {
        self.urid.map(uri)
    }

    /// The underlying URID map.
    pub fn urid_map(&self) -> &Arc<UridMap> {
        &self.urid
    }

    fn atom_seq_urid(&self) -> u32 {
        self.atom_seq_urid
    }

    fn atom_chunk_urid(&self) -> u32 {
        self.atom_chunk_urid
    }
}

struct UridState {
    by_uri: HashMap<CString, u32>,
    by_id: Vec<CString>,
}

/// Centralized URID map store, shared by all instances of a `World`.
pub struct UridMap {
    inner: Mutex<UridState>,
}

impl UridMap {
    fn new() -> Self {
        UridMap {
            inner: Mutex::new(UridState {
                by_uri: HashMap::new(),
                by_id: Vec::new(),
            }),
        }
    }

    pub fn map(&self, uri: &str) -> u32 {
        match CString::new(uri) {
            Ok(c) => self.map_c(&c),
            Err(_) => 0,
        }
    }

    fn map_c(&self, uri: &CStr) -> u32 {
        let mut st = self.inner.lock().unwrap();
        if let Some(&id) = st.by_uri.get(uri) {
            return id;
        }
        let id = (st.by_id.len() + 1) as u32;
        let owned = uri.to_owned();
        st.by_id.push(owned.clone());
        st.by_uri.insert(owned, id);
        id
    }

    fn map_ptr(&self, uri: *const c_char) -> u32 {
        if uri.is_null() {
            return 0;
        }
        self.map_c(unsafe { CStr::from_ptr(uri) })
    }

    pub fn unmap(&self, id: u32) -> Option<String> {
        let st = self.inner.lock().unwrap();
        st.by_id
            .get(id.checked_sub(1)? as usize)
            .map(|c| c.to_string_lossy().into_owned())
    }

    /// Pointer stays valid: CString buffers are heap allocations that never move.
    fn unmap_ptr(&self, id: u32) -> *const c_char {
        let st = self.inner.lock().unwrap();
        match id.checked_sub(1).and_then(|i| st.by_id.get(i as usize)) {
            Some(c) => c.as_ptr(),
            None => std::ptr::null(),
        }
    }
}

unsafe extern "C" fn urid_map_cb(
    handle: lv2_sys::LV2_URID_Map_Handle,
    uri: *const c_char,
) -> lv2_sys::LV2_URID {
    if handle.is_null() || uri.is_null() {
        return 0;
    }
    let map = unsafe { &*(handle as *const UridMap) };
    map.map_ptr(uri)
}

unsafe extern "C" fn urid_unmap_cb(
    handle: lv2_sys::LV2_URID_Unmap_Handle,
    urid: lv2_sys::LV2_URID,
) -> *const c_char {
    if handle.is_null() {
        return std::ptr::null();
    }
    let map = unsafe { &*(handle as *const UridMap) };
    map.unmap_ptr(urid)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PortKind {
    Audio,
    Control,
    Cv,
    AtomSequence,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Port {
    pub index: u32,
    pub symbol: String,
    pub name: String,
    pub direction: PortDirection,
    pub kind: PortKind,
    pub default: Option<f32>,
    pub minimum: Option<f32>,
    pub maximum: Option<f32>,
    pub optional: bool,
}

impl Port {
    /// Return the combined `PortType` (direction + kind).
    pub fn port_type(&self) -> PortType {
        match (self.direction, self.kind) {
            (PortDirection::Input, PortKind::Control) => PortType::ControlInput,
            (PortDirection::Output, PortKind::Control) => PortType::ControlOutput,
            (PortDirection::Input, PortKind::Audio) => PortType::AudioInput,
            (PortDirection::Output, PortKind::Audio) => PortType::AudioOutput,
            (PortDirection::Input, PortKind::AtomSequence) => PortType::AtomSequenceInput,
            (PortDirection::Output, PortKind::AtomSequence) => PortType::AtomSequenceOutput,
            (PortDirection::Input, PortKind::Cv) => PortType::CVInput,
            (PortDirection::Output, PortKind::Cv) => PortType::CVOutput,
            (_, PortKind::Unknown) => {
                if self.direction == PortDirection::Input {
                    PortType::ControlInput
                } else {
                    PortType::ControlOutput
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct UiPlugin {
    pub uri: String,
    pub ui_type: String,
    pub binary_path: PathBuf,
    pub shown_by_default: bool,
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub uri: String,
    pub name: String,
    pub bundle_path: PathBuf,
    pub binary_path: PathBuf,
    pub ports: Vec<Port>,
    pub required_features: Vec<String>,
    pub uis: Vec<UiPlugin>,
}

impl Plugin {
    /// Return a slice of all ports.
    pub fn ports(&self) -> &[Port] {
        &self.ports
    }

    /// The number of ports by type.
    pub fn port_counts(&self) -> PortCounts {
        let mut c = PortCounts::default();
        for p in &self.ports {
            match p.port_type() {
                PortType::ControlInput => c.control_inputs += 1,
                PortType::ControlOutput => c.control_outputs += 1,
                PortType::AudioInput => c.audio_inputs += 1,
                PortType::AudioOutput => c.audio_outputs += 1,
                PortType::AtomSequenceInput => c.atom_sequence_inputs += 1,
                PortType::AtomSequenceOutput => c.atom_sequence_outputs += 1,
                PortType::CVInput => c.cv_inputs += 1,
                PortType::CVOutput => c.cv_outputs += 1,
            }
        }
        c
    }

    /// Return `true` if the plugin is an instrument
    /// (has at least one atom-sequence input and at least one audio output).
    pub fn is_instrument(&self) -> bool {
        let mut has_atom_in = false;
        let mut has_audio_out = false;
        for p in &self.ports {
            match p.port_type() {
                PortType::AtomSequenceInput => has_atom_in = true,
                PortType::AudioOutput => has_audio_out = true,
                _ => {}
            }
        }
        has_atom_in && has_audio_out
    }
}

// -- small RDF helpers -------------------------------------------------------

fn nn(s: &str) -> NamedNodeRef<'_> {
    NamedNodeRef::new(s).expect("host-internal IRI is valid")
}

fn term_str(t: TermRef<'_>) -> Option<String> {
    match t {
        TermRef::Literal(l) => Some(l.value().to_string()),
        _ => None,
    }
}

fn term_f32(t: TermRef<'_>) -> Option<f32> {
    match t {
        TermRef::Literal(l) => l.value().trim().parse::<f32>().ok(),
        _ => None,
    }
}

fn term_u32(t: TermRef<'_>) -> Option<u32> {
    match t {
        TermRef::Literal(l) => l.value().trim().parse::<u32>().ok(),
        _ => None,
    }
}

fn parse_ttl_into(graph: &mut Graph, path: &Path) -> Result<(), Error> {
    let base = path_to_file_uri(path);
    let file = std::fs::File::open(path)?;
    let parser = TurtleParser::new()
        .with_base_iri(base)
        .map_err(|e| Error::Parse(e.to_string()))?;
    for triple in parser.for_reader(std::io::BufReader::new(file)) {
        let triple = triple.map_err(|e| Error::Parse(e.to_string()))?;
        graph.insert(triple.as_ref());
    }
    Ok(())
}

pub fn default_lv2_paths() -> Vec<PathBuf> {
    if let Ok(env) = std::env::var("LV2_PATH") {
        return env.split(':').map(PathBuf::from).collect();
    }
    let mut paths = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(home).join(".lv2"));
    }
    for p in [
        "/usr/local/lib/lv2",
        "/usr/lib/lv2",
        "/usr/lib64/lv2",
        "/usr/lib/x86_64-linux-gnu/lv2",
        "/Library/Audio/Plug-Ins/LV2",
    ] {
        paths.push(PathBuf::from(p));
    }
    paths
}

pub struct World {
    pub plugins: Vec<Plugin>,
    urid: Arc<UridMap>,
}

impl World {
    /// Discover all bundles on `$LV2_PATH` (or the standard directories).
    pub fn discover() -> Self {
        Self::with_paths(&default_lv2_paths())
    }

    pub fn with_paths(paths: &[PathBuf]) -> Self {
        let mut world = World {
            plugins: Vec::new(),
            urid: Arc::new(UridMap::new()),
        };
        let mut seen: HashSet<String> = HashSet::new();
        for dir in paths {
            let Ok(read_dir) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in read_dir.flatten() {
                let bundle = entry.path();
                if !bundle.is_dir() {
                    continue;
                }
                if let Err(e) = world.load_bundle(&bundle, &mut seen) {
                    eprintln!("warning: skipping bundle {}: {e}", bundle.display());
                }
            }
        }
        world.plugins.sort_by(|a, b| a.uri.cmp(&b.uri));
        world
    }

    pub fn urid_map(&self) -> &Arc<UridMap> {
        &self.urid
    }

    /// Build a shareable `Features` object from this world.
    pub fn build_features(&self, builder: FeaturesBuilder) -> Arc<Features> {
        let atom_seq_urid = self.urid.map(uris::ATOM_SEQUENCE);
        let atom_chunk_urid = self.urid.map(uris::ATOM_CHUNK);
        let midi_urid = self.urid.map(uris::MIDI_EVENT);
        Arc::new(Features {
            urid: self.urid.clone(),
            min_block_length: builder.min_block_length,
            max_block_length: builder.max_block_length,
            atom_seq_urid,
            atom_chunk_urid,
            midi_urid,
        })
    }

    /// Iterate through all discovered plugins.
    pub fn iter_plugins(&self) -> impl ExactSizeIterator<Item = &Plugin> {
        self.plugins.iter()
    }

    pub fn plugin_by_uri(&self, uri: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.uri == uri)
    }

    fn load_bundle(&mut self, bundle: &Path, seen: &mut HashSet<String>) -> Result<(), Error> {
        let manifest = bundle.join("manifest.ttl");
        if !manifest.is_file() {
            return Ok(());
        }
        let mut graph = Graph::default();
        parse_ttl_into(&mut graph, &manifest)?;

        // Subjects declared `a lv2:Plugin` in the manifest.
        let subjects: Vec<NamedNode> = graph
            .subjects_for_predicate_object(nn(uris::RDF_TYPE), nn(uris::LV2_PLUGIN))
            .filter_map(|s| match s {
                NamedOrBlankNodeRef::NamedNode(n) => Some(n.into_owned()),
                _ => None,
            })
            .collect();
        if subjects.is_empty() {
            return Ok(());
        }

        // Pull in rdfs:seeAlso data files (e.g. the real <plugin>.ttl).
        let mut extra_files: Vec<PathBuf> = Vec::new();
        for s in &subjects {
            for obj in graph.objects_for_subject_predicate(s.as_ref(), nn(uris::RDFS_SEE_ALSO)) {
                if let TermRef::NamedNode(n) = obj
                    && let Some(p) = file_uri_to_path(n.as_str())
                {
                    extra_files.push(p);
                }
            }
        }
        for f in extra_files {
            if let Err(e) = parse_ttl_into(&mut graph, &f) {
                eprintln!("warning: {}: {e}", f.display());
            }
        }

        for s in &subjects {
            if seen.contains(s.as_str()) {
                continue; // same plugin found earlier on the search path
            }
            match build_plugin(&graph, s, bundle) {
                Ok(p) => {
                    seen.insert(p.uri.clone());
                    self.plugins.push(p);
                }
                Err(e) => eprintln!("warning: {}: {e}", s.as_str()),
            }
        }
        Ok(())
    }

    /// Instantiate a plugin with default features (min_block=1).
    pub fn instantiate(
        &self,
        plugin: &Plugin,
        sample_rate: f64,
        max_block: usize,
    ) -> Result<Instance, Error> {
        let features = self.build_features(FeaturesBuilder {
            min_block_length: 1,
            max_block_length: max_block,
        });
        self.instantiate_with_features(plugin, sample_rate, &features)
    }

    /// Instantiate a plugin with the given shareable `Features`.
    pub fn instantiate_with_features(
        &self,
        plugin: &Plugin,
        sample_rate: f64,
        features: &Features,
    ) -> Result<Instance, Error> {
        // 1. Feature check.
        for f in &plugin.required_features {
            if !SUPPORTED_FEATURES.contains(&f.as_str()) {
                return Err(Error::MissingFeature(f.clone()));
            }
        }
        // 2. Port support check (before touching any C code).
        for port in &plugin.ports {
            if port.kind == PortKind::Unknown && !port.optional {
                return Err(Error::UnsupportedPort(format!(
                    "{} (port '{}', index {})",
                    plugin.uri, port.symbol, port.index
                )));
            }
        }

        let max_block = features.max_block_length();
        let min_block = features.min_block_length();

        // 3. Build the C feature array (heap-pinned; must outlive the instance).
        let mut c_features = build_c_features(
            &self.urid,
            min_block as i32,
            max_block as i32,
            ATOM_SEQUENCE_CAPACITY as i32,
        );

        // 4. Load the shared object and find the matching descriptor.
        let library = unsafe { libloading::Library::new(&plugin.binary_path) }
            .map_err(|e| Error::Library(e.to_string()))?;
        let descriptor: *const lv2_sys::LV2_Descriptor = unsafe {
            let entry: libloading::Symbol<
                unsafe extern "C" fn(u32) -> *const lv2_sys::LV2_Descriptor,
            > = library
                .get(b"lv2_descriptor\0")
                .map_err(|e| Error::Library(e.to_string()))?;
            let mut i: u32 = 0;
            loop {
                let d = entry(i);
                if d.is_null() {
                    return Err(Error::PluginNotFound(plugin.uri.clone()));
                }
                if CStr::from_ptr((*d).URI).to_bytes() == plugin.uri.as_bytes() {
                    break d;
                }
                i += 1;
            }
        };

        // 5. Instantiate.
        let mut bundle = plugin.bundle_path.to_string_lossy().into_owned();
        if !bundle.ends_with('/') {
            bundle.push('/');
        }
        let bundle_c =
            CString::new(bundle).map_err(|_| Error::Instantiation(plugin.uri.clone()))?;
        let instantiate = unsafe { (*descriptor).instantiate }
            .ok_or_else(|| Error::Instantiation(plugin.uri.clone()))?;
        let handle = unsafe {
            instantiate(
                descriptor,
                sample_rate,
                bundle_c.as_ptr(),
                c_features.feature_ptrs.as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(Error::Instantiation(plugin.uri.clone()));
        }

        // 5a. Check for worker interface extension and start worker thread.
        let worker_runtime = if let Some(ext_data) = unsafe { (*descriptor).extension_data } {
            let worker_iface_uri = CString::new(uris::WORKER_INTERFACE).expect("valid CString");
            let iface_ptr = unsafe { ext_data(worker_iface_uri.as_ptr()) };
            if !iface_ptr.is_null() {
                let iface = unsafe { &*(iface_ptr as *const lv2_sys::LV2_Worker_Interface) };
                let runtime = WorkerRuntime::new(iface, handle);
                let shared_ptr = Arc::as_ptr(&runtime.shared) as *mut std::ffi::c_void;
                c_features.schedule.handle = shared_ptr;
                Some(runtime)
            } else {
                None
            }
        } else {
            None
        };

        // 6. Allocate and connect port buffers.
        let seq_urid = self.urid.map(uris::ATOM_SEQUENCE);
        let chunk_urid = self.urid.map(uris::ATOM_CHUNK);
        let midi_urid = self.urid.map(uris::MIDI_EVENT);

        let mut buffers: Vec<PortBuffer> = plugin
            .ports
            .iter()
            .map(|port| match (port.kind, port.direction) {
                (PortKind::Control, _) => {
                    PortBuffer::Control(Box::new(port.default.or(port.minimum).unwrap_or(0.0)))
                }
                (PortKind::Audio | PortKind::Cv, _) => PortBuffer::Audio(vec![0.0f32; max_block]),
                (PortKind::AtomSequence, PortDirection::Input) => PortBuffer::AtomIn(
                    AtomSequence::new(ATOM_SEQUENCE_CAPACITY, seq_urid, chunk_urid, true),
                ),
                (PortKind::AtomSequence, PortDirection::Output) => PortBuffer::AtomOut(
                    AtomSequence::new(ATOM_SEQUENCE_CAPACITY, seq_urid, chunk_urid, false),
                ),
                (PortKind::Unknown, _) => PortBuffer::Unconnected,
            })
            .collect();

        let connect = unsafe { (*descriptor).connect_port }
            .ok_or_else(|| Error::Instantiation(plugin.uri.clone()))?;
        for (port, buf) in plugin.ports.iter().zip(buffers.iter_mut()) {
            let ptr: *mut c_void = match buf {
                PortBuffer::Control(v) => (&mut **v) as *mut f32 as *mut c_void,
                PortBuffer::Audio(v) => v.as_mut_ptr() as *mut c_void,
                PortBuffer::AtomIn(s) | PortBuffer::AtomOut(s) => s.as_mut_ptr(),
                PortBuffer::Unconnected => std::ptr::null_mut(),
            };
            unsafe { connect(handle, port.index, ptr) };
        }

        // 7. Build port index mappings for run_with_ports.
        let mut audio_input_indices = Vec::new();
        let mut audio_output_indices = Vec::new();
        let mut atom_input_indices = Vec::new();
        let mut atom_output_indices = Vec::new();
        let mut control_input_map = HashMap::new();
        let mut control_output_map = HashMap::new();

        for (buf_idx, port) in plugin.ports.iter().enumerate() {
            match port.port_type() {
                PortType::AudioInput => audio_input_indices.push(port.index),
                PortType::AudioOutput => audio_output_indices.push(port.index),
                PortType::AtomSequenceInput => atom_input_indices.push(port.index),
                PortType::AtomSequenceOutput => atom_output_indices.push(port.index),
                PortType::ControlInput => {
                    control_input_map.insert(port.index, buf_idx);
                }
                PortType::ControlOutput => {
                    control_output_map.insert(port.index, buf_idx);
                }
                _ => {}
            }
        }

        let port_counts = plugin.port_counts();

        let bundle = plugin.bundle_path.to_string_lossy().into_owned();
        let bundle_str = if bundle.ends_with('/') {
            bundle
        } else {
            format!("{bundle}/")
        };

        Ok(Instance {
            handle,
            descriptor,
            active: false,
            ports: plugin.ports.clone(),
            buffers,
            _features: c_features,
            worker_runtime,
            _urid: self.urid.clone(),
            midi_urid,
            sample_rate,
            max_block,
            min_block,
            port_counts,
            audio_input_indices,
            audio_output_indices,
            atom_input_indices,
            atom_output_indices,
            control_input_map,
            control_output_map,
            _library: library,
            plugin_uri: plugin.uri.clone(),
            bundle_path: bundle_str,
            uis: plugin.uis.clone(),
        })
    }
}

fn build_plugin(graph: &Graph, subject: &NamedNode, bundle: &Path) -> Result<Plugin, Error> {
    let s = subject.as_ref();

    let binary_path = graph
        .object_for_subject_predicate(s, nn(uris::LV2_BINARY))
        .and_then(|t| match t {
            TermRef::NamedNode(n) => file_uri_to_path(n.as_str()),
            _ => None,
        })
        .ok_or_else(|| Error::MissingBinary(subject.as_str().to_string()))?;

    let name = graph
        .object_for_subject_predicate(s, nn(uris::DOAP_NAME))
        .and_then(term_str)
        .unwrap_or_else(|| subject.as_str().to_string());

    let required_features: Vec<String> = graph
        .objects_for_subject_predicate(s, nn(uris::LV2_REQUIRED_FEATURE))
        .filter_map(|t| match t {
            TermRef::NamedNode(n) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect();

    let mut ports = Vec::new();
    for term in graph.objects_for_subject_predicate(s, nn(uris::LV2_PORT)) {
        let ps: NamedOrBlankNodeRef = match term {
            TermRef::NamedNode(n) => n.into(),
            TermRef::BlankNode(b) => b.into(),
            _ => continue,
        };
        let Some(index) = graph
            .object_for_subject_predicate(ps, nn(uris::LV2_INDEX))
            .and_then(term_u32)
        else {
            continue;
        };
        let symbol = graph
            .object_for_subject_predicate(ps, nn(uris::LV2_SYMBOL))
            .and_then(term_str)
            .unwrap_or_else(|| format!("port_{index}"));
        let pname = graph
            .object_for_subject_predicate(ps, nn(uris::LV2_NAME))
            .and_then(term_str)
            .unwrap_or_else(|| symbol.clone());

        let classes: Vec<String> = graph
            .objects_for_subject_predicate(ps, nn(uris::RDF_TYPE))
            .filter_map(|t| match t {
                TermRef::NamedNode(n) => Some(n.as_str().to_string()),
                _ => None,
            })
            .collect();
        let has = |c: &str| classes.iter().any(|x| x == c);

        let (direction, mut kind) = {
            let dir = if has(uris::LV2_INPUT_PORT) {
                Some(PortDirection::Input)
            } else if has(uris::LV2_OUTPUT_PORT) {
                Some(PortDirection::Output)
            } else {
                None
            };
            let kind = if has(uris::LV2_AUDIO_PORT) {
                PortKind::Audio
            } else if has(uris::LV2_CONTROL_PORT) {
                PortKind::Control
            } else if has(uris::LV2_CV_PORT) {
                PortKind::Cv
            } else if has(uris::ATOM_PORT) {
                PortKind::AtomSequence
            } else {
                PortKind::Unknown
            };
            (dir.unwrap_or(PortDirection::Input), kind)
        };
        if !has(uris::LV2_INPUT_PORT) && !has(uris::LV2_OUTPUT_PORT) {
            kind = PortKind::Unknown;
        }

        let optional = graph
            .objects_for_subject_predicate(ps, nn(uris::LV2_PORT_PROPERTY))
            .any(|t| matches!(t, TermRef::NamedNode(n) if n.as_str() == uris::LV2_CONNECTION_OPTIONAL));

        let get_f = |p: &str| {
            graph
                .object_for_subject_predicate(ps, nn(p))
                .and_then(term_f32)
        };

        ports.push(Port {
            index,
            symbol,
            name: pname,
            direction,
            kind,
            default: get_f(uris::LV2_DEFAULT),
            minimum: get_f(uris::LV2_MINIMUM),
            maximum: get_f(uris::LV2_MAXIMUM),
            optional,
        });
    }
    ports.sort_by_key(|p| p.index);

    let mut uis = Vec::new();
    for ui_term in graph.objects_for_subject_predicate(s, nn(uris::UI_UI)) {
        let ui_node: NamedOrBlankNodeRef = match ui_term {
            TermRef::NamedNode(n) => n.into(),
            TermRef::BlankNode(b) => b.into(),
            _ => continue,
        };
        let ui_uri = match ui_term {
            TermRef::NamedNode(n) => n.as_str().to_string(),
            _ => String::new(),
        };
        let ui_type = graph
            .objects_for_subject_predicate(ui_node, nn(uris::RDF_TYPE))
            .filter_map(|t| match t {
                TermRef::NamedNode(n) => Some(n.as_str()),
                _ => None,
            })
            .find(|u| u == &uris::UI_X11_UI || u == &uris::UI_GTK_UI || u == &uris::UI_GTK3_UI)
            .map(|s| s.to_string());
        let Some(ui_type) = ui_type else {
            continue;
        };
        let binary_path = graph
            .object_for_subject_predicate(ui_node, nn(uris::UI_BINARY))
            .and_then(|t| match t {
                TermRef::NamedNode(n) => file_uri_to_path(n.as_str()),
                _ => None,
            })
            .unwrap_or_else(|| bundle.join("ui.so"));
        let shown_by_default = graph
            .object_for_subject_predicate(ui_node, nn(uris::UI_SHOWN_BY_DEFAULT))
            .and_then(term_str)
            .map(|s| s == "true")
            .unwrap_or(false);
        uis.push(UiPlugin {
            uri: ui_uri,
            ui_type,
            binary_path,
            shown_by_default,
        });
    }

    Ok(Plugin {
        uri: subject.as_str().to_string(),
        name,
        bundle_path: bundle.to_path_buf(),
        binary_path,
        ports,
        required_features,
        uis,
    })
}

/// Shared state accessed by the schedule callback (from the audio thread)
/// and the worker thread.
struct WorkerShared {
    pending: Mutex<Vec<Vec<u8>>>,
    completed: Mutex<Vec<(u32, Vec<u8>)>>,
    waker: Condvar,
    interface: Mutex<lv2_sys::LV2_Worker_Interface>,
    handle: Mutex<lv2_sys::LV2_Handle>,
    running: AtomicBool,
}

// Raw pointers inside Mutex are safe to send/sync because we only
// access them under the lock.
unsafe impl Send for WorkerShared {}
unsafe impl Sync for WorkerShared {}

/// The `handle` parameter of `respond` is the `LV2_Worker_Respond_Handle` that
/// was passed as the third argument to `work()`.  Plugins MUST pass this
/// handle back when calling `respond`, so we can recover the `WorkerShared`.
unsafe extern "C" fn worker_respond_cb(
    handle: lv2_sys::LV2_Worker_Respond_Handle,
    size: u32,
    data: *const std::ffi::c_void,
) -> lv2_sys::LV2_Worker_Status {
    if handle.is_null() || data.is_null() {
        return lv2_sys::LV2_Worker_Status_LV2_WORKER_ERR_UNKNOWN;
    }
    let shared = unsafe { &*(handle as *const WorkerShared) };
    let slice = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
    if let Ok(mut completed) = shared.completed.lock() {
        completed.push((size, slice.to_vec()));
        lv2_sys::LV2_Worker_Status_LV2_WORKER_SUCCESS
    } else {
        lv2_sys::LV2_Worker_Status_LV2_WORKER_ERR_UNKNOWN
    }
}

unsafe extern "C" fn worker_schedule_cb(
    handle: lv2_sys::LV2_Worker_Schedule_Handle,
    size: u32,
    data: *const std::ffi::c_void,
) -> lv2_sys::LV2_Worker_Status {
    if handle.is_null() || data.is_null() {
        return lv2_sys::LV2_Worker_Status_LV2_WORKER_ERR_UNKNOWN;
    }
    let shared = unsafe { &*(handle as *const WorkerShared) };
    let slice = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
    if let Ok(mut pending) = shared.pending.lock() {
        pending.push(slice.to_vec());
        shared.waker.notify_one();
        lv2_sys::LV2_Worker_Status_LV2_WORKER_SUCCESS
    } else {
        lv2_sys::LV2_Worker_Status_LV2_WORKER_ERR_UNKNOWN
    }
}

/// Background worker that processes plugin work requests.
struct WorkerRuntime {
    shared: Arc<WorkerShared>,
    thread: Option<thread::JoinHandle<()>>,
}

impl WorkerRuntime {
    fn new(iface: &lv2_sys::LV2_Worker_Interface, plugin_handle: lv2_sys::LV2_Handle) -> Self {
        let shared = Arc::new(WorkerShared {
            pending: Mutex::new(Vec::new()),
            completed: Mutex::new(Vec::new()),
            waker: Condvar::new(),
            interface: Mutex::new(*iface),
            handle: Mutex::new(plugin_handle),
            running: AtomicBool::new(true),
        });

        let thread_shared = shared.clone();
        let thread = thread::Builder::new()
            .name("lv2-worker".into())
            .spawn(move || Self::thread_main(thread_shared))
            .ok();

        Self { shared, thread }
    }

    fn thread_main(shared: Arc<WorkerShared>) {
        let mut pending_owned = Vec::new();
        loop {
            let mut pending = match shared.pending.lock() {
                Ok(g) => {
                    if g.is_empty() {
                        match shared.waker.wait(g) {
                            Ok(g) => g,
                            Err(_) => break,
                        }
                    } else {
                        g
                    }
                }
                Err(_) => break,
            };

            if !shared.running.load(Ordering::Acquire) {
                break;
            }

            pending_owned.clear();
            std::mem::swap(&mut *pending, &mut pending_owned);
            drop(pending);

            let iface = match shared.interface.lock() {
                Ok(g) => *g,
                Err(_) => break,
            };
            let plugin_handle = match shared.handle.lock() {
                Ok(g) => *g,
                Err(_) => break,
            };

            // The respond handle is a pointer to WorkerShared so the
            // respond callback can recover it without a thread-local.
            let respond_handle =
                Arc::as_ptr(&shared) as *const WorkerShared as *mut std::ffi::c_void;

            for data in &pending_owned {
                if let Some(work) = iface.work {
                    unsafe {
                        work(
                            plugin_handle,
                            Some(worker_respond_cb),
                            respond_handle,
                            data.len() as u32,
                            data.as_ptr() as *const std::ffi::c_void,
                        );
                    }
                }
            }
        }
    }

    /// Drain completed responses and deliver to the plugin via work_response().
    fn deliver_responses(&self, handle: lv2_sys::LV2_Handle) {
        let iface = match self.shared.interface.lock() {
            Ok(g) => *g,
            Err(_) => return,
        };
        let completed = match self.shared.completed.lock() {
            Ok(mut g) => {
                let mut ready = Vec::new();
                std::mem::swap(&mut *g, &mut ready);
                ready
            }
            Err(_) => return,
        };
        for (size, data) in &completed {
            if let Some(rsp) = iface.work_response {
                unsafe {
                    rsp(handle, *size, data.as_ptr() as *const std::ffi::c_void);
                }
            }
        }
    }

    fn end_run(&self, handle: lv2_sys::LV2_Handle) {
        if let Ok(iface) = self.shared.interface.lock()
            && let Some(end) = iface.end_run
        {
            unsafe { end(handle) };
        }
    }

    fn shutdown(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        self.shared.waker.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

unsafe impl Send for WorkerRuntime {}
unsafe impl Sync for WorkerRuntime {}

struct InstanceFeatures {
    _urid: Arc<UridMap>,
    map: lv2_sys::LV2_URID_Map,
    unmap: lv2_sys::LV2_URID_Unmap,
    schedule: lv2_sys::LV2_Worker_Schedule,
    min_block: i32,
    max_block: i32,
    nominal_block: i32,
    seq_size: i32,
    options: Vec<lv2_sys::LV2_Options_Option>,
    uris_c: Vec<CString>,
    features: Vec<lv2_sys::LV2_Feature>,
    feature_ptrs: Vec<*const lv2_sys::LV2_Feature>,
}

fn opt(key: u32, type_: u32, value: *const c_void) -> lv2_sys::LV2_Options_Option {
    lv2_sys::LV2_Options_Option {
        context: 0, // LV2_OPTIONS_INSTANCE
        subject: 0,
        key,
        size: std::mem::size_of::<i32>() as u32,
        type_,
        value,
    }
}

fn build_c_features(
    urid: &Arc<UridMap>,
    min_block: i32,
    max_block: i32,
    seq_size: i32,
) -> Box<InstanceFeatures> {
    let mut f = Box::new(InstanceFeatures {
        _urid: urid.clone(),
        map: lv2_sys::LV2_URID_Map {
            handle: std::ptr::null_mut(),
            map: Some(urid_map_cb),
        },
        unmap: lv2_sys::LV2_URID_Unmap {
            handle: std::ptr::null_mut(),
            unmap: Some(urid_unmap_cb),
        },
        schedule: lv2_sys::LV2_Worker_Schedule {
            handle: std::ptr::null_mut(),
            schedule_work: Some(worker_schedule_cb),
        },
        min_block,
        max_block,
        nominal_block: max_block,
        seq_size,
        options: Vec::new(),
        uris_c: Vec::new(),
        features: Vec::new(),
        feature_ptrs: Vec::new(),
    });

    let handle = Arc::as_ptr(&f._urid) as *mut c_void;
    f.map.handle = handle;
    f.unmap.handle = handle;

    let atom_int = urid.map(uris::ATOM_INT);
    let p_min = &f.min_block as *const i32 as *const c_void;
    let p_max = &f.max_block as *const i32 as *const c_void;
    let p_nom = &f.nominal_block as *const i32 as *const c_void;
    let p_seq = &f.seq_size as *const i32 as *const c_void;
    f.options = vec![
        opt(urid.map(uris::BUF_MIN_BLOCK), atom_int, p_min),
        opt(urid.map(uris::BUF_MAX_BLOCK), atom_int, p_max),
        opt(urid.map(uris::BUF_NOMINAL_BLOCK), atom_int, p_nom),
        opt(urid.map(uris::BUF_SEQ_SIZE), atom_int, p_seq),
        lv2_sys::LV2_Options_Option {
            context: 0,
            subject: 0,
            key: 0,
            size: 0,
            type_: 0,
            value: std::ptr::null(),
        },
    ];

    f.uris_c = vec![
        CString::new(uris::URID_MAP).unwrap(),
        CString::new(uris::URID_UNMAP).unwrap(),
        CString::new(uris::OPTIONS_OPTIONS).unwrap(),
        CString::new(uris::BUF_BOUNDED).unwrap(),
        CString::new(uris::WORKER_SCHEDULE).unwrap(),
    ];
    let p_map = &mut f.map as *mut lv2_sys::LV2_URID_Map as *mut c_void;
    let p_unmap = &mut f.unmap as *mut lv2_sys::LV2_URID_Unmap as *mut c_void;
    let p_opts = f.options.as_ptr() as *mut c_void;
    let p_sched = &mut f.schedule as *mut lv2_sys::LV2_Worker_Schedule as *mut c_void;
    f.features = vec![
        lv2_sys::LV2_Feature {
            URI: f.uris_c[0].as_ptr(),
            data: p_map,
        },
        lv2_sys::LV2_Feature {
            URI: f.uris_c[1].as_ptr(),
            data: p_unmap,
        },
        lv2_sys::LV2_Feature {
            URI: f.uris_c[2].as_ptr(),
            data: p_opts,
        },
        lv2_sys::LV2_Feature {
            URI: f.uris_c[3].as_ptr(),
            data: std::ptr::null_mut(),
        },
        lv2_sys::LV2_Feature {
            URI: f.uris_c[4].as_ptr(),
            data: p_sched,
        },
    ];
    f.feature_ptrs = f
        .features
        .iter()
        .map(|feat| feat as *const lv2_sys::LV2_Feature)
        .collect();
    f.feature_ptrs.push(std::ptr::null());
    f
}

/// One event read out of / written into an atom sequence.
pub struct AtomEvent<'a> {
    pub frames: i64,
    pub type_urid: u32,
    pub data: &'a [u8],
}

/// An LV2 atom:Sequence buffer with correct C layout and 64-bit alignment.
///
/// Layout: `LV2_Atom{u32 size,u32 type}` + `body{u32 unit,u32 pad}` + events,
/// each event being `i64 frames` + `LV2_Atom{u32 size,u32 type}` + data,
/// padded to 8 bytes.
pub struct AtomSequence {
    buf: Vec<u64>,
    body_capacity: usize,
    events_bytes: usize,
    seq_urid: u32,
    chunk_urid: u32,
    input: bool,
}

const SEQ_HEADER: usize = 16;

impl AtomSequence {
    /// Create a new atom sequence with the given capacity (bytes for event body).
    /// Supply the pre-resolved URIDs for `atom:Sequence` and `atom:Chunk`.
    /// When `input` is true the header is set as a sequence; when false it is
    /// set as a chunk (output).
    pub fn new(body_capacity: usize, seq_urid: u32, chunk_urid: u32, input: bool) -> Self {
        let total = SEQ_HEADER + body_capacity;
        let mut s = AtomSequence {
            buf: vec![0u64; total.div_ceil(8)],
            body_capacity,
            events_bytes: 0,
            seq_urid,
            chunk_urid,
            input,
        };
        if input {
            s.write_input_header();
        } else {
            s.prepare_output();
        }
        s
    }

    /// Create a new atom sequence using pre-resolved URIDs from `Features`.
    pub fn with_features(body_capacity: usize, features: &Features, input: bool) -> Self {
        Self::new(
            body_capacity,
            features.atom_seq_urid(),
            features.atom_chunk_urid(),
            input,
        )
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr() as *const u8, self.buf.len() * 8) }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe {
            std::slice::from_raw_parts_mut(self.buf.as_mut_ptr() as *mut u8, self.buf.len() * 8)
        }
    }

    fn put_u32(&mut self, off: usize, v: u32) {
        self.bytes_mut()[off..off + 4].copy_from_slice(&v.to_ne_bytes());
    }

    fn put_i64(&mut self, off: usize, v: i64) {
        self.bytes_mut()[off..off + 8].copy_from_slice(&v.to_ne_bytes());
    }

    fn write_input_header(&mut self) {
        let size = (8 + self.events_bytes) as u32;
        let seq = self.seq_urid;
        self.put_u32(0, size);
        self.put_u32(4, seq);
        self.put_u32(8, 0); // unit: frames
        self.put_u32(12, 0); // pad
    }

    /// Host-side preparation of an output sequence before `run()`:
    /// set atom.size to the buffer capacity and type to atom:Chunk.
    pub fn prepare_output(&mut self) {
        let cap = self.body_capacity as u32;
        let chunk = self.chunk_urid;
        self.put_u32(0, cap);
        self.put_u32(4, chunk);
    }

    /// Clear all events and designate the sequence as a chunk (output).
    /// Equivalent to livi's `clear_as_chunk`.
    pub fn clear_as_chunk(&mut self) {
        self.events_bytes = 0;
        self.prepare_output();
    }

    /// Remove all events (input sequences).
    pub fn clear(&mut self) {
        self.events_bytes = 0;
        if self.input {
            self.write_input_header();
        }
    }

    /// Append an event (e.g. a raw MIDI message) at frame time `frames`.
    pub fn push_event(&mut self, frames: i64, type_urid: u32, data: &[u8]) -> Result<(), Error> {
        let pad = (8 - data.len() % 8) % 8;
        let needed = 16 + data.len() + pad;
        if self.events_bytes + needed > self.body_capacity {
            return Err(Error::BufferTooSmall);
        }
        let off = SEQ_HEADER + self.events_bytes;
        self.put_i64(off, frames);
        self.put_u32(off + 8, data.len() as u32);
        self.put_u32(off + 12, type_urid);
        self.bytes_mut()[off + 16..off + 16 + data.len()].copy_from_slice(data);
        for b in &mut self.bytes_mut()[off + 16 + data.len()..off + needed] {
            *b = 0;
        }
        self.events_bytes += needed;
        self.write_input_header();
        Ok(())
    }

    /// Iterate events (useful for reading plugin MIDI/atom outputs after run).
    pub fn events(&self) -> Vec<AtomEvent<'_>> {
        let bytes = self.bytes();
        let size = u32::from_ne_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let end = (8 + size).min(bytes.len());
        let mut out = Vec::new();
        let mut off = SEQ_HEADER;
        while off + 16 <= end {
            let frames = i64::from_ne_bytes(bytes[off..off + 8].try_into().unwrap());
            let sz = u32::from_ne_bytes(bytes[off + 8..off + 12].try_into().unwrap()) as usize;
            let ty = u32::from_ne_bytes(bytes[off + 12..off + 16].try_into().unwrap());
            if off + 16 + sz > end {
                break;
            }
            out.push(AtomEvent {
                frames,
                type_urid: ty,
                data: &bytes[off + 16..off + 16 + sz],
            });
            off += 16 + sz + ((8 - sz % 8) % 8);
        }
        out
    }

    /// Constant pointer to the underlying C-compatible buffer.
    pub fn as_ptr(&self) -> *const c_void {
        self.buf.as_ptr() as *const c_void
    }

    /// Mutable pointer to the underlying C-compatible buffer.
    pub fn as_mut_ptr(&mut self) -> *mut c_void {
        self.buf.as_mut_ptr() as *mut c_void
    }
}

/// A builder for port connections passed to `Instance::run_with_ports`.
pub struct PortConnections<'a, AI, AO, ASI, ASO>
where
    AI: ExactSizeIterator + Iterator<Item = &'a [f32]>,
    AO: ExactSizeIterator + Iterator<Item = &'a mut [f32]>,
    ASI: ExactSizeIterator + Iterator<Item = &'a AtomSequence>,
    ASO: ExactSizeIterator + Iterator<Item = &'a mut AtomSequence>,
{
    /// Audio input buffers.
    pub audio_inputs: AI,
    /// Audio output buffers.
    pub audio_outputs: AO,
    /// Atom-sequence input buffers.
    pub atom_sequence_inputs: ASI,
    /// Atom-sequence output buffers.
    pub atom_sequence_outputs: ASO,
}

/// A `PortConnections` with no connections.
pub type EmptyPortConnections = PortConnections<
    'static,
    std::iter::Empty<&'static [f32]>,
    std::iter::Empty<&'static mut [f32]>,
    std::iter::Empty<&'static AtomSequence>,
    std::iter::Empty<&'static mut AtomSequence>,
>;

impl EmptyPortConnections {
    /// Create a new `PortConnections` object without any connections.
    pub fn new() -> EmptyPortConnections {
        EmptyPortConnections {
            audio_inputs: std::iter::empty(),
            audio_outputs: std::iter::empty(),
            atom_sequence_inputs: std::iter::empty(),
            atom_sequence_outputs: std::iter::empty(),
        }
    }
}

impl Default for EmptyPortConnections {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, AI, AO, ASI, ASO> PortConnections<'a, AI, AO, ASI, ASO>
where
    AI: ExactSizeIterator + Iterator<Item = &'a [f32]>,
    AO: ExactSizeIterator + Iterator<Item = &'a mut [f32]>,
    ASI: ExactSizeIterator + Iterator<Item = &'a AtomSequence>,
    ASO: ExactSizeIterator + Iterator<Item = &'a mut AtomSequence>,
{
    /// Add audio input buffers.
    pub fn with_audio_inputs<I>(self, audio_inputs: I) -> PortConnections<'a, I, AO, ASI, ASO>
    where
        I: ExactSizeIterator + Iterator<Item = &'a [f32]>,
    {
        PortConnections {
            audio_inputs,
            audio_outputs: self.audio_outputs,
            atom_sequence_inputs: self.atom_sequence_inputs,
            atom_sequence_outputs: self.atom_sequence_outputs,
        }
    }

    /// Add audio output buffers.
    pub fn with_audio_outputs<I>(self, audio_outputs: I) -> PortConnections<'a, AI, I, ASI, ASO>
    where
        I: ExactSizeIterator + Iterator<Item = &'a mut [f32]>,
    {
        PortConnections {
            audio_inputs: self.audio_inputs,
            audio_outputs,
            atom_sequence_inputs: self.atom_sequence_inputs,
            atom_sequence_outputs: self.atom_sequence_outputs,
        }
    }

    /// Add atom-sequence input buffers.
    pub fn with_atom_sequence_inputs<I>(
        self,
        atom_sequence_inputs: I,
    ) -> PortConnections<'a, AI, AO, I, ASO>
    where
        I: ExactSizeIterator + Iterator<Item = &'a AtomSequence>,
    {
        PortConnections {
            audio_inputs: self.audio_inputs,
            audio_outputs: self.audio_outputs,
            atom_sequence_inputs,
            atom_sequence_outputs: self.atom_sequence_outputs,
        }
    }

    /// Add atom-sequence output buffers.
    pub fn with_atom_sequence_outputs<I>(
        self,
        atom_sequence_outputs: I,
    ) -> PortConnections<'a, AI, AO, ASI, I>
    where
        I: ExactSizeIterator + Iterator<Item = &'a mut AtomSequence>,
    {
        PortConnections {
            audio_inputs: self.audio_inputs,
            audio_outputs: self.audio_outputs,
            atom_sequence_inputs: self.atom_sequence_inputs,
            atom_sequence_outputs,
        }
    }
}

enum PortBuffer {
    Control(Box<f32>),
    Audio(Vec<f32>),
    AtomIn(AtomSequence),
    AtomOut(AtomSequence),
    Unconnected,
}

/// A live plugin instance with owned, connected port buffers.
pub struct Instance {
    handle: lv2_sys::LV2_Handle,
    descriptor: *const lv2_sys::LV2_Descriptor,
    active: bool,
    ports: Vec<Port>,
    buffers: Vec<PortBuffer>,
    _features: Box<InstanceFeatures>,
    worker_runtime: Option<WorkerRuntime>,
    _urid: Arc<UridMap>,
    midi_urid: u32,
    sample_rate: f64,
    min_block: usize,
    max_block: usize,
    port_counts: PortCounts,
    audio_input_indices: Vec<u32>,
    audio_output_indices: Vec<u32>,
    atom_input_indices: Vec<u32>,
    atom_output_indices: Vec<u32>,
    control_input_map: HashMap<u32, usize>,
    control_output_map: HashMap<u32, usize>,
    _library: libloading::Library,
    plugin_uri: String,
    bundle_path: String,
    uis: Vec<UiPlugin>,
}

/// A running LV2 UI instance.
pub struct UiInstance {
    handle: lv2_sys::LV2UI_Handle,
    widget: lv2_sys::LV2UI_Widget,
    descriptor: *const lv2_sys::LV2UI_Descriptor,
    controller: *mut UiWriteController,
    idle: Option<unsafe extern "C" fn(lv2_sys::LV2UI_Handle) -> ::std::os::raw::c_int>,
    _library: libloading::Library,
}

/// Opaque controller passed as LV2UI_Controller for ui_write_cb.
/// Holds a map of control input port indices to their float buffers
/// so the UI callback can write parameter changes directly.
struct UiWriteController {
    control_inputs: HashMap<u32, *mut f32>,
}

// UiInstance is Send + Sync because it's just FFI handles; the
// underlying LV2 functions (port_event, idle) are expected to be
// safe to call from any thread per the LV2 UI extension spec.
unsafe impl Send for UiInstance {}
unsafe impl Sync for UiInstance {}

impl UiInstance {
    /// The native widget handle (X11 Window, GtkWidget*, etc.).
    pub fn widget(&self) -> lv2_sys::LV2UI_Widget {
        self.widget
    }

    /// Send a port event to the UI.
    /// # Safety
    /// `buffer` must be a valid pointer to `buffer_size` bytes of memory for the
    /// given `protocol`.
    pub unsafe fn port_event(
        &self,
        port_index: u32,
        buffer_size: u32,
        protocol: u32,
        buffer: *const c_void,
    ) {
        if let Some(port_event) = unsafe { (*self.descriptor).port_event } {
            unsafe { port_event(self.handle, port_index, buffer_size, protocol, buffer) };
        }
    }

    /// Process one iteration of the UI's idle loop (if supported).
    /// Returns `None` if idle interface is not available, `Some(0)` if ok,
    /// `Some(non-zero)` if the UI wants to close.
    pub fn idle(&self) -> Option<i32> {
        self.idle.map(|f| unsafe { f(self.handle) })
    }

    /// Destroy the UI (calls cleanup and drops the library).
    pub fn cleanup(self) {
        // drop handles cleanup via Drop impl
    }
}

impl Drop for UiInstance {
    fn drop(&mut self) {
        if let Some(cleanup) = unsafe { (*self.descriptor).cleanup } {
            unsafe { cleanup(self.handle) };
        }
        if !self.controller.is_null() {
            unsafe { drop(Box::from_raw(self.controller)) };
        }
    }
}

// An instance may be moved to (and used from exactly) one audio thread.
unsafe impl Send for Instance {}

impl Instance {
    pub fn ports(&self) -> &[Port] {
        &self.ports
    }

    /// Port counts for this instance.
    pub fn port_counts(&self) -> PortCounts {
        self.port_counts
    }

    pub fn activate(&mut self) {
        if !self.active {
            if let Some(activate) = unsafe { (*self.descriptor).activate } {
                unsafe { activate(self.handle) };
            }
            self.active = true;
        }
    }

    pub fn deactivate(&mut self) {
        if self.active {
            if let Some(deactivate) = unsafe { (*self.descriptor).deactivate } {
                unsafe { deactivate(self.handle) };
            }
            self.active = false;
        }
    }

    /// Process `frames` samples using the instance's own internal buffers.
    pub fn run(&mut self, frames: usize) -> Result<(), Error> {
        if frames < self.min_block {
            return Err(Error::BlockTooSmall {
                requested: frames,
                min: self.min_block,
            });
        }
        if frames > self.max_block {
            return Err(Error::BlockTooLarge {
                requested: frames,
                max: self.max_block,
            });
        }
        if !self.active {
            self.activate();
        }
        for buf in &mut self.buffers {
            if let PortBuffer::AtomOut(seq) = buf {
                seq.prepare_output();
            }
        }
        if let Some(ref worker) = self.worker_runtime {
            worker.deliver_responses(self.handle);
        }
        let run = unsafe { (*self.descriptor).run }
            .ok_or_else(|| Error::Instantiation("descriptor has no run()".into()))?;
        unsafe { run(self.handle, frames as u32) };
        if let Some(ref worker) = self.worker_runtime {
            worker.end_run(self.handle);
        }
        for buf in &mut self.buffers {
            if let PortBuffer::AtomIn(seq) = buf {
                seq.clear();
            }
        }
        Ok(())
    }

    /// Process `samples` frames using externally-provided buffers.
    ///
    /// This matches livi's `instance.run(samples, ports)` pattern:
    /// audio and atom ports are re-connected every call, while control ports
    /// remain connected to the internal buffers set during instantiation.
    ///
    /// # Safety
    /// Running plugin code is inherently unsafe. The caller must ensure all
    /// buffers are valid for the duration of the call.
    pub fn run_with_ports<'a, AI, AO, ASI, ASO>(
        &mut self,
        samples: usize,
        ports: PortConnections<'a, AI, AO, ASI, ASO>,
    ) -> Result<(), Error>
    where
        AI: ExactSizeIterator + Iterator<Item = &'a [f32]>,
        AO: ExactSizeIterator + Iterator<Item = &'a mut [f32]>,
        ASI: ExactSizeIterator + Iterator<Item = &'a AtomSequence>,
        ASO: ExactSizeIterator + Iterator<Item = &'a mut AtomSequence>,
    {
        if samples < self.min_block {
            return Err(Error::BlockTooSmall {
                requested: samples,
                min: self.min_block,
            });
        }
        if samples > self.max_block {
            return Err(Error::BlockTooLarge {
                requested: samples,
                max: self.max_block,
            });
        }

        let connect = unsafe { (*self.descriptor).connect_port }
            .ok_or_else(|| Error::Instantiation("descriptor has no connect_port".into()))?;

        // Validate and connect audio inputs.
        if ports.audio_inputs.len() != self.audio_input_indices.len() {
            return Err(Error::PortCountMismatch {
                expected: self.audio_input_indices.len(),
                actual: ports.audio_inputs.len(),
            });
        }
        for (data, &index) in ports.audio_inputs.zip(self.audio_input_indices.iter()) {
            if data.len() < samples {
                return Err(Error::PortCountMismatch {
                    expected: samples,
                    actual: data.len(),
                });
            }
            unsafe { connect(self.handle, index, data.as_ptr() as *mut c_void) };
        }

        // Validate and connect audio outputs.
        if ports.audio_outputs.len() != self.audio_output_indices.len() {
            return Err(Error::PortCountMismatch {
                expected: self.audio_output_indices.len(),
                actual: ports.audio_outputs.len(),
            });
        }
        for (data, &index) in ports.audio_outputs.zip(self.audio_output_indices.iter()) {
            if data.len() < samples {
                return Err(Error::PortCountMismatch {
                    expected: samples,
                    actual: data.len(),
                });
            }
            unsafe { connect(self.handle, index, data.as_mut_ptr() as *mut c_void) };
        }

        // Validate and connect atom sequence inputs.
        if ports.atom_sequence_inputs.len() != self.atom_input_indices.len() {
            return Err(Error::PortCountMismatch {
                expected: self.atom_input_indices.len(),
                actual: ports.atom_sequence_inputs.len(),
            });
        }
        for (seq, &index) in ports
            .atom_sequence_inputs
            .zip(self.atom_input_indices.iter())
        {
            unsafe { connect(self.handle, index, seq.as_ptr() as *mut c_void) };
        }

        // Validate, clear-as-chunk, and connect atom sequence outputs.
        if ports.atom_sequence_outputs.len() != self.atom_output_indices.len() {
            return Err(Error::PortCountMismatch {
                expected: self.atom_output_indices.len(),
                actual: ports.atom_sequence_outputs.len(),
            });
        }
        for (seq, &index) in ports
            .atom_sequence_outputs
            .zip(self.atom_output_indices.iter())
        {
            seq.clear_as_chunk();
            unsafe { connect(self.handle, index, seq.as_mut_ptr()) };
        }

        if !self.active {
            self.activate();
        }

        if let Some(ref worker) = self.worker_runtime {
            worker.deliver_responses(self.handle);
        }
        let run = unsafe { (*self.descriptor).run }
            .ok_or_else(|| Error::Instantiation("descriptor has no run()".into()))?;
        unsafe { run(self.handle, samples as u32) };
        if let Some(ref worker) = self.worker_runtime {
            worker.end_run(self.handle);
        }

        Ok(())
    }

    /// Queue a raw MIDI message into the first atom-sequence input port.
    pub fn push_midi(&mut self, frame: i64, message: &[u8]) -> Result<(), Error> {
        let midi = self.midi_urid;
        for (port, buf) in self.ports.iter().zip(self.buffers.iter_mut()) {
            if port.direction == PortDirection::Input
                && let PortBuffer::AtomIn(seq) = buf
            {
                return seq.push_event(frame, midi, message);
            }
        }
        Err(Error::UnsupportedPort(
            "plugin has no atom-sequence input".into(),
        ))
    }

    /// Set a control input by its port index.
    ///
    /// Returns the clamped value that was actually written.
    pub fn set_control_input(&mut self, index: PortIndex, value: f32) -> Option<f32> {
        let &buf_idx = self.control_input_map.get(&index.0)?;
        match &mut self.buffers[buf_idx] {
            PortBuffer::Control(v) => {
                let port = &self.ports[buf_idx];
                let min = port.minimum.unwrap_or(f32::NEG_INFINITY);
                let max = port.maximum.unwrap_or(f32::INFINITY);
                let clamped = value.clamp(min, max);
                **v = clamped;
                Some(clamped)
            }
            _ => None,
        }
    }

    /// Set a control input by symbol name.
    pub fn set_control(&mut self, symbol: &str, value: f32) -> bool {
        for (port, buf) in self.ports.iter().zip(self.buffers.iter_mut()) {
            if port.symbol == symbol
                && let PortBuffer::Control(v) = buf
            {
                **v = value;
                return true;
            }
        }
        false
    }

    /// Read a control port value by symbol name.
    pub fn control(&self, symbol: &str) -> Option<f32> {
        for (port, buf) in self.ports.iter().zip(self.buffers.iter()) {
            if port.symbol == symbol
                && let PortBuffer::Control(v) = buf
            {
                return Some(**v);
            }
        }
        None
    }

    /// Read a control input value by its port index.
    pub fn control_input(&self, index: PortIndex) -> Option<f32> {
        let &buf_idx = self.control_input_map.get(&index.0)?;
        match &self.buffers[buf_idx] {
            PortBuffer::Control(v) => Some(**v),
            _ => None,
        }
    }

    /// Read a control output value by its port index (matching livi's
    /// `instance.control_output(index)`).
    pub fn control_output(&self, index: PortIndex) -> Option<f32> {
        let &buf_idx = self.control_output_map.get(&index.0)?;
        match &self.buffers[buf_idx] {
            PortBuffer::Control(v) => Some(**v),
            _ => None,
        }
    }

    /// Forward all control port values (inputs and outputs) to a UI instance
    /// via `port_event`.  The host should call this after each `run()` cycle
    /// so the UI reflects the current state of all control ports.
    pub fn update_ui(&self, ui: &UiInstance) {
        const FLOAT_PROTOCOL: u32 = 0;
        for (port, buf) in self.ports.iter().zip(self.buffers.iter()) {
            if let PortBuffer::Control(v) = buf {
                let value = **v;
                unsafe {
                    ui.port_event(
                        port.index,
                        4,
                        FLOAT_PROTOCOL,
                        &value as *const f32 as *const c_void,
                    );
                }
            }
        }
    }

    /// Iterate over all control ports and their values.
    pub fn controls(&self) -> impl Iterator<Item = (&Port, f32)> + '_ {
        self.ports
            .iter()
            .zip(self.buffers.iter())
            .filter_map(|(p, b)| match b {
                PortBuffer::Control(v) => Some((p, **v)),
                _ => None,
            })
    }

    fn nth_audio(&self, dir: PortDirection, nth: usize) -> Option<usize> {
        let mut n = 0;
        for (i, port) in self.ports.iter().enumerate() {
            if matches!(port.kind, PortKind::Audio | PortKind::Cv) && port.direction == dir {
                if n == nth {
                    return Some(i);
                }
                n += 1;
            }
        }
        None
    }

    pub fn n_audio_inputs(&self) -> usize {
        self.ports
            .iter()
            .filter(|p| {
                matches!(p.kind, PortKind::Audio | PortKind::Cv)
                    && p.direction == PortDirection::Input
            })
            .count()
    }

    pub fn n_audio_outputs(&self) -> usize {
        self.ports
            .iter()
            .filter(|p| {
                matches!(p.kind, PortKind::Audio | PortKind::Cv)
                    && p.direction == PortDirection::Output
            })
            .count()
    }

    pub fn audio_input_mut(&mut self, nth: usize) -> Option<&mut [f32]> {
        let i = self.nth_audio(PortDirection::Input, nth)?;
        match &mut self.buffers[i] {
            PortBuffer::Audio(v) => Some(v.as_mut_slice()),
            _ => None,
        }
    }

    pub fn audio_output(&self, nth: usize) -> Option<&[f32]> {
        let i = self.nth_audio(PortDirection::Output, nth)?;
        match &self.buffers[i] {
            PortBuffer::Audio(v) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// nth atom-sequence output (e.g. MIDI out of an arpeggiator).
    pub fn atom_output(&self, nth: usize) -> Option<&AtomSequence> {
        let mut n = 0;
        for (port, buf) in self.ports.iter().zip(self.buffers.iter()) {
            if port.direction == PortDirection::Output
                && let PortBuffer::AtomOut(seq) = buf
            {
                if n == nth {
                    return Some(seq);
                }
                n += 1;
            }
        }
        None
    }

    /// Whether this plugin has any discoverable UI.
    pub fn has_editor(&self) -> bool {
        !self.uis.is_empty()
    }

    /// Open the first discoverable UI for this plugin with no parent window.
    pub fn open_editor(&self) -> Result<UiInstance, Error> {
        self.open_editor_with_parent(0)
    }

    /// Open the first discoverable UI for this plugin.
    ///
    /// When `parent_window` is non-zero, it is passed as the LV2_UI__parent
    /// feature (embedded mode).  When zero, uses ui:showInterface (floating).
    pub fn open_editor_with_parent(&self, parent_window: usize) -> Result<UiInstance, Error> {
        let ui = self
            .uis
            .first()
            .ok_or_else(|| Error::PluginNotFound(self.plugin_uri.clone()))?;

        let library = unsafe { libloading::Library::new(&ui.binary_path) }
            .map_err(|e| Error::Library(e.to_string()))?;

        let descriptor_fn: libloading::Symbol<
            unsafe extern "C" fn(u32) -> *const lv2_sys::LV2UI_Descriptor,
        > = unsafe { library.get(b"lv2ui_descriptor\0") }
            .map_err(|e| Error::Library(e.to_string()))?;

        let descriptor = {
            let mut i: u32 = 0;
            let mut first: *const lv2_sys::LV2UI_Descriptor = std::ptr::null();
            loop {
                let d = unsafe { descriptor_fn(i) };
                if d.is_null() {
                    // If we have a named UI URI try matching it; otherwise
                    // fall back to matching the plugin URI (common for
                    // blank-node UIs).  As a last resort use the first
                    // descriptor found.
                    if !first.is_null() {
                        break first;
                    }
                    return Err(Error::Library("no matching LV2UI_Descriptor found".into()));
                }
                if first.is_null() {
                    first = d;
                }
                let desc_uri = unsafe { std::ffi::CStr::from_ptr((*d).URI) }.to_bytes();
                // Try matching UI URI first, then plugin URI as fallback
                if (!ui.uri.is_empty() && desc_uri == ui.uri.as_bytes())
                    || (ui.uri.is_empty() && desc_uri == self.plugin_uri.as_bytes())
                {
                    break d;
                }
                i += 1;
            }
        };

        let bundle_c = CString::new(self.bundle_path.as_str())
            .map_err(|_| Error::Instantiation(self.plugin_uri.clone()))?;
        let plugin_uri_c = CString::new(self.plugin_uri.as_str())
            .map_err(|_| Error::Instantiation(self.plugin_uri.clone()))?;

        // Build feature data that outlives the instantiate call
        let urid_handle = Arc::as_ptr(&self._urid) as *mut c_void;
        let map_uri_c = CString::new(uris::URID_MAP).unwrap();
        let unmap_uri_c = CString::new(uris::URID_UNMAP).unwrap();
        let parent_uri_c = CString::new(uris::UI_PARENT).unwrap();
        let show_ui_c = CString::new(uris::UI_SHOW_INTERFACE).unwrap();
        let opt_uri_c = CString::new(uris::OPTIONS_OPTIONS).unwrap();
        let urid_map = lv2_sys::LV2_URID_Map {
            handle: urid_handle,
            map: Some(urid_map_cb),
        };
        let urid_unmap = lv2_sys::LV2_URID_Unmap {
            handle: urid_handle,
            unmap: Some(urid_unmap_cb),
        };

        let sample_rate_f32: f32 = self.sample_rate as f32;
        let sample_rate_val = &sample_rate_f32 as *const f32 as *mut c_void;
        let atom_float = self._urid.map(uris::ATOM_FLOAT);
        let sr_urid = self._urid.map(uris::PARAM_SAMPLE_RATE);
        let ui_options = [
            lv2_sys::LV2_Options_Option {
                context: 0,
                subject: 0,
                key: sr_urid,
                size: 4,
                type_: atom_float,
                value: sample_rate_val,
            },
            lv2_sys::LV2_Options_Option {
                context: 0,
                subject: 0,
                key: 0,
                size: 0,
                type_: 0,
                value: std::ptr::null(),
            },
        ];

        let mut ui_features = vec![
            lv2_sys::LV2_Feature {
                URI: map_uri_c.as_ptr(),
                data: &urid_map as *const lv2_sys::LV2_URID_Map as *mut c_void,
            },
            lv2_sys::LV2_Feature {
                URI: unmap_uri_c.as_ptr(),
                data: &urid_unmap as *const lv2_sys::LV2_URID_Unmap as *mut c_void,
            },
            lv2_sys::LV2_Feature {
                URI: opt_uri_c.as_ptr(),
                data: ui_options.as_ptr() as *mut c_void,
            },
        ];
        if parent_window != 0 {
            ui_features.push(lv2_sys::LV2_Feature {
                URI: parent_uri_c.as_ptr(),
                data: parent_window as *mut c_void,
            });
        } else {
            ui_features.push(lv2_sys::LV2_Feature {
                URI: show_ui_c.as_ptr(),
                data: std::ptr::null_mut(),
            });
        }
        let mut ui_feature_ptrs: Vec<*const lv2_sys::LV2_Feature> = ui_features
            .iter()
            .map(|f| f as *const lv2_sys::LV2_Feature)
            .collect();
        ui_feature_ptrs.push(std::ptr::null());

        unsafe extern "C" fn ui_write_cb(
            controller: lv2_sys::LV2UI_Controller,
            port_index: u32,
            buffer_size: u32,
            port_protocol: u32,
            buffer: *const std::ffi::c_void,
        ) {
            let ctrl = unsafe { &*(controller as *const UiWriteController) };
            if port_protocol == 0 && buffer_size == 4 && !buffer.is_null() {
                let value = unsafe { *(buffer as *const f32) };
                if let Some(&ptr) = ctrl.control_inputs.get(&port_index) {
                    unsafe { *ptr = value };
                }
            }
        }

        let mut widget: lv2_sys::LV2UI_Widget = std::ptr::null_mut();

        let instantiate = unsafe { (*descriptor).instantiate }
            .ok_or_else(|| Error::Instantiation(self.plugin_uri.clone()))?;

        // for ui_write_cb to be able to forward parameter changes to the plugin instance.
        let mut control_inputs = HashMap::new();
        for (&port_idx, &buf_idx) in &self.control_input_map {
            if let PortBuffer::Control(v) = &self.buffers[buf_idx] {
                control_inputs.insert(port_idx, &**v as *const f32 as *mut f32);
            }
        }
        let ui_ctrl = Box::into_raw(Box::new(UiWriteController { control_inputs }));
        let controller = ui_ctrl as *mut std::ffi::c_void;

        let handle = unsafe {
            instantiate(
                descriptor,
                plugin_uri_c.as_ptr(),
                bundle_c.as_ptr(),
                Some(ui_write_cb),
                controller,
                &mut widget,
                ui_feature_ptrs.as_ptr(),
            )
        };

        if handle.is_null() {
            unsafe { drop(Box::from_raw(ui_ctrl)) };
            return Err(Error::Instantiation(self.plugin_uri.clone()));
        }

        // Call show() via the showInterface extension to display the UI
        // in floating mode (no parent window).
        if parent_window == 0 {
            let show_ext_uri = CString::new(uris::UI_SHOW_INTERFACE).expect("valid CString");
            if let Some(ext_data) = unsafe { (*descriptor).extension_data } {
                let iface_ptr = unsafe { ext_data(show_ext_uri.as_ptr()) };
                if !iface_ptr.is_null() {
                    let show_iface =
                        unsafe { &*(iface_ptr as *const lv2_sys::LV2UI_Show_Interface) };
                    if let Some(show) = show_iface.show {
                        unsafe { show(handle) };
                    }
                }
            }
        }

        // Query the idle interface (ui:idleInterface) for periodic pumping.
        let idle_ext_uri = CString::new(uris::UI_IDLE_INTERFACE).expect("valid CString");
        let idle = unsafe { (*descriptor).extension_data }.and_then(|ext_data| {
            let ptr = unsafe { ext_data(idle_ext_uri.as_ptr()) };
            if ptr.is_null() {
                None
            } else {
                let iface = unsafe { &*(ptr as *const lv2_sys::LV2UI_Idle_Interface) };
                iface.idle
            }
        });
        if idle.is_some() {
            eprintln!("[yeli debug] ui:idleInterface available");
        } else {
            eprintln!("[yeli debug] ui:idleInterface NOT available");
        }

        // Forward initial control port values to the UI so it shows the
        // current state (levels, knob positions, etc.) immediately.
        if let Some(port_event_fn) = unsafe { (*descriptor).port_event } {
            for (port, buf) in self.ports.iter().zip(self.buffers.iter()) {
                if let PortBuffer::Control(v) = buf {
                    let value = **v;
                    unsafe {
                        port_event_fn(
                            handle,
                            port.index,
                            4,
                            0,
                            &value as *const f32 as *const c_void,
                        )
                    };
                }
            }
        }

        Ok(UiInstance {
            handle,
            widget,
            descriptor,
            controller: ui_ctrl,
            idle,
            _library: library,
        })
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Shutdown worker thread first so it can't access the plugin after cleanup.
        if let Some(mut worker) = self.worker_runtime.take() {
            worker.shutdown();
        }
        unsafe {
            if self.active
                && let Some(deactivate) = (*self.descriptor).deactivate
            {
                deactivate(self.handle);
            }
            if let Some(cleanup) = (*self.descriptor).cleanup {
                cleanup(self.handle);
            }
        }
    }
}
