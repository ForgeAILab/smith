# Security threat model

Smith is a local coding agent that combines untrusted repository content,
remote model output, user credentials, filesystem/process tools, persistence,
and optional child sessions. Its central security rule is that text is not
authority: repository instructions, model output, skill front matter, tool
arguments, and questionnaire answers cannot grant a permission.

## Trust boundaries

| Boundary | Smith's posture |
| --- | --- |
| Repository/workspace | Untrusted input until host policy says otherwise |
| User configuration | May select credentials, persistence, and automation authority when owner-only |
| Remote provider | Receives the planned model context; output is untrusted |
| Runtime registry | Sealed, deterministic descriptors and activation epochs |
| Approval UI | Exact immutable prepared action; the only interactive authority decision |
| Questionnaire UI | Task information only; never authority |
| Session snapshot/journal | Owner-only and redacted, but not a secret store |
| Protected checkpoint | Encrypted/authenticated exact in-flight state |
| Artifact store | Owner-only content plus runtime-enforced session ownership |
| Child session | Depth-one, scoped tool view, separate artifact ownership |
| Trusted native module | In-process Rust with Smith's ambient process authority; never a sandbox |
| MCP server | Explicitly trusted external process/service; unreviewed tools carry possible external-write and egress authority |
| User-installed executable plugin | Refused until the capability-brokered subprocess protocol exists |
| Skill/declarative panel | Bounded content only; no executable authority or runtime/renderer handle |

## Authorization and tool execution

All executable tools pass through Agent Runtime's central executor. The
sequence is validation, canonical preparation, concrete permission/resource
calculation, authorization, optional approval, and execution of that exact
prepared action. Privileged composition fails closed without an authoritative
security check.

Filesystem resources are canonical segments over an explicit mount, not
unchecked string prefixes. Ordinary filesystem tools operate relative to an
opened project-directory capability; traversal, symlink escape, magic-link
escape, and component replacement are refused rather than converted into a
host-root approval. Reads inspect and consume one opened handle under a
`max_bytes + 1` limiter. Edits carry an identity, size, timestamp, and content
hash from that same read and revalidate immediately before publication. Atomic
replacement preserves supported metadata and synchronizes the file and parent
directory. Shell cannot be safely narrowed from command text and
is not sandboxed by its working directory. It therefore declares an explicit
`host-shell` resource with same-user host filesystem, inherited environment and
credentials, child-process, network, and data-egress authority. The resource ID
binds the command, canonical cwd, background flag, timeout, and inherited
environment policy by digest without reproducing command text. Every shell
invocation gets a process group, deadline, bounded output, and group
cancellation.

An approval shows the exact resource, material arguments or patch, typed
permissions, broad-authority warnings, fingerprint, and deadline. `y`/`a`
authorize only the immutable prepared evidence. Any argument edit requires
preparation and authorization again. Approval queues are FIFO and every
responder resolves exactly once on decision, timeout, cancellation, or
shutdown.

Headless `ask` cannot wait for a user and exits 4 with redaction-safe prepared
metadata. `allow-all` is appropriate only inside an already isolated and
trusted automation boundary. The explicit `--yolo` alias selects that same
approval mode; it bypasses approval prompts — including the prompts that gate
host-shell execution — but does not bypass authorization, ordinary-tool
workspace containment, or profile capability narrowing. In particular, a
`plan` profile remains read-only under `--yolo`.

## Repository and configuration attacks

Smith does not execute configuration, derive authority from repository text,
or treat a repository as trusted because it was opened. Project-controlled
files cannot:

- set `approval.mode = "allow-all"` or add `[[approval.auto]]` grants;
- disable or redirect user session persistence;
- store an inline API key;
- redirect the provider through authorization-bearing headers.

Project skills are indexed with provenance but activate only under explicit
workspace trust. Untrusted skills cannot shadow a user skill. Skill metadata
and bodies are bounded and revision-pinned; front matter cannot promote its own
trust class.

A `<project>/.smith/skills/<name>/SKILL.md` is the one path by which repository
content can ask to become part of the model's instructions, so it is decided
like any other project-supplied authority: hash-bound project trust, recorded
against the file's content and its project-relative path together. Editing an
approved skill invalidates the decision rather than inheriting it, and a
`SKILL.md` resolving outside the project root — a symlink aimed at a user skill
or at anything else on the machine — is refused rather than covered by the
project's trust. The skill's own text is never shown as the argument for
trusting it; the confirmation shows the path and the digest. A skill is named
by its directory, so a project cannot use frontmatter to point its shadowing at
a name the reader does not see in the path.

Every discovered body, user skills included, is pinned to the bytes discovery
read. A body rewritten between indexing and activation fails closed instead of
entering context unreviewed. Discovery reads only: a malformed skill file is
excluded and reported, never a reason a session cannot start, and never a
prompt in an unattended run.

The standard host deliberately activates root `AGENTS.md` as declarative
project guidance. It accepts only an exact regular UTF-8 file up to 32 KiB that
resolves inside the project root — an in-project symlink such as `AGENTS.md ->
CLAUDE.md` is followed, one escaping the workspace is refused — captures it
once before runtime construction, and gives direct
children the same immutable revision. It performs no nested discovery,
automatic include expansion, or file watching. This activation changes model
guidance, not authority: instructions cannot register tools, widen the
workspace, alter approval policy, approve an action, expose a credential, or
authorize project executables. A changed file receives a new prompt/cache
identity only when a later runtime is constructed.

Capability activation is derived from current intent, trust, host readiness,
and policy. Registered mutation tools are not advertised for a read-only
request merely because they exist. Descriptor permission upper bounds are
checked against each prepared invocation and fail closed on expansion.

## Credentials and secret handling

Preferred credentials are opaque Keychain/Secret Service or environment
references. Plaintext `api_key` is an explicit no-prompt user-config option
only; the file must be a regular, non-symlinked, current-user-owned mode-0600
file. Setup masks input, renders `[redacted]`, and atomically restores config
and credential state on failed preflight.

Resolved secret types do not implement revealing display/debug/serialization.
Authorization-bearing header configuration is refused. Known secret literals
are registered with the persistence redactor. Events, default JSONL journals,
ordinary snapshots, errors, setup review, and headless machine output are
tested for non-disclosure.

A user-declared `command-jsonl` provider is trusted executable code, not a
sandboxed Smith tool. Its absolute executable, fixed argv, cwd, and explicit
environment may come only from owner-controlled user configuration; project
configuration may select the provider but cannot alter that invocation. Smith
uses direct argv, clears ambient environment, resolves declared environment
credentials through the same bounded secret boundary, and registers every
resulting value with persistence redaction before its compatibility probe.
These controls prevent accidental repository authority and secret inheritance,
but the process still has the Smith user's OS access and receives prompts,
history, tool schemas, and tool results on stdin. Selecting it is therefore a
data-egress and local-code trust decision. Smith's approval system governs tool
calls returned by the bridge; it cannot constrain what the executable itself
does outside that protocol.

Protected-checkpoint keys are independent of provider credentials. Smith may
load one from OS protected storage, the dedicated `SMITH_CHECKPOINT_KEY`
environment value, an owner-only inline `persistence.checkpoint_key`, or an
explicit `env:` checkpoint-key reference. Inline and environment routes are
mutually exclusive with the OS reference, never initialize or query
Keychain/Secret Service, and zeroize decoded key material. Project files may
not define or redirect any checkpoint-key source. Source changes refuse before
configuration mutation whenever encrypted checkpoints exist; Smith never
silently abandons unreadable state or falls back to plaintext.

Redaction is defense in depth, not permission to put secrets in observability.
Low-entropy argument fingerprints can confirm a guess and are correlation
metadata, not cryptographic secrecy.

ChatGPT subscription authentication is an explicitly experimental Smith-owned
integration. Browser login uses PKCE S256, an allow-listed localhost callback,
and exact state validation; device login uses the fixed issuer endpoints and a
bounded poll. Verifiers, callback queries, and authorization codes stay in
memory and are zeroized. Smith stores the access token, rotating refresh token,
expiry, and account ID as one versioned secret in the `chatgpt` entry of the
fixed plaintext `~/.smith/auth.json`. Smith constrains `~/.smith` to mode `0700`
and `auth.json` to mode `0600`, serializes writers, fsyncs atomic replacements,
and refuses symlink, non-regular, malformed, or oversized storage. These mode
bits do not protect against same-user processes or backups. Project
configuration cannot redirect the issuer, client, callback, endpoint, account
header, entry, or storage location. ChatGPT connect, resolution, refresh,
reconnect, and disconnect do not access Keychain or Secret Service, and Smith
does not read or delete the legacy `keychain:smith/chatgpt` entry.

The direct Responses adapter exposes only fixed authentication/protocol
classifications. Token values, refresh material, account IDs, callback data,
raw OAuth bodies, and raw provider error bodies are absent from errors, events,
render state, transcripts, snapshots, and logs. Acquired and rotated tokens are
registered with the persistence redactor. A current-revision 401 may trigger
one pre-output refresh replay; a stale rejection, second rejection, partial
stream, cancellation, or deadline expiry cannot loop or replay.

This endpoint is not a supported public OpenAI Platform API contract. Smith
does not launch Codex, import Codex/OpenCode caches, or represent subscription
tokens as Platform API keys. The normal Smith runtime remains the execution
owner for tools, approvals, checkpoints, goals, persistence, attachments,
steering, cancellation, and usage.

## Persistence and recovery attacks

Ordinary snapshots and journals are owner-only/redacted but may contain
conversation content. Exact pending actions and sensitive answers belong only
in the protected checkpoint, encrypted with XChaCha20-Poly1305 and
authenticated to project/session/turn identity. Corruption, wrong key, moved
records, unsupported schema, and key-service failure share a non-oracular
diagnostic.

Journal overflow and oversized records leave explicit markers; sequence gaps
are never silently replayed as complete history. Checkpoint watermarks are
published only after the prior journal prefix is synced. Cross-process
lifecycle and writer leases prevent concurrent session owners from racing
state.

Artifacts are content-integrity checked and authorized by requesting session;
an opaque reference is not a bearer token. Child-to-parent transfer performs an
explicit bounded copy with source lineage. Semantic summaries store exact
source groups first and use a separate tool-free model purpose and usage
ledger.

Durable child records are parent-owned and policy-fingerprinted. Startup only
reconciles metadata; it does not construct a provider or execute a tool.
Follow-up reuses an idle child's canonical history, while exact resume requires
a compatible safe checkpoint and explicit confirmation. Missing, corrupt,
regressed, indeterminate-provider, or incompatible state fails closed without
spawning a replacement. The parent's cross-process lifecycle lease and the
runtime's one-coordinator lease prevent competing child continuations.
Journal-only legacy children and process monitor markers are interrupted on
restart, not re-executed. Smith does not claim recovery for arbitrary shell
side effects.

## Provider and prompt-injection risks

The selected provider sees Smith policy, activated root project instructions,
activated skills/memory, conversation, tool schemas, bounded results, and any
user content sent to it. `AGENTS.md` is not copied into canonical conversation
history, but its prompt fragment is provider input. Do not place secrets there
or send data to a provider whose retention and access policy you do not accept.

Provider output is untrusted. It can request tools but cannot bypass schema
validation, capability activation, authorization, approval, workspace
containment, deadlines, or output bounds. Failed attempt output remains
speculative until a commit event and is discarded on retry.

The local interactive transcript may resolve a Smith built-in tool call from
canonical in-process history to explain what ran. Before display, the host
clones the arguments and applies the same credential-shaped-key and registered
exact-secret scrubbing used by persistence. The bounded typed projector may
then show ordinary operational values such as paths, read windows, search
patterns, commands, flags, and timeouts. API keys, authorization values,
tokens, passwords, credentials, private keys, bearer values, secrets, and
registered literals render only as `[redacted]`. Bulk edit bodies and tool
results remain outside compact rows, and unknown tool schemas receive no
guessed value projection.

This local enrichment does not enable raw tool arguments in runtime events,
journals, observability, headless text, JSON, or stream JSON. Request-time and
completion-time lookup use the same stable call ID and redacted projector, so
retrying presentation enrichment cannot change execution or disclosure policy.

The `@file` and leading-`!` composer shortcuts are not alternate authority
paths. Attachments execute the canonical exact prepared `read`; local shell
executes the canonical prepared `host-shell` action with its host-wide
permission bound, approval, deadline, cancellation, output offload, and
checkpoint semantics.
Neither shortcut spends provider tokens during local preparation. Registered
Child-enabled `@profile` presets are host-resolved, read-only, depth-one
children and require explicit provider-spend confirmation. A profile may name
another declared provider/model, but Smith completes the same credential,
catalog, limit, context, and provider preflight before allocating the child.
The effective capability view is parent authority intersected with the
read-only direct-child ceiling and profile posture. Profile instructions are
provider input, not permission; raw bodies stay out of ordinary status, debug,
journals, and canonical user history.

Questionnaire answers resume task reasoning but grant no permission or
remembered approval. Child agents inherit a scoped tool view, capacity and
deadline limits, no nested delegation, and root-only direct questionnaire
readiness by default.

## Residual risks and deployment responsibilities

- An approved shell can run arbitrary host programs, read or change any
  same-user file available to the Smith process (including data outside the
  project), inspect inherited credentials, spawn children, and access the
  network. Use OS/container sandboxing when that authority is too broad.
- Another process running as the same OS user may read ordinary session files
  and plaintext user-config credentials.
- A compromised provider or credential service is outside Smith's process
  boundary.
- Redaction cannot remove an unknown secret that was never classified or
  registered.
- Owner-only files do not protect against host compromise, backups, or an
  administrator.
- Smith currently has no general network broker or fully isolated process
  backend; approval and workspace policy do not substitute for one.
- Monitor reconciliation exists, but Smith does not infer or restart a monitor
  executor from persisted metadata.

For storage details, see
[`persistence-recovery.md`](persistence-recovery.md). For unattended authority
and redaction, see [`headless-protocol.md`](headless-protocol.md).
