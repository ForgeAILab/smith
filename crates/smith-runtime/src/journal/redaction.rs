use super::*;

/// The replacement written in place of a redacted value.
pub(super) const REDACTED: &str = "[redacted]";

/// Credential words matched anywhere in a key once separators and case are
/// removed, so `apiKey`, `API_KEY`, and `x-api-key` all match.
///
/// Every entry is specific enough that it cannot occur inside a benign
/// identifier — which is why the plain word `token` is *not* here: the shared
/// event vocabulary is full of legitimate `*_tokens` counters, and redacting
/// `reserved_tokens` would corrupt canonical usage data to protect nothing.
const SENSITIVE_KEY_NEEDLES: &[&str] = &[
    "accesskey",
    "accesstoken",
    "apikey",
    "authorization",
    "authtoken",
    "bearer",
    "credential",
    "passphrase",
    "password",
    "privatekey",
    "refreshtoken",
    "secret",
    "sessionkey",
];

/// Credential words matched only as a whole word of a key, where the plural
/// counters above cannot collide with them: `token` and `key` are sensitive,
/// `reserved_tokens` and `keyframes` are not.
const SENSITIVE_KEY_WORDS: &[&str] = &["auth", "key", "token"];

/// The journal's redaction seam.
///
/// Runs over the fully serialized line, in the writer task, before a single
/// byte reaches the file. Placing it here rather than at each call site means
/// a new event variant cannot introduce a leak by forgetting to opt in.
pub trait Redactor: Send + Sync + std::fmt::Debug {
    /// Rewrites `line` in place, removing anything that must not be persisted.
    fn redact(&self, line: &mut Value);
}

/// A redactor that keeps everything. Useful only where the caller has already
/// proven the event stream carries no credentials.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeepEverything;

impl Redactor for KeepEverything {
    fn redact(&self, _line: &mut Value) {}
}

/// Smith's default redaction policy.
///
/// Two independent rules, because a secret can enter the journal two different
/// ways. A *structural* rule replaces the value of any credential-shaped key,
/// which covers tool arguments and provider metadata whose shape Smith does
/// not control. A *literal* rule replaces any registered secret value wherever
/// it appears, including inside free text, which covers a model echoing a key
/// back into an assistant message.
#[derive(Clone, Default)]
pub struct DefaultRedactor {
    secrets: Arc<RwLock<Vec<String>>>,
}

impl DefaultRedactor {
    /// A redactor with the structural rule only.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a known secret literal to scrub wherever it appears.
    ///
    /// Resolved credentials are the intended input: the host knows the exact
    /// string it handed the provider, so the journal can be made to never
    /// contain it. An empty value is ignored, since it would match everywhere.
    pub fn with_secret(self, secret: impl Into<String>) -> Self {
        self.register_value(secret.into());
        self
    }

    /// Registers a resolved credential without exposing its value at the call
    /// site. Clones share the same registry, allowing the factory to resolve a
    /// credential after the host has already injected persistence adapters.
    pub fn register_secret(&self, secret: &Secret) {
        self.register_value(secret.expose().to_owned());
    }

    /// Registers an exact sensitive task value without classifying it as a
    /// credential.
    ///
    /// Interactive questionnaire answers use this path before the value enters
    /// canonical live history. Clones share the registry, so event and session
    /// persistence apply the same literal redaction.
    pub fn register_sensitive_value(&self, value: &str) {
        self.register_value(value.to_owned());
    }

    /// Returns a credential-redacted clone suitable for bounded local display.
    ///
    /// The canonical value remains untouched. Structural credential keys and
    /// every exact registered literal use the same policy as persistence.
    pub fn redacted_clone(&self, value: &Value) -> Value {
        let mut redacted = value.clone();
        self.scrub(&mut redacted);
        redacted
    }

    fn register_value(&self, secret: String) {
        if !secret.is_empty() {
            let mut secrets = self
                .secrets
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !secrets.contains(&secret) {
                secrets.push(secret);
            }
        }
    }

    fn scrub(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, child) in map.iter_mut() {
                    if is_sensitive_key(key) {
                        *child = Value::String(REDACTED.to_owned());
                    } else {
                        self.scrub(child);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.scrub(item);
                }
            }
            Value::String(text) => {
                let secrets = self
                    .secrets
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for secret in secrets.iter() {
                    if text.contains(secret.as_str()) {
                        *text = text.replace(secret.as_str(), REDACTED);
                    }
                }
            }
            _ => {}
        }
    }
}

impl smith_host::SensitiveValueSink for DefaultRedactor {
    fn register_sensitive_value(&self, value: &str) {
        Self::register_sensitive_value(self, value);
    }
}

impl std::fmt::Debug for DefaultRedactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let registered = self
            .secrets
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        f.debug_struct("DefaultRedactor")
            .field("registered_secrets", &registered)
            .finish()
    }
}

impl Redactor for DefaultRedactor {
    fn redact(&self, line: &mut Value) {
        self.scrub(line);
    }
}

/// Whether a key's *value* must never be persisted.
///
/// Deliberately biased towards over-redaction: a journal that hides one field
/// it did not have to is recoverable, a journal that persists one credential
/// is not. The one place that bias is held back is the `*_tokens` family, where
/// over-redaction would destroy the usage accounting the journal exists to
/// preserve.
fn is_sensitive_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    let compact: String = lowered
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if SENSITIVE_KEY_NEEDLES
        .iter()
        .any(|needle| compact.contains(needle))
    {
        return true;
    }
    lowered
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|word| SENSITIVE_KEY_WORDS.contains(&word))
}
