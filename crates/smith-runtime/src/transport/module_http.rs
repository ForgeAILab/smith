//! Module requests share the provider client's TLS, proxies and connection timeout.
use super::*;
use smith_module::{
    MODULE_HTTP_BODY_LIMIT, ModuleHttp, ModuleHttpMethod, ModuleHttpRequest, ModuleHttpResponse,
};

impl ModuleHttp for ReqwestTransport {
    fn request(&self, request: ModuleHttpRequest) -> smith_module::ModuleHttpFuture<'_> {
        Box::pin(self.send_module(request))
    }
}
impl ReqwestTransport {
    async fn send_module(
        &self,
        request: ModuleHttpRequest,
    ) -> Result<ModuleHttpResponse, ProviderError> {
        let invalid =
            || ProviderError::new(ProviderErrorKind::BadRequest, "invalid module HTTP request");
        if request.body.len() > MODULE_HTTP_BODY_LIMIT
            || request.url.len() > 4096
            || request.headers.len() > 64
            || request.headers.iter().any(|(name, value)| {
                name.len() > 128
                    || value.len() > 4096
                    || matches!(
                        name.to_ascii_lowercase().as_str(),
                        "host" | "connection" | "transfer-encoding" | "content-length"
                    )
            })
        {
            return Err(invalid());
        }
        let url = Url::parse(&request.url).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        let endpoint = endpoint_label(&url);
        let headers = header_map(&request.headers, &endpoint)?;
        let method = match request.method {
            ModuleHttpMethod::Get => reqwest::Method::GET,
            ModuleHttpMethod::Head => reqwest::Method::HEAD,
            ModuleHttpMethod::Post => reqwest::Method::POST,
            ModuleHttpMethod::Put => reqwest::Method::PUT,
            ModuleHttpMethod::Patch => reqwest::Method::PATCH,
            ModuleHttpMethod::Delete => reqwest::Method::DELETE,
            ModuleHttpMethod::Options => reqwest::Method::OPTIONS,
        };
        let deadline = Instant::now() + request.timeout.min(self.config.request_deadline);
        let response = tokio::time::timeout_at(
            deadline,
            self.client
                .request(method, url)
                .headers(headers)
                .body(request.body)
                .send(),
        )
        .await
        .map_err(|_| {
            fail(
                ProviderErrorKind::Timeout,
                "module HTTP deadline elapsed",
                &endpoint,
            )
        })?
        .map_err(|error| send_error(&error, &endpoint))?;
        let status = response.status().as_u16();
        // Refusing every redirect is conservative, including a redirect within
        // the grant. No credentials or body ever go to a second endpoint.
        if (300..400).contains(&status) {
            return Err(fail(
                ProviderErrorKind::BadRequest,
                "module HTTP redirect refused",
                &endpoint,
            ));
        }
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                let value = value.to_str().map_err(|_| invalid())?;
                if name.as_str().len() > 128 || value.len() > 4096 {
                    return Err(invalid());
                }
                Ok((name.as_str().to_owned(), value.to_owned()))
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        if headers.len() > 64 {
            return Err(invalid());
        }
        let maximum = request.max_response_bytes.min(MODULE_HTTP_BODY_LIMIT) as u64;
        let mut stream = bounded(
            response,
            endpoint,
            deadline,
            self.config.stall_timeout,
            maximum.min(self.config.max_response_bytes),
        );
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            body.extend(chunk?);
        }
        Ok(ModuleHttpResponse {
            status,
            headers,
            body,
        })
    }
}
