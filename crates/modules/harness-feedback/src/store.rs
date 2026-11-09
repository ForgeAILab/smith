use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smith_module::{FeedbackRedactor, SessionIdentity, ToolFailure};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

const MARKER: &str = "… [truncated]";
pub(crate) fn bounded(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.into();
    }
    text.chars()
        .take(limit.saturating_sub(MARKER.chars().count()))
        .collect::<String>()
        + MARKER
}
/// Local review state; queued reports are never pruned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Awaiting user review.
    Queued,
    /// Handed off to the browser by a review surface.
    Filed,
    /// Dismissed locally.
    Dismissed,
}
/// The three supported categories of Smith feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An existing Smith feature behaved incorrectly.
    Bug,
    /// Smith could not support a reasonable request.
    MissingCapability,
    /// A proposed improvement to Smith.
    Idea,
}
/// Host-supplied facts that tool arguments cannot override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// Resolved host identity at invocation.
    pub identity: SessionIdentity,
    /// Operating system of the Smith host.
    pub os: String,
    /// Session owning the invocation.
    pub session: String,
    /// Turn owning the invocation, when available.
    pub turn: Option<String>,
    /// Call that filed this evidence.
    pub call: String,
    /// Child session id when invoked by a child.
    pub child: Option<String>,
    /// Local workspace path, omitted from the public environment section.
    pub workspace: String,
    /// Up to five recent failed Smith tool calls.
    pub failures: Vec<ToolFailure>,
    /// States whether evidence is limited to bridged calls.
    pub coverage: String,
}
/// Agent-authored report text; always untrusted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportInput {
    /// Category chosen by the reporting agent.
    pub kind: Kind,
    /// Smith feature or subsystem involved.
    pub area: String,
    /// One-line title, bounded to 120 characters.
    pub title: String,
    /// Agent account of the failure; treated as untrusted text.
    pub what_happened: String,
    #[serde(default)]
    /// Optional user quote; treated as untrusted text.
    pub user_said: String,
    #[serde(default)]
    /// Optional reproduction steps; treated as untrusted text.
    pub repro: String,
}
/// One local report with bounded text and host evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    session_key: String,
    duplicate_key: String,
    /// Stable local report id.
    pub id: String,
    /// Creation time in Unix milliseconds.
    pub created: i64,
    /// Last mutation time in Unix milliseconds.
    pub updated: i64,
    /// Local review disposition.
    pub status: Status,
    /// Whether a review surface has shown this report.
    pub viewed: bool,
    /// Number of matching reports received in the session.
    pub count: u64,
    #[serde(flatten)]
    /// Bounded, redacted agent-authored fields.
    pub input: ReportInput,
    /// Bounded evidence snapshots for repeated calls.
    pub evidence: Vec<Evidence>,
}
/// A partial listing keeps malformed files visible by name.
#[derive(Debug, Default)]
pub struct Listing {
    /// Valid reports ordered newest first.
    pub reports: Vec<Report>,
    /// Names of unreadable reports skipped by the listing.
    pub malformed: Vec<String>,
}
/// Session-independent access to the owner-only feedback directory.
#[derive(Debug, Clone)]
pub struct FeedbackStore {
    root: PathBuf,
}
fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}
fn private(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "owner-only Smith feedback persistence is unavailable on this platform",
        ))
    }
}
impl FeedbackStore {
    /// Opening needs no live session and creates no files until a mutation.
    pub fn open(user_dir: impl AsRef<Path>) -> Self {
        Self {
            root: user_dir.as_ref().join("feedback"),
        }
    }
    fn lock(&self) -> io::Result<File> {
        if !self.root.exists() {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&self.root) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        if fs::symlink_metadata(&self.root)?.file_type().is_symlink() {
            return Err(invalid("feedback directory is a symlink"));
        }
        private(&self.root, 0o700)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let path = self.root.join(".lock");
        if path.is_symlink() {
            return Err(invalid("feedback lock is a symlink"));
        }
        let file = options.open(&path)?;
        private(&path, 0o600)?;
        file.lock_exclusive()?;
        Ok(file)
    }
    fn path(&self, id: &str) -> io::Result<PathBuf> {
        if id.len() != 18
            || !id.starts_with("hf-")
            || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(invalid("invalid feedback id"));
        }
        Ok(self.root.join(format!("{id}.md")))
    }
    /// Read one report without marking it viewed.
    pub fn show(&self, id: &str) -> io::Result<Report> {
        let path = self.path(id)?;
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || meta.len() > 16384 {
            return Err(invalid("invalid feedback file"));
        }
        let text = fs::read_to_string(path)?;
        let (header, body) = text
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---\n"))
            .ok_or_else(|| invalid("missing frontmatter"))?;
        let value: serde_json::Value = serde_json::from_str(header).map_err(io::Error::other)?;
        let lengths: Vec<usize> =
            serde_json::from_value(value["section_bytes"].clone()).map_err(io::Error::other)?;
        let mut report: Report = serde_json::from_value(value).map_err(io::Error::other)?;
        let mut rest = body;
        for ((label, field), length) in [
            ("What happened", &mut report.input.what_happened),
            ("User said", &mut report.input.user_said),
            ("Repro", &mut report.input.repro),
        ]
        .into_iter()
        .zip(lengths.iter())
        {
            rest = rest
                .strip_prefix(&format!("\n## {label}\n"))
                .ok_or_else(|| invalid("invalid feedback section"))?;
            let content = rest
                .get(..*length)
                .ok_or_else(|| invalid("invalid section length"))?;
            *field = content.into();
            rest = &rest[*length..];
            rest = rest
                .strip_prefix('\n')
                .ok_or_else(|| invalid("invalid section end"))?;
        }
        if lengths.len() != 3 || !rest.is_empty() {
            return Err(invalid("invalid feedback sections"));
        }
        if report.id != id || report.evidence.is_empty() {
            return Err(invalid("invalid feedback metadata"));
        }
        Ok(report)
    }
    /// List valid reports and malformed filenames without changing review state.
    pub fn list(&self) -> io::Result<Listing> {
        let mut result = Listing::default();
        let entries = match fs::read_dir(&self.root) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(result),
            Err(e) => return Err(e),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|v| v != "md") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            match self.show(path.file_stem().unwrap().to_str().unwrap_or("")) {
                Ok(r) => result.reports.push(r),
                Err(_) => result.malformed.push(name),
            }
        }
        result
            .reports
            .sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
        result.malformed.sort();
        Ok(result)
    }
    fn write(&self, report: &mut Report) -> io::Result<()> {
        loop {
            let mut metadata = serde_json::to_value(&*report).map_err(io::Error::other)?;
            metadata["section_bytes"] = serde_json::json!([
                report.input.what_happened.len(),
                report.input.user_said.len(),
                report.input.repro.len()
            ]);
            for key in ["what_happened", "user_said", "repro"] {
                metadata[key] = "".into();
            }
            let header = serde_json::to_string(&metadata).map_err(io::Error::other)?;
            let body = format!(
                "---\n{header}\n---\n\n## What happened\n{}\n\n## User said\n{}\n\n## Repro\n{}\n",
                report.input.what_happened, report.input.user_said, report.input.repro
            );
            if body.len() <= 16384 {
                let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
                private(temp.path(), 0o600)?;
                temp.write_all(body.as_bytes())?;
                temp.as_file().sync_all()?;
                temp.persist(self.path(&report.id)?)
                    .map_err(io::Error::other)?;
                File::open(&self.root)?.sync_all()?;
                return Ok(());
            }
            if report.evidence.len() > 1 {
                report.evidence.remove(0);
                continue;
            }
            // UTF-8 and escaped frontmatter can exceed the byte ceiling even
            // when every individual field is within its character ceiling.
            let mut value = serde_json::to_value(&*report).map_err(io::Error::other)?;
            fn shrink(v: &mut serde_json::Value) {
                match v {
                    serde_json::Value::String(s) if s.chars().count() > 80 => {
                        *s = bounded(s, s.chars().count() / 2)
                    }
                    serde_json::Value::Array(a) => a.iter_mut().for_each(shrink),
                    serde_json::Value::Object(o) => o.values_mut().for_each(shrink),
                    _ => {}
                }
            }
            shrink(&mut value);
            *report = serde_json::from_value(value).map_err(io::Error::other)?;
        }
    }
    /// Change disposition under the store lock.
    pub fn set_status(&self, id: &str, status: Status) -> io::Result<()> {
        let _lock = self.lock()?;
        let mut r = self.show(id)?;
        r.status = status;
        r.updated = Utc::now().timestamp_millis();
        self.write(&mut r)
    }
    /// Mark only the reports a review surface actually showed.
    pub fn mark_viewed(&self, ids: &[String]) -> io::Result<()> {
        let _lock = self.lock()?;
        for id in ids {
            let mut r = self.show(id)?;
            r.viewed = true;
            self.write(&mut r)?;
        }
        Ok(())
    }
    /// Redact and bound a report while serializing duplicate checks and pruning.
    pub fn record(
        &self,
        input: ReportInput,
        evidence: Evidence,
        redactor: Option<&dyn FeedbackRedactor>,
    ) -> io::Result<Report> {
        let _lock = self.lock()?;
        // Keep matching stable even when the byte ceiling shortens evidence.
        let session_key = format!("{:x}", Sha256::digest(evidence.session.as_bytes()));
        let normalized_title = input
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let duplicate_key = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(input.kind, &input.area, normalized_title))
                    .map_err(io::Error::other)?
            )
        );
        let mut value = serde_json::to_value((input, evidence)).map_err(io::Error::other)?;
        fn clean(value: &mut serde_json::Value, redactor: Option<&dyn FeedbackRedactor>) {
            match value {
                serde_json::Value::String(s) => {
                    if let Some(r) = redactor {
                        *s = r.redact_text(s);
                    }
                    *s = bounded(s, 2000);
                }
                serde_json::Value::Array(a) => a.iter_mut().for_each(|v| clean(v, redactor)),
                serde_json::Value::Object(o) => o.values_mut().for_each(|v| clean(v, redactor)),
                _ => {}
            }
        }
        clean(&mut value, redactor);
        let (mut input, mut evidence): (ReportInput, Evidence) =
            serde_json::from_value(value).map_err(io::Error::other)?;
        input.title = bounded(&input.title.replace(['\r', '\n'], " "), 120);
        evidence.failures.truncate(5);
        for f in &mut evidence.failures {
            f.error = bounded(&f.error, 500);
        }
        let now = Utc::now().timestamp_millis();
        let list = self.list()?;
        if let Some(mut r) = list
            .reports
            .iter()
            .find(|r| r.session_key == session_key && r.duplicate_key == duplicate_key)
            .cloned()
        {
            r.count = r.count.saturating_add(1);
            r.updated = now;
            r.viewed = false;
            r.evidence.push(evidence);
            if r.evidence.len() > 5 {
                r.evidence.remove(0);
            }
            self.write(&mut r)?;
            return Ok(r);
        }
        if list
            .reports
            .iter()
            .filter(|r| r.session_key == session_key)
            .count()
            >= 5
        {
            return Err(invalid("feedback session limit reached"));
        }
        let mut count = list.reports.len() + list.malformed.len();
        if count >= 500 {
            for r in &list.reports {
                if r.status != Status::Queued && r.updated < now - 30 * 86400 * 1000 {
                    fs::remove_file(self.path(&r.id)?)?;
                    count -= 1;
                }
            }
        }
        if count >= 500 {
            return Err(invalid("feedback store is full"));
        }
        let id = loop {
            let bytes = uuid::Uuid::new_v4();
            let alphabet = b"abcdefghijklmnopqrstuvwxyz234567";
            let suffix: String = bytes.as_bytes()[..6]
                .iter()
                .map(|v| alphabet[(v & 31) as usize] as char)
                .collect();
            let id = format!("hf-{}-{suffix}", Utc::now().format("%Y%m%d"));
            if !self.path(&id)?.exists() {
                break id;
            }
        };
        let mut r = Report {
            session_key,
            duplicate_key,
            id,
            created: now,
            updated: now,
            status: Status::Queued,
            viewed: false,
            count: 1,
            input,
            evidence: vec![evidence],
        };
        self.write(&mut r)?;
        Ok(r)
    }
}
