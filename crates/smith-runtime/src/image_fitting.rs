//! Request-local image preparation, shared by every attempt in a provider stack.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use agent_runtime_core::content::ContentPart;
use agent_runtime_core::provider::{
    CacheResourceProvider, Capabilities, ModelDescriptor, ModelId, Provider, ProviderCallContext,
    ProviderError, ProviderRequest, ProviderStream,
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use smith_tools::ImageLimits;
use smith_tools::image_preparation::prepare_image_data_url;

const CACHE_ENTRIES: usize = 32;
const CACHE_BYTES: usize = 64 * 1024 * 1024;

type PreparedUrl = Result<String, String>;

struct CacheEntry {
    key: [u8; 32],
    output: PreparedUrl,
}

impl CacheEntry {
    fn size(&self) -> usize {
        match &self.output {
            Ok(url) => url.len(),
            Err(reason) => reason.len(),
        }
    }
}

#[derive(Default)]
struct ImageCache {
    entries: VecDeque<CacheEntry>,
    bytes: usize,
    #[cfg(test)]
    preparations: usize,
}

impl ImageCache {
    fn get(&mut self, key: &[u8; 32]) -> Option<PreparedUrl> {
        let index = self.entries.iter().position(|entry| &entry.key == key)?;
        let entry = self.entries.remove(index)?;
        let output = entry.output.clone();
        self.entries.push_back(entry);
        Some(output)
    }

    fn insert(&mut self, key: [u8; 32], output: PreparedUrl) {
        let entry = CacheEntry { key, output };
        let size = entry.size();
        if size > CACHE_BYTES {
            return;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + size > CACHE_BYTES {
            if let Some(oldest) = self.entries.pop_front() {
                self.bytes -= oldest.size();
            }
        }
        self.bytes += size;
        self.entries.push_back(entry);
    }
}

pub(crate) struct ImageFittingProvider {
    inner: Arc<dyn Provider>,
    limits: ImageLimits,
    limits_key: [u8; 32],
    cache: Mutex<ImageCache>,
}

impl fmt::Debug for ImageFittingProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageFittingProvider")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl ImageFittingProvider {
    pub(crate) fn new(inner: Arc<dyn Provider>, limits: ImageLimits) -> Self {
        // Hash the complete policy once; URL bytes are hashed on each lookup.
        let limits_key = Sha256::digest(format!("{limits:?}").as_bytes()).into();
        Self {
            inner,
            limits,
            limits_key,
            cache: Mutex::new(ImageCache::default()),
        }
    }

    fn fit_url(&self, url: &str) -> PreparedUrl {
        let mut digest = Sha256::new();
        digest.update(self.limits_key);
        digest.update(url.as_bytes());
        let key = digest.finalize().into();
        // Serialize preparation as well as lookup so concurrent retries cannot
        // decode the same image twice. Never hold this lock across provider I/O.
        let mut cache = self.cache.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(output) = cache.get(&key) {
            return output;
        }
        #[cfg(test)]
        {
            cache.preparations += 1;
        }
        let output = prepare_image_data_url(url, &self.limits).map_err(|error| error.to_string());
        cache.insert(key, output.clone());
        output
    }

    fn fit_content(&self, content: &mut [ContentPart]) {
        for part in content {
            match part {
                ContentPart::Image { url, .. } if url.starts_with("data:") => {
                    match self.fit_url(url) {
                        Ok(prepared) => *url = prepared,
                        Err(reason) => {
                            *part = ContentPart::text(format!("[image omitted: {reason}]"))
                        }
                    }
                }
                ContentPart::ToolResult(result) => self.fit_content(&mut result.content),
                _ => {}
            }
        }
    }
}

#[async_trait]
impl Provider for ImageFittingProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.inner.describe()
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }

    fn requires_nonempty_assistant_content(&self) -> bool {
        self.inner.requires_nonempty_assistant_content()
    }

    fn cache_resource_provider(&self) -> Option<&dyn CacheResourceProvider> {
        self.inner.cache_resource_provider()
    }

    async fn stream(
        &self,
        mut request: ProviderRequest,
        context: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        for message in &mut request.messages {
            self.fit_content(&mut message.content);
        }
        self.inner.stream(request, context).await
    }
}

#[cfg(test)]
#[path = "image_fitting_tests.rs"]
mod tests;
