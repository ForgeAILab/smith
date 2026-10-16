# tool-execution Specification

## Purpose
How tools run: preparation, authorization, workspace policy, shell and background execution, and result bounds.
## Requirements
### Requirement: Minimal built-in coding tools

Smith SHALL provide built-in file read, path/text search, patch application,
and shell command tools implementing Agent Runtime's neutral `Tool` contract.
Every tool MUST publish a stable name, description, effects, input schema, and
bounded structured outcome.

#### Scenario: Apply a project patch

- **GIVEN** the agent submits a valid patch inside the allowed project root
- **WHEN** approval policy permits the write
- **THEN** Smith applies the patch atomically where practical
- **AND** records a structured result and affected paths

### Requirement: Scoped execution context

Every tool call MUST use the shared invocation context carrying workspace,
deadline, cancellation token, output limit, approval decision, and request
identity. A Smith tool MUST NOT infer broader filesystem authority from the
process working directory.

#### Scenario: Tool targets outside its scope

- **GIVEN** a filesystem tool is restricted to one project root
- **WHEN** its resolved target escapes that root
- **THEN** Smith rejects the call before mutation
- **AND** records a permission failure

### Requirement: Approval before material side effects

Smith SHALL inject a configurable approval policy into Agent Runtime. The
shared executor MUST evaluate it before shell execution or filesystem mutation;
an action requiring confirmation remains pending until approved and fails
closed when no allowing policy is available.

#### Scenario: Interactive mutation requires confirmation

- **GIVEN** a patch is classified as requiring approval
- **WHEN** the agent requests it in the TUI
- **THEN** Smith displays the exact tool, scope, and material arguments
- **AND** executes only after user approval

#### Scenario: Headless approval cannot be collected

- **GIVEN** `smith -p` has no TTY and no policy authorizing a shell command
- **WHEN** the agent requests that command
- **THEN** Smith returns an approval-required result and stable non-success
  outcome
- **AND** never hangs waiting for input

### Requirement: Bounded cancellable processes

Shell and monitor commands MUST run in Smith-owned process groups on macOS and
Linux. Smith SHALL enforce deadlines and output limits and MUST terminate the
owned group on cancellation or confirmed shutdown.

#### Scenario: Shell command spawns a child process

- **GIVEN** a shell command starts a subprocess
- **WHEN** the tool call is cancelled
- **THEN** Smith terminates the owned process group within the cleanup grace
  period
- **AND** records whether forced termination was required

### Requirement: Side-effect-aware tool scheduling

Smith SHALL configure Agent Runtime's side-effect-aware scheduler. Independent
read-only tools MAY run concurrently, but shared execution MUST serialize or
reject calls whose declared write scopes overlap unless an explicit conflict
policy allows another deterministic outcome.

#### Scenario: Two patches overlap

- **GIVEN** one model turn requests two patches to the same file
- **WHEN** Smith schedules the calls
- **THEN** it does not apply them concurrently
- **AND** preserves deterministic result order

### Requirement: Explicit edit operations

The `edit` tool SHALL accept an explicit `operation` of `replace`, `create`,
`overwrite`, or `delete`, defaulting to `replace` when absent. Each operation
MUST request only the permissions it needs: `replace` and `overwrite` request
filesystem read and write, `create` requests filesystem create and write, and
`delete` requests filesystem read and delete. An empty `old_string` MUST
continue to mean `create` so existing transcripts replay unchanged.

#### Scenario: Overwrite replaces a file without echoing its contents
- **GIVEN** an existing project file the session has read in full
- **WHEN** the model calls `edit` with `operation` `overwrite` and a new body
- **THEN** the file contains exactly the new body
- **AND** the call did not require the previous contents as an argument

#### Scenario: Create still refuses an existing target
- **GIVEN** an existing project file
- **WHEN** the model calls `edit` with `operation` `create`
- **THEN** the call fails
- **AND** the existing file is unchanged

#### Scenario: Delete requests the narrow permission
- **GIVEN** a prepared `edit` call with `operation` `delete`
- **WHEN** the prepared action is presented for authorization
- **THEN** it requests filesystem read and delete only
- **AND** it requests neither process spawn nor network

#### Scenario: A legacy empty old_string still creates
- **GIVEN** a recorded call passing an empty `old_string` and no `operation`
- **WHEN** it is replayed
- **THEN** the file is created exactly as before this change

### Requirement: Destructive operations require a current full read

`overwrite` and `delete` SHALL be refused unless the session has already read
the exact target path in full during this session, and the file's modification
time is not newer than that read. A partial read using an offset or a limit MUST
NOT satisfy the precondition. The refusal MUST name which condition failed.

#### Scenario: Overwrite without a prior read is refused
- **GIVEN** an existing file the session has not read
- **WHEN** the model calls `edit` with `operation` `overwrite`
- **THEN** the call fails with a message saying the file must be read first
- **AND** the file is unchanged

#### Scenario: A partial read does not authorize an overwrite
- **GIVEN** a file the session read with an `offset` and a `limit`
- **WHEN** the model calls `edit` with `operation` `overwrite`
- **THEN** the call fails
- **AND** the message distinguishes a partial view from an unread file

#### Scenario: An external modification invalidates the read
- **GIVEN** a file the session read in full
- **AND** the file was subsequently modified outside Smith
- **WHEN** the model calls `edit` with `operation` `overwrite`
- **THEN** the call fails with a message saying the file changed since it was
  read
- **AND** the external modification is preserved

#### Scenario: Exact replacement keeps its existing contract
- **GIVEN** a file the session has never read
- **WHEN** the model calls `edit` with `operation` `replace` and an
  `old_string` that matches exactly once
- **THEN** the edit applies
- **AND** no read precondition is imposed

### Requirement: Deletion is attributed like any other mutation

A completed `delete` SHALL be recorded in the turn's change set with the same
attribution as an exact edit, retaining the pre-image needed for conflict
checked undo and recording only hashes and path metadata in the persisted
journal.

#### Scenario: A deleted file can be undone
- **GIVEN** a file deleted by an `edit` call in the current session
- **WHEN** the user undoes that turn's changes
- **THEN** the file is restored with its exact previous contents

#### Scenario: The journal records no file contents
- **GIVEN** a completed `delete`
- **WHEN** the session journal is written
- **THEN** it contains the path metadata and content hashes
- **AND** it contains no file body

### Requirement: Explicit background shell execution

The `shell` tool SHALL accept an optional `run_in_background` boolean. When
true, Smith MUST validate and authorize the call exactly as a foreground call,
spawn the command in a Smith-owned process group, and return immediately with
a session-scoped task ID and a bounded output-spool reference instead of
waiting for completion. A background task SHALL run until it exits, is stopped
by `task_stop`, or the session shuts down; `timeout_ms` MAY still be supplied
to bound it. Smith MUST NOT convert a foreground call to background
automatically under any condition.

#### Scenario: Start a build in the background

- **GIVEN** the agent calls `shell` with a build command and
  `run_in_background: true`
- **WHEN** approval policy permits the command
- **THEN** the tool call returns promptly with a task ID and spool reference
- **AND** the build process keeps running after the tool call resolves

#### Scenario: Approval still gates a background command

- **GIVEN** a background shell request whose command requires confirmation
- **WHEN** the user has not yet approved it
- **THEN** no process is spawned
- **AND** the pending approval shows the same material arguments as a
  foreground call

#### Scenario: A bounded background task is killed at its deadline

- **GIVEN** a background task started with an explicit `timeout_ms`
- **WHEN** the deadline elapses before the process exits
- **THEN** Smith terminates the owned process group
- **AND** the terminal notification states the task was killed at its deadline

### Requirement: Background task lifecycle and notification

A background task's stdout and stderr SHALL spool to a bounded per-task
output file; exceeding the byte cap MUST truncate with an explicit marker, not
grow unbounded. When the task reaches a terminal state — exit, stop, deadline
kill, or session shutdown — Smith MUST emit exactly one terminal notification
through the session inbox carrying the task ID, terminal state, exit code when
available, and a bounded output tail. The journal SHALL record metadata-only
lifecycle markers for task start and terminal state, never output bodies.

#### Scenario: Task exits while the model is streaming

- **GIVEN** a background task exits during an in-flight provider response
- **WHEN** the terminal notification is published
- **THEN** the current response stream is not mutated
- **AND** the agent receives the notification at the next safe boundary

#### Scenario: Journal stays metadata-only

- **GIVEN** a background task produced output and exited
- **WHEN** the session journal is written
- **THEN** it contains the task's lifecycle markers and metadata
- **AND** it contains no spooled output body

### Requirement: Background task inspection and control

Smith SHALL provide a `task_output` tool returning a background task's status,
exit code when terminal, and a bounded incremental slice of its spooled output
addressed by offset, and a `task_stop` tool that terminates a running task's
owned process group by task ID. Both MUST fail with a stable error for an
unknown task ID. `task_stop` on an already-terminal task MUST be idempotent
and report the existing terminal state.

#### Scenario: Poll a running task's output

- **GIVEN** a running background task that has produced output
- **WHEN** the agent calls `task_output` with its task ID and a prior offset
- **THEN** it receives only the output after that offset
- **AND** the reported status is running with no exit code

#### Scenario: Stop a running task

- **GIVEN** a running background task
- **WHEN** the agent calls `task_stop` with its task ID
- **THEN** Smith terminates the owned process group within the cleanup grace
  period
- **AND** exactly one terminal notification reports the stopped state

### Requirement: Actionable foreground timeout outcome

A foreground shell command that reaches its timeout MUST still be killed with
its process group and returned as an error containing the output captured so
far. The outcome text SHALL state the elapsed limit and name the concrete
options: raising `timeout_ms` toward the documented maximum, narrowing the
command, or rerunning with `run_in_background: true`.

#### Scenario: Timed-out grep teaches the model its options

- **GIVEN** a foreground shell command killed at its 120000 ms default timeout
- **WHEN** the tool outcome is returned
- **THEN** it includes the partial output and names the timeout that elapsed
- **AND** it mentions `timeout_ms`, narrowing the command, and
  `run_in_background` as next steps

### Requirement: Image generation tool

Smith SHALL provide a `generate_image` tool that creates an image from a
prompt, or edits reference images, through the active provider's Images API.
It SHALL accept at most five references, given either as `reference_paths`
or as a count of `recent_images`, and MUST reject a call that supplies both.
Reference paths MUST resolve inside the workspace or Smith's generated-image
directory.

#### Scenario: Generate a new image

- **GIVEN** the active provider is ChatGPT
- **WHEN** the model calls `generate_image` with only a prompt
- **THEN** Smith posts to `{base}/images/generations` with the configured
  image model
- **AND** it saves the PNG under
  `~/.smith/generated_images/<session>/<call>.png`
- **AND** the tool result contains the image and its saved path

#### Scenario: Edit a referenced image

- **WHEN** the model calls `generate_image` with a prompt and one workspace
  image path
- **THEN** Smith posts to `{base}/images/edits` with that image as a data URL

#### Scenario: Both reference kinds supplied

- **WHEN** a call supplies both `reference_paths` and `recent_images`
- **THEN** the tool returns an error to the model without contacting the
  provider

### Requirement: Image tool availability

Smith SHALL register `generate_image` only when the active provider is ChatGPT
or the OpenAI Platform endpoint and image generation is enabled. The tool
MUST reuse that provider's credential lease and MUST NOT be offered in `plan`
posture.

#### Scenario: Unsupported provider

- **GIVEN** the active provider is a GLM OpenAI-compatible endpoint
- **WHEN** the tool list is built
- **THEN** `generate_image` is absent

#### Scenario: Plan posture

- **GIVEN** the active profile uses `plan` posture
- **WHEN** the tool list is built
- **THEN** `generate_image` is absent

### Requirement: Image tool results survive output bounding

The tool-output bound SHALL measure an image part by its image cost, not by
the length of its data URL. A generated image that fits the bound MUST
reach the model intact.

#### Scenario: Large data URL

- **GIVEN** a tool result with a 2 MB PNG data URL and a 32 KiB output limit
- **WHEN** the result is bounded
- **THEN** the image part is kept unchanged

### Requirement: Runtime-scoped background task services

Background shell, output, stop, exit-policy, and shutdown operations SHALL use
an explicitly injected task service owned by the composing Smith host. Tool
execution MUST NOT discover a first-install-wins process-global host or task
registry, and two hosts in one process MUST NOT observe or control each other's
tasks.

#### Scenario: Two embedded Smith hosts run tasks

- **GIVEN** two Smith hosts composed in one process with separate background
  services
- **WHEN** each starts a background task
- **THEN** each host lists, polls, stops, and shuts down only its own task
- **AND** installing or dropping one host does not change the other's service

#### Scenario: Direct embedder omits background service

- **GIVEN** a direct embedder supplies no background task service
- **WHEN** it resolves built-in capabilities
- **THEN** background-capable tools are omitted or fail preflight explicitly
- **AND** no ambient global service is consulted

### Requirement: Task stop returns acknowledged terminal state

A successful `task_stop` call for a running task SHALL wait within the bounded
cleanup period until the owned process group is terminated, one terminal state
is committed, and its terminal notification is enqueued. The first successful
result MUST report that terminal state and MUST NOT report `running`; an
already-terminal task remains idempotent and an unconfirmed cleanup returns an
error rather than false success.

#### Scenario: Stop a running task once

- **GIVEN** a task is running when `task_stop` is invoked
- **WHEN** the worker accepts and completes the stop
- **THEN** the first tool result reports `stopped`
- **AND** exactly one terminal journal record and notification exist

#### Scenario: Task exits while stop is requested

- **GIVEN** a task naturally exits while a stop request races it
- **WHEN** both terminal paths converge
- **THEN** `task_stop` returns the single winning terminal state
- **AND** no second terminal record or notification is emitted

#### Scenario: Process cleanup exceeds its bound

- **GIVEN** Smith cannot confirm process-group termination within the cleanup
  deadline
- **WHEN** `task_stop` reaches that deadline
- **THEN** it returns a bounded error or explicit unconfirmed outcome
- **AND** it does not claim that the task stopped

### Requirement: Truthful host-shell authority

Smith SHALL, until it supplies an enforceable operating-system sandbox, make
the `shell` tool declare an unrestricted same-user host-shell resource rather
than a workspace filesystem resource. Its approval presentation MUST state that the
command can access host files, inherited environment and credentials, child
processes, the network, and data egress outside the project.

#### Scenario: Shell cwd is inside the project

- **GIVEN** a shell call whose current directory resolves beneath the project
- **WHEN** Smith prepares the call without an operating-system sandbox
- **THEN** the security resource identifies host-shell execution
- **AND** it does not claim that the process is filesystem-contained by its cwd

#### Scenario: Headless shell lacks unrestricted policy

- **GIVEN** a headless run without explicit `allow-all` authority
- **WHEN** the model requests a shell command
- **THEN** the run returns a structured approval-required outcome
- **AND** no command process is spawned

### Requirement: Handle-relative workspace enforcement

Built-in filesystem tools SHALL enforce the project boundary through a
directory capability and handle-relative operations. Canonical pathname
validation MAY provide display and preliminary diagnostics but MUST NOT be the
final containment primitive, and a bounded read MUST inspect and consume one
opened object through a `max_bytes + 1` limiter.

#### Scenario: Symlink is swapped after validation

- **GIVEN** a repository path validated beneath the workspace
- **AND** an adversary replaces a traversed component with an escaping symlink
- **WHEN** the built-in tool opens or mutates the target
- **THEN** the handle-relative operation refuses the escape
- **AND** no object outside the workspace is read or changed

#### Scenario: Absolute path is outside the project

- **GIVEN** a built-in read, list, search, or edit call names an absolute path
  outside the workspace
- **WHEN** Smith prepares the ordinary filesystem tool
- **THEN** it refuses the call rather than routing it to approval
- **AND** host access remains available only through a separately labeled host
  capability

#### Scenario: File grows after metadata inspection

- **GIVEN** a file whose initial metadata is within the read limit
- **WHEN** the object grows while Smith reads it
- **THEN** Smith reads no more than `max_bytes + 1`
- **AND** returns the oversized-file outcome without retaining the full object

### Requirement: Version-checked metadata-preserving edits

A full read SHALL produce a version token from the same file handle as the
returned bytes. Overwrite, delete, and exact replacement MUST revalidate file
identity, size, modification time, and content hash immediately before commit;
a mismatch MUST leave the newer object unchanged. Replacing an existing file
MUST preserve the platform's required security-relevant metadata, including
permission and executable bits, and MUST durably order temporary-file and
parent-directory synchronization where supported.

#### Scenario: User edits during an approval prompt

- **GIVEN** Smith fully read a file and prepared an overwrite
- **AND** the user changes the file before approving the call
- **WHEN** invocation reaches the commit boundary
- **THEN** version revalidation refuses the overwrite as stale
- **AND** the user's newer bytes remain unchanged

#### Scenario: Changed bytes retain the same modification time

- **GIVEN** the target's bytes changed after Smith read it
- **AND** its reported modification time was restored to the prior value
- **WHEN** Smith revalidates the destructive edit
- **THEN** the content hash or file identity mismatch refuses the commit

#### Scenario: Replace an executable script

- **GIVEN** an existing executable script with supported security metadata
- **WHEN** an approved exact replacement or overwrite succeeds
- **THEN** its permission and executable bits are preserved
- **AND** required metadata is not silently discarded by the atomic rename

### Requirement: View image tool

Smith SHALL provide a `view_image` tool that asks the resolved vision model
one question about one image and returns the model's text answer. The call
SHALL name exactly one of these, and Smith MUST reject a call that names both
or neither:

- `image`: an image id from a placeholder.
- `path`: an image file inside the workspace or Smith's generated-image
  directory.

`question` SHALL be required, with 1 to 2,000 characters. The tool's
description SHALL tell the model that an `[image <id>: …]` placeholder in the
conversation can be inspected by passing that id as `image`. The vision request
SHALL carry a fixed instruction, the image, the image's pixel dimensions, the
name of the tool that produced the image when known, and the question. It
SHALL carry no conversation history. The instruction SHALL ask for positions
as pixel coordinates in the image's own dimensions. The tool SHALL declare
network and data-egress effects to the vision model's endpoint, plus a host
read when `path` is used, so that approval policy applies. Vision usage SHALL
be recorded against the session, the same way advisor usage is.

#### Scenario: Locate a control in a screenshot

- **GIVEN** a GLM main model, `vision = "google/gemini-3-flash"`, and a
  history placeholder `[image 3f9a2c1b: image/png 1440×900 not included in this request]`
- **WHEN** the agent calls `view_image` with `image = "3f9a2c1b"` and
  `question = "Where is the Save button?"`
- **THEN** Smith sends that PNG, its dimensions, and the question to the
  vision model
- **AND** the tool result is the model's text answer

#### Scenario: Workspace image

- **WHEN** the agent calls `view_image` with `path = "docs/mockup.png"` and
  a question
- **THEN** Smith reads the file after approval and sends it to the vision
  model

#### Scenario: Image no longer in history

- **GIVEN** an image id whose message was compacted away
- **WHEN** the agent calls `view_image` with that id
- **THEN** the tool returns an error to the model saying the image is no
  longer in session history
- **AND** no vision request is sent

#### Scenario: Path outside the allowed roots

- **WHEN** the agent calls `view_image` with `path = "/etc/hosts.png"`
- **THEN** the tool returns an error without reading the file or contacting
  the vision model

### Requirement: View image availability

Smith SHALL offer `view_image` through compiled-in module `vision`. At each
composition, the module SHALL mount only when the active model is text-only
and a vision model resolves. Otherwise it SHALL report itself inactive with
the reason. Turning the module off SHALL NOT change the image placeholders
sent to a text-only model.

#### Scenario: Vision main model

- **GIVEN** a main model that accepts image input
- **WHEN** the tool list is built
- **THEN** `view_image` is absent
- **AND** `/modules` reports `vision` inactive because the main model accepts
  images

#### Scenario: Switching to a text-only model mounts the tool

- **GIVEN** a session on a vision model and `vision = "google/gemini-3-flash"`
- **WHEN** the user switches with `/model` to a text-only model
- **THEN** after the rebuild, `view_image` is in the tool list

#### Scenario: Read-only posture

- **GIVEN** a text-only main model, a vision model, and `plan` posture
- **WHEN** the tool list is built
- **THEN** `view_image` is absent, because read-only posture admits no
  network or egress tools
- **AND** `/modules` reports `vision` inactive for read-only posture

#### Scenario: No vision model configured

- **GIVEN** a text-only main model and no `vision` selection
- **WHEN** the tool list is built
- **THEN** `view_image` is absent
- **AND** images still reach the model as placeholders

### Requirement: Web fetch tool

Smith SHALL provide a `web_fetch` tool that GETs one `http` or `https` URL and
returns its content as text. HTML SHALL be reduced to its main content and
converted to Markdown with links preserved. `text/*`, JSON, and XML SHALL be
returned as is. `image/*` SHALL be returned as an image part. Any other
content type SHALL be a tool error naming the type. The tool SHALL bound
each fetch at 5 MiB, 30 seconds, and 5 redirects. It SHALL send no cookies or
credentials, and SHALL use a fixed Smith User-Agent. The result SHALL begin
with the final URL, status, content type, and page title, followed by a
line marking the body as external, untrusted content from that URL.

#### Scenario: Read a documentation page

- **WHEN** the agent calls `web_fetch` with `https://docs.rs/serde/latest/serde/`
- **THEN** the result begins with the final URL, `200`, and `text/html`
- **AND** the body is Markdown of the page's main content with its links

#### Scenario: Oversized download

- **GIVEN** a URL serving 40 MiB
- **WHEN** the agent fetches it
- **THEN** Smith stops reading at 5 MiB and returns the converted prefix
  with a note that it was truncated

#### Scenario: Unsupported type

- **WHEN** the URL serves `application/zip`
- **THEN** the tool returns an error naming `application/zip`

### Requirement: Web fetch refuses private destinations

Smith SHALL resolve the destination of every request and redirect hop before
connecting. It SHALL refuse loopback, private, carrier-grade NAT,
link-local, unique-local, and unspecified addresses unless
`tools.web_fetch.allow_private_networks` is true. It SHALL connect to the
address it vetted. Only `http` and `https` URLs without userinfo SHALL be
accepted.

#### Scenario: Metadata address

- **WHEN** the agent fetches `http://169.254.169.254/latest/meta-data/`
- **THEN** the tool returns an error that the destination is a private
  address
- **AND** no connection is made

#### Scenario: Redirect into a private network

- **GIVEN** a public URL that redirects to `http://10.0.0.5/admin`
- **WHEN** the agent fetches it
- **THEN** the redirect is refused at the second hop

#### Scenario: Local docs server allowed

- **GIVEN** `[tools.web_fetch] allow_private_networks = true`
- **WHEN** the agent fetches `http://localhost:3000/docs`
- **THEN** the page is returned

### Requirement: Web search through a configured backend

When the search route is a backend, Smith SHALL provide a `web_search` tool
that takes `query` (1 to 400 characters) and an optional `max_results`
(1 to 10, default 5). It SHALL query the selected backend (DuckDuckGo, Brave,
Tavily, or SearXNG) and return each result's title, URL, and snippet,
labelled as external, untrusted content. Backend errors SHALL be tool errors
that carry the backend name and HTTP status, without credentials. DuckDuckGo
needs no key. Smith SHALL query DuckDuckGo by POSTing the search form to its
HTML endpoint without a User-Agent header, and SHALL retry once on its lite
endpoint when the HTML endpoint answers with a challenge page, a non-200
status, or a transport failure. Results that point back at DuckDuckGo (ads
and internal links) SHALL be dropped. A rate-limit or challenge from both
endpoints SHALL be reported as a tool error that names the keyed backends.

#### Scenario: Brave backend on a text-only provider

- **GIVEN** a GLM main model and `[tools.web_search] backend = "brave"` with a
  stored key
- **WHEN** the agent searches `tokio select cancellation safety`
- **THEN** Smith calls the Brave web search API with the key header
- **AND** returns up to 5 titled results with URLs and snippets

#### Scenario: Free search with no configuration

- **GIVEN** a GLM main model and no `[tools.web_search]` settings
- **WHEN** the agent searches `ratatui scrollbar example`
- **THEN** Smith queries DuckDuckGo with no key
- **AND** returns titled results with unwrapped target URLs

#### Scenario: DuckDuckGo lite fallback

- **GIVEN** DuckDuckGo's HTML endpoint answers HTTP 202 with its challenge page
- **WHEN** the agent searches
- **THEN** Smith POSTs the query to the lite endpoint
- **AND** returns the lite results with their direct URLs and snippets

#### Scenario: DuckDuckGo rate-limits

- **GIVEN** both DuckDuckGo endpoints answer with the challenge page
- **WHEN** the agent searches
- **THEN** the tool returns an error that DuckDuckGo rate-limited the
  request and names `brave`, `tavily`, and `searxng` as alternatives

#### Scenario: Search turned off

- **GIVEN** `[tools.web_search] mode = "off"`
- **WHEN** the tool list is built
- **THEN** `web_search` is absent and no hosted search is declared

### Requirement: User-chosen network modules in read-only posture

Smith SHALL let read-only postures keep tools from the reviewed modules
listed in `[agent] read_only_network_modules`. The list defaults to
`["web", "vision"]`. A tool qualifies only if its effects are network read
and data egress, with no filesystem or external write. Smith SHALL still
apply the approval policy to these tools. Unreviewed MCP and plugin tools
SHALL remain excluded from read-only posture whatever the list says.

#### Scenario: Plan posture reads documentation

- **GIVEN** a profile in `plan` posture and the default list
- **WHEN** the tool list is built
- **THEN** `web_fetch` is present
- **AND** `edit` and unreviewed MCP tools are absent

#### Scenario: Vision while planning

- **GIVEN** a text-only main model, a vision model, `plan` posture, and the
  default list
- **WHEN** the tool list is built
- **THEN** `view_image` is present

#### Scenario: User restores the strict filter

- **GIVEN** `[agent] read_only_network_modules = []` in user configuration
- **WHEN** a `plan` profile builds its tool list
- **THEN** neither `web_fetch` nor `view_image` is present

### Requirement: Host-scoped web approval

`web_fetch` SHALL prepare each call with the resource `web:<host>`, and
`web_search` with `web-search:<backend>`, together with network and
data-egress effects scoped to that destination. Prepared-call-scoped
automatic approval rules SHALL therefore be able to approve one host without
approving others.

#### Scenario: Auto-approve one documentation host

- **GIVEN** a user rule approving `smith/web_fetch` for resource `web:docs.rs`
- **WHEN** the agent fetches a `docs.rs` page and then a `example.com` page
- **THEN** the first call runs without a prompt
- **AND** the second call asks for approval

### Requirement: Read returns image files as images

The `read` tool SHALL return a file whose content is a PNG, JPEG, GIF or WebP
image as a prepared image part followed by one line naming the path, the
original pixel size, and the size, format and byte count sent. The type SHALL
be recognized from the file's signature, not its name. Any other binary
content SHALL keep the existing refusal. Workspace confinement and approval
SHALL be unchanged.

#### Scenario: Image-capable model reads a picture
- **GIVEN** a main model that accepts images and `photo.png` in the project
- **WHEN** the model calls `read` on `photo.png`
- **THEN** the tool result carries the image and a line describing it
- **AND** the next request sends the image to the model

#### Scenario: Text-only model reads a picture
- **GIVEN** a text-only main model and a configured vision model
- **WHEN** the model calls `read` on `photo.png`
- **THEN** the model receives the runtime's `[image <id>: …]` placeholder
- **AND** `view_image` can answer a question about that id

#### Scenario: A file named like an image is not one
- **GIVEN** `fake.png` whose bytes are not an image signature
- **WHEN** the model calls `read` on it
- **THEN** it is handled as before: text if it is text, a binary refusal
  otherwise

### Requirement: Images are fitted to the active provider

Smith SHALL prepare every image file it reads or attaches against the active
main model's provider limits, kept in one table: formats accepted, longest
side, pixel budget, and encoded size. An image that already fits SHALL be
sent unchanged. Otherwise Smith SHALL:
- downscale it to fit;
- convert a format the provider does not accept to PNG;
- reduce an animated GIF to its first frame;
- re-encode a PNG without transparency as JPEG at quality 85 when it is still
  too large.

A file over 64 MiB, an image over 100 megapixels, or a corrupt image SHALL be
refused with the reason before any provider request.

#### Scenario: Large photo for ChatGPT
- **GIVEN** the `chatgpt` provider and a 4032×3024 JPEG
- **WHEN** it is read or attached
- **THEN** the model receives a JPEG within 2048 px on its longest side and
  within 2,500 tiles of 32 px
- **AND** the description names both sizes

#### Scenario: Small screenshot passes through
- **GIVEN** a 1200×800 PNG within every limit
- **WHEN** it is read
- **THEN** the image part carries the file's original bytes

#### Scenario: Format the provider rejects
- **GIVEN** the `xai` provider and a WebP image
- **WHEN** it is attached
- **THEN** the model receives it as a PNG

#### Scenario: Decompression guard
- **GIVEN** a PNG that declares 20000×20000 pixels
- **WHEN** it is read
- **THEN** the result is an error naming the 100-megapixel limit
- **AND** the image is not decoded
