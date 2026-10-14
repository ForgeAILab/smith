//! Fallback cleanup for endpoints declared to this test's harness only.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub(crate) struct BridgeCleanup {
    record: Option<PathBuf>,
    argv_directory: Option<PathBuf>,
    endpoints: RefCell<BTreeSet<PathBuf>>,
}

impl BridgeCleanup {
    #[allow(dead_code)]
    pub(crate) fn from_record(record: PathBuf) -> Self {
        Self {
            record: Some(record),
            argv_directory: None,
            endpoints: RefCell::new(BTreeSet::new()),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn from_argv_directory(directory: PathBuf) -> Self {
        Self {
            record: None,
            argv_directory: Some(directory),
            endpoints: RefCell::new(BTreeSet::new()),
        }
    }

    fn remember_args(&self, args: &Value) {
        if let Some(args) = args.as_array()
            && args.len() == 2
            && args[0] == "mcp-relay"
            && let Some(endpoint) = args[1].as_str().map(PathBuf::from)
            && endpoint.is_absolute()
            && endpoint.parent().is_some_and(|directory| {
                directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".sb-"))
            })
        {
            self.endpoints.borrow_mut().insert(endpoint);
        }
    }

    fn remember_record(&self, path: &Path) {
        if let Ok(bytes) = std::fs::read(path)
            && let Ok(record) = serde_json::from_slice::<Value>(&bytes)
        {
            self.remember_args(&record["server"]["args"]);
            // A stand-in can fail before parsing the declaration into server.
            if let Some(argv) = record["argv"].as_array() {
                let argv = argv.iter().filter_map(Value::as_str).collect::<Vec<_>>();
                self.remember_argv(&argv);
            }
        }
    }

    fn remember_argv(&self, argv: &[&str]) {
        for pair in argv.windows(2) {
            if pair[0] == "--mcp-config"
                && let Ok(config) = serde_json::from_str::<Value>(pair[1])
            {
                self.remember_args(&config["mcpServers"]["smith"]["args"]);
            }
            if pair[0] == "-c"
                && let Some(args) = pair[1].strip_prefix("mcp_servers.smith.args=")
                && let Ok(args) = serde_json::from_str::<Value>(args)
            {
                self.remember_args(&args);
            }
        }
    }

    pub(crate) fn capture(&self) {
        if let Some(record) = &self.record {
            self.remember_record(record);
            // Each child has an independent archive; the primary record can
            // be overwritten by another invocation or deleted before resume.
            if let Some(directory) = record.parent()
                && let Some(stem) = record.file_stem().and_then(|stem| stem.to_str())
                && let Ok(entries) = std::fs::read_dir(directory)
            {
                let prefix = format!("{stem}.");
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    if name.to_str().is_some_and(|name| {
                        name.starts_with(&prefix) && name.ends_with(".invocation.json")
                    }) {
                        self.remember_record(&entry.path());
                    }
                }
            }
        }
        if let Some(directory) = &self.argv_directory
            && let Ok(entries) = std::fs::read_dir(directory)
        {
            for entry in entries.flatten() {
                if entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "argv")
                    && let Ok(bytes) = std::fs::read(entry.path())
                    && let Ok(argv) = std::str::from_utf8(&bytes)
                {
                    self.remember_argv(&argv.split('\0').collect::<Vec<_>>());
                }
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn endpoints(&self) -> Vec<PathBuf> {
        self.capture();
        self.endpoints.borrow().iter().cloned().collect()
    }
}

impl Drop for BridgeCleanup {
    fn drop(&mut self) {
        // Process guards must stop Smith before this guard runs. Reading its
        // durable declarations again also covers panic before the first poll.
        self.capture();
        for endpoint in self.endpoints.get_mut().iter() {
            if let Some(directory) = endpoint.parent() {
                let _ = std::fs::remove_dir_all(directory);
            }
        }
    }
}
