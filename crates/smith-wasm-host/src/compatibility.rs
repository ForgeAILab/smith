//! Frozen 0.1 bindings adapt to the current broker and contribution projection.
use crate::bindings::{Module as Current, smith::module::types as w};
use crate::bindings_v01::{
    Module as Legacy,
    smith::module::{host, types as old},
};
use crate::broker::Broker;
use serde::{Serialize, de::DeserializeOwned};
use wasmtime::{
    Store,
    component::{Component, HasSelf, Linker},
};

// Shared records are identical across versions. Keep conversion fallible so a
// later interface edit cannot turn a legacy call into a host panic.
fn convert<T: DeserializeOwned>(value: &impl Serialize) -> wasmtime::Result<T> {
    Ok(serde_json::from_slice(&serde_json::to_vec(value)?)?)
}

pub(crate) enum Guest {
    Current(Current),
    Legacy(Legacy),
}
impl Guest {
    pub(crate) fn instantiate(
        store: &mut Store<Broker>,
        component: &Component,
    ) -> wasmtime::Result<Self> {
        let legacy = component
            .component_type()
            .imports(store.engine())
            .any(|(name, _)| name.ends_with("@0.1.0") && name.starts_with("smith:module/"));
        let mut linker = Linker::new(store.engine());
        if legacy {
            Legacy::add_to_linker::<_, HasSelf<Broker>>(&mut linker, |broker: &mut Broker| broker)?;
            Ok(Self::Legacy(Legacy::instantiate(
                store, component, &linker,
            )?))
        } else {
            Current::add_to_linker::<_, HasSelf<Broker>>(&mut linker, |broker: &mut Broker| {
                broker
            })?;
            Ok(Self::Current(Current::instantiate(
                store, component, &linker,
            )?))
        }
    }
    pub(crate) fn call_describe(
        &self,
        store: &mut Store<Broker>,
    ) -> wasmtime::Result<w::Descriptor> {
        match self {
            Self::Current(guest) => guest.call_describe(store),
            Self::Legacy(guest) => {
                let value = guest.call_describe(&mut *store)?;
                adapt_result(store, &value)
            }
        }
    }
    pub(crate) fn call_mount(
        &self,
        store: &mut Store<Broker>,
        ctx: &w::MountContext,
    ) -> wasmtime::Result<Result<w::Mounted, w::ModuleError>> {
        match self {
            Self::Current(guest) => guest.call_mount(store, ctx),
            Self::Legacy(guest) => {
                let mut ctx = ctx.clone();
                ctx.grants.retain(|g| {
                    !matches!(
                        g.capability,
                        w::Capability::Model | w::Capability::SessionNotify
                    )
                });
                let value = guest.call_mount(&mut *store, &convert(&ctx)?)?;
                adapt_result(store, &value)
            }
        }
    }
    pub(crate) fn call_status(
        &self,
        store: &mut Store<Broker>,
        id: &str,
    ) -> wasmtime::Result<Option<w::StatusItem>> {
        match self {
            Self::Current(guest) => guest.call_status(store, id),
            Self::Legacy(guest) => {
                let value = guest.call_status(&mut *store, id)?;
                adapt_result(store, &value)
            }
        }
    }
    pub(crate) fn call_poll(
        &self,
        store: &mut Store<Broker>,
        id: &str,
    ) -> wasmtime::Result<Result<Vec<w::Notice>, w::ModuleError>> {
        match self {
            Self::Current(guest) => guest.call_poll(store, id),
            Self::Legacy(_) => Ok(Err(w::ModuleError {
                kind: w::ErrorKind::UnknownId,
                message: "interface 0.1 has no watchers".into(),
            })),
        }
    }
}
macro_rules! guest_call {
    ($method:ident, $input:ty, $output:ty) => {
        impl Guest {
            pub(crate) fn $method(
                &self,
                store: &mut Store<Broker>,
                id: &str,
                input: &$input,
            ) -> wasmtime::Result<Result<$output, w::ModuleError>> {
                match self {
                    Self::Current(guest) => guest.$method(store, id, input),
                    Self::Legacy(guest) => {
                        let value = guest.$method(&mut *store, id, &convert(input)?)?;
                        adapt_result(store, &value)
                    }
                }
            }
        }
    };
}
impl Guest {
    pub(crate) fn call_call_tool(
        &self,
        store: &mut Store<Broker>,
        id: &str,
        input: &w::ToolInput,
    ) -> wasmtime::Result<Result<String, w::ModuleError>> {
        match self {
            Self::Current(guest) => guest.call_call_tool(store, id, input),
            Self::Legacy(guest) => {
                let value = guest.call_call_tool(&mut *store, id, &convert(input)?)?;
                Ok(match value {
                    Ok(output) => Ok(output),
                    Err(error) => Err(adapt_result(store, &error)?),
                })
            }
        }
    }
}
guest_call!(call_contribute_context, w::ContextView, w::ContextPatch);
guest_call!(
    call_process_tool_output,
    w::ToolOutputView,
    w::ToolOutputPatch
);

fn adapt_result<T: DeserializeOwned>(
    store: &mut Store<Broker>,
    value: &impl Serialize,
) -> wasmtime::Result<T> {
    if let Err(error) = crate::limits::bounded(value, &store.data().limits) {
        store.data_mut().failure = Some(error.clone());
        return Err(error.into());
    }
    convert(value)
}

impl old::Host for Broker {}
fn legacy_error(error: w::HostError) -> old::HostError {
    convert(&error)
        .unwrap_or_else(|_| old::HostError::Unavailable("interface adaptation failed".into()))
}
fn legacy_result<T: Serialize, U: DeserializeOwned>(
    result: Result<T, w::HostError>,
) -> Result<U, old::HostError> {
    match result {
        Ok(value) => convert(&value)
            .map_err(|_| old::HostError::Unavailable("interface adaptation failed".into())),
        Err(error) => Err(legacy_error(error)),
    }
}
fn legacy_input<T: DeserializeOwned>(
    broker: &mut Broker,
    input: &impl Serialize,
) -> Result<T, old::HostError> {
    broker.check(input).map_err(legacy_error)?;
    convert(input).map_err(|_| old::HostError::InvalidInput("interface adaptation failed".into()))
}
impl host::Host for Broker {
    fn log(&mut self, level: old::LogLevel, message: String) -> Result<(), old::HostError> {
        let level = legacy_input(self, &level)?;
        crate::bindings::smith::module::host::Host::log(self, level, message).map_err(legacy_error)
    }
    fn fs_read(&mut self, file: old::FilePath) -> Result<Vec<u8>, old::HostError> {
        let file = w::FilePath {
            root: file.root,
            path: file.path,
        };
        crate::bindings::smith::module::host::Host::fs_read(self, file).map_err(legacy_error)
    }
    fn fs_write(&mut self, write: old::FileWrite) -> Result<(), old::HostError> {
        let write = w::FileWrite {
            file: w::FilePath {
                root: write.file.root,
                path: write.file.path,
            },
            bytes: write.bytes,
        };
        crate::bindings::smith::module::host::Host::fs_write(self, write).map_err(legacy_error)
    }
    fn http(&mut self, request: old::HttpRequest) -> Result<old::HttpResponse, old::HostError> {
        let request = w::HttpRequest {
            method: legacy_input(self, &request.method)?,
            url: request.url,
            headers: request
                .headers
                .into_iter()
                .map(|header| w::HttpHeader {
                    name: header.name,
                    value: header.value,
                })
                .collect(),
            body: request.body,
        };
        let response = crate::bindings::smith::module::host::Host::http(self, request)
            .map_err(legacy_error)?;
        Ok(old::HttpResponse {
            status: response.status,
            headers: response
                .headers
                .into_iter()
                .map(|header| old::HttpHeader {
                    name: header.name,
                    value: header.value,
                })
                .collect(),
            body: response.body,
        })
    }
    fn kv_get(&mut self, key: String) -> Result<Option<Vec<u8>>, old::HostError> {
        crate::bindings::smith::module::host::Host::kv_get(self, key).map_err(legacy_error)
    }
    fn kv_set(&mut self, entry: old::KvEntry) -> Result<(), old::HostError> {
        let entry = w::KvEntry {
            key: entry.key,
            value: entry.value,
        };
        crate::bindings::smith::module::host::Host::kv_set(self, entry).map_err(legacy_error)
    }
    fn session_history(&mut self, limit: u32) -> Result<old::HistorySnapshot, old::HostError> {
        legacy_result(crate::bindings::smith::module::host::Host::session_history(
            self, limit,
        ))
    }
}
