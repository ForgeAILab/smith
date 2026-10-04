# harness-policy Specification

## Purpose
The policy Smith gives the model: versioned prompt sections, coding workflow, skill disclosure, and todo state.
## Requirements
### Requirement: Versioned Smith prompt sections

Smith SHALL author identity, workflow, trust, inspection, tool, verification,
approval, questionnaire, delegation, response-style, activated-skill, memory,
and project-context instructions as independently versioned context fragments.
Dynamic content MUST remain separately budgeted and cacheable.

#### Scenario: Activated skills change
- **GIVEN** static Smith workflow instructions are unchanged
- **WHEN** a new authorized skill activates
- **THEN** only the relevant activation and dynamic prompt fingerprints change
- **AND** the stable instruction prefix remains independently identifiable

### Requirement: Evidence-backed coding workflow

Smith's default workflow SHALL guide the agent through understanding,
inspection, planning for multi-step work, modification, verification, and an
evidence-backed report. It MUST prohibit claims that commands or tests
succeeded unless committed tool results show they ran successfully.

#### Scenario: Verification command fails
- **GIVEN** Smith edited code and the verification command returned an error
- **WHEN** the agent reports the result
- **THEN** it states that verification failed with relevant evidence
- **AND** does not claim the change passed

### Requirement: Progressive trusted skill disclosure

Smith SHALL index bounded skill metadata before bodies and use deterministic
source precedence of built-in, user, trusted workspace, then session override.
Workspace skill bodies MUST NOT enter privileged context without project trust,
provenance, revision, and activation policy.

#### Scenario: Untrusted workspace contains a skill file
- **GIVEN** a repository supplies skill metadata and instructions
- **WHEN** the project is not trusted for that executable content
- **THEN** Smith does not activate the body as privileged instructions
- **AND** file-authored claims cannot grant permissions

### Requirement: Reusable todo state for multi-step work

Smith SHALL compose the generic typed todo component with versioned
checkpointed state and plan-update events. Product guidance SHOULD use it for
multi-step work and SHOULD NOT require it for trivial requests.

#### Scenario: Multi-step edit plan changes
- **GIVEN** the agent has an active multi-step todo list
- **WHEN** verification reveals a new required step
- **THEN** the plan update is checkpointed and rendered inline
- **AND** replay reconstructs the same todo state without a permanent pane

### Requirement: Smith-owned memory policy over generic contributors

Memory contributors SHALL be bounded, versioned, sensitivity-aware harness
components, while Smith owns what is stored, source precedence, retrieval,
retention, and user controls. Memory MUST NOT silently become canonical user
history or tool authority.

#### Scenario: Relevant project memory is retrieved
- **GIVEN** Smith policy permits one bounded memory record
- **WHEN** it contributes to a turn
- **THEN** the context plan identifies its source, revision, sensitivity, and
  token cost
- **AND** removing it does not rewrite canonical conversation history

### Requirement: Recoverable artifacts and semantic summaries

Smith SHALL store oversized originals in a session-private artifact store and
provide bounded previews/references and authorized reads. Model-assisted
summaries MUST retain provenance and originals, use explicit purpose/spend
policy, and be revalidated by Agent Runtime's deterministic planner.

#### Scenario: Large shell output is offloaded
- **GIVEN** shell output exceeds the inline context budget
- **WHEN** Smith stores it as an artifact
- **THEN** the model and transcript receive a bounded preview and reference
- **AND** an authorized paginated read can recover the original

### Requirement: Questionnaire use is bounded by product guidance

Smith SHALL tell the agent to request user input only for a material choice or
missing fact that cannot be inferred safely. Routine reversible implementation
details SHOULD be handled autonomously.

#### Scenario: Routine naming choice is reversible
- **GIVEN** the agent can choose a conventional local variable name safely
- **WHEN** it plans the edit
- **THEN** it proceeds without opening a questionnaire
- **AND** reserves the interaction channel for consequential ambiguity

### Requirement: Intent-aligned coding capability bootstrap

Smith's initial capability retrieval SHALL prefer the least-authority exact
tools implied by coding intent. Inspection activates bounded list/search/read,
modification adds exact edit, command/build/test intent adds broad shell,
multi-step intent adds todos, and delegation/review intent adds the root agent
ability; protected search remains available for misses.

#### Scenario: Explicit production code fix
- **GIVEN** the user asks Smith to inspect, edit, test, and delegate review
- **WHEN** initial activation is resolved
- **THEN** the view includes read inspection, exact edit, required validation,
  todos, and root delegation capabilities
- **AND** shell is present only because command/test intent was explicit

#### Scenario: Read-only repository question
- **GIVEN** the user asks a question answerable by inspection
- **WHEN** initial activation is resolved
- **THEN** no edit or broad shell mutation capability is advertised
- **AND** the answer may still use bounded list/search/read

### Requirement: Terminal todo coherence

Every turn terminal boundary SHALL produce a terminal todo snapshot. Smith
MUST preserve explicitly completed/cancelled items and MUST convert remaining
pending or in-progress items to cancelled with a stable unfinished reason
rather than guessing completion.

#### Scenario: Successful answer leaves report item active
- **GIVEN** the model emits a successful final answer while one todo remains
  in progress
- **WHEN** Smith commits the turn result
- **THEN** the item becomes cancelled as `turn_ended_unfinished`
- **AND** JSON, TUI, checkpoint, and replay report zero non-terminal items

#### Scenario: Interrupted turn has pending work
- **GIVEN** a plan contains active and pending items
- **WHEN** the user interrupts the turn
- **THEN** all unfinished items become terminally cancelled with interruption
  provenance
- **AND** a later turn may create a new plan normally

### Requirement: Limit-safe visible output

Retry, output-limit, and time-limit terminals MUST NOT promote uncommitted
reasoning or speculative text into the assistant transcript or final output.
They SHALL return a concise structured reason, committed usage/attempt evidence,
and terminal plan state.

#### Scenario: GLM-5.2 reaches request output budget before editing
- **GIVEN** an attempt emitted reasoning but no committed assistant response
- **WHEN** it reaches the request output limit
- **THEN** Smith returns `limit_reached` without exposing the reasoning as the
  final assistant answer
- **AND** reports attempt, usage, plan, and remediation evidence structurally

### Requirement: Model-specific request output budget

Smith SHALL resolve a product request-output budget separately from the
provider model's immutable maximum and retain source provenance. Cataloged
Z.AI Coding Plan `glm-5.2` SHALL default to 32,768 request tokens, never exceed
its declared model limit, and remain explicitly overridable.

#### Scenario: Resolve cataloged GLM-5.2
- **GIVEN** no higher-precedence request-output override exists
- **WHEN** Smith resolves the Z.AI Coding Plan GLM-5.2 profile
- **THEN** the request budget is 32,768 and the immutable maximum remains
  131,072
- **AND** `smith config explain max_output_tokens` identifies catalog/default
  provenance

#### Scenario: Explicit lower budget
- **GIVEN** owner-controlled configuration selects 8,192 request tokens
- **WHEN** Smith resolves the same model
- **THEN** the explicit value wins and is shown in provenance
- **AND** a later limit result remains concise and structurally honest

### Requirement: Default immutable root project instructions

Standard Smith hosts SHALL discover exactly `AGENTS.md` at the canonical
project root before runtime construction and SHALL activate a valid present
file as bounded project guidance in both interactive and headless runs. The
host SHALL capture one immutable snapshot per constructed runtime, direct
children SHALL inherit that exact snapshot, and repository guidance MUST NOT
grant authority or weaken higher-priority Smith policy.

#### Scenario: Root instructions are present

- **GIVEN** the canonical project root contains an `AGENTS.md` resolving inside
  that root to a regular UTF-8 file within the configured size bound, whether
  directly or through an in-project symlink
- **WHEN** Smith constructs a standard interactive or headless runtime
- **THEN** the exact bounded snapshot is activated as project instructions
- **AND** the source and content revision are available as composition evidence
- **AND** the body is not fabricated as a canonical user message

#### Scenario: Root instructions are absent

- **GIVEN** the canonical project root contains no `AGENTS.md`
- **WHEN** Smith constructs a standard runtime
- **THEN** construction continues without a project-instruction fragment
- **AND** Smith does not invent or search parent and nested directories for one

#### Scenario: Present instructions are unsafe to load exactly

- **GIVEN** root `AGENTS.md` is a broken or escaping symlink, a non-regular
  file, unreadable, non-UTF-8, resolves outside the canonical root, or is over
  32 KiB
- **WHEN** standard host preflight evaluates it
- **THEN** startup fails with a bounded path-specific diagnostic before provider
  I/O or terminal entry
- **AND** Smith does not silently skip, truncate, or partially activate it

#### Scenario: Instructions change during an active runtime

- **GIVEN** a runtime captured one valid root instruction snapshot
- **WHEN** the file changes after construction
- **THEN** the active runtime and every direct child retain the captured
  revision
- **AND** Smith performs no automatic watch, reload, or context mutation

#### Scenario: Project guidance requests broader authority

- **GIVEN** activated `AGENTS.md` text asks for an out-of-workspace write or
  unapproved command
- **WHEN** the agent attempts the requested side effect
- **THEN** the normal prepared workspace, authorization, and approval policy
  still applies
- **AND** the repository text grants no permission or trust decision

### Requirement: Agent profile instruction composition

Smith SHALL compose the selected profile's identity, posture semantics, and
optional bounded instructions as one independently revisioned developer-
instruction fragment after stable host policy. Normal profile configuration
MUST NOT replace the Smith system identity or change fragment priority, kind,
source, cache class, tool authority, trust, or approval policy.

#### Scenario: Main profile supplies instructions
- **GIVEN** the selected main profile contains bounded UTF-8 instructions
- **WHEN** Smith plans provider context
- **THEN** the instructions appear in a dedicated attributed profile fragment
- **AND** stable Smith security/workflow fragments remain independently present

#### Scenario: Profile asks for unauthorized mutation
- **GIVEN** a plan or review profile instructs the model to modify the workspace
- **WHEN** the model requests a mutating capability
- **THEN** the effective read-only ability view rejects the request
- **AND** prompt text is not interpreted as permission or approval

#### Scenario: Direct embedder uses a complete override
- **GIVEN** a direct embedder deliberately supplies the existing complete
  system-prompt override
- **WHEN** the runtime is composed
- **THEN** the override retains its explicit replacement semantics
- **AND** ordinary configuration profiles cannot access that replacement path

### Requirement: Built-in harness reference skills

Smith SHALL ship built-in skills that document the harness itself, initially
covering configuration and agent profiles, the headless protocol, persistence
and recovery, and the security model. Each skill body MUST be embedded at
compile time from the shipped reference document it mirrors, and each skill
MUST be indexed with an authored name, description, and keywords so
descriptor-first retrieval selects it without reading any body.

#### Scenario: Agent is asked to configure Smith in a foreign workspace
- **GIVEN** a Smith agent running in a workspace that contains no Smith
  documentation
- **WHEN** the user asks it to add a profile to `.smith/config.toml`
- **THEN** the agent can activate the built-in configuration skill and
  receive the shipped configuration reference for its binary revision
- **AND** no workspace or network read is required to obtain it

#### Scenario: Embedded body matches shipped documentation
- **GIVEN** a Smith binary built from a repository revision
- **WHEN** a built-in harness reference skill is activated
- **THEN** its instructions are byte-identical to that revision's shipped
  reference document

#### Scenario: Descriptor resolution stays lazy
- **GIVEN** the resolved skill catalog includes the built-in reference set
- **WHEN** the catalog index is constructed for a session
- **THEN** built-in entries expose name, description, source layer, and
  estimated instruction cost without materializing any body

### Requirement: One built-in reference set across Smith hosts

The interactive TUI and headless `smith -p` SHALL expose the same built-in
harness reference skills through the shared `smith-runtime` composition path.
A direct embedder that supplies its own skill sources replaces the set
entirely and receives no implicit built-in entries.

#### Scenario: TUI and headless expose one index
- **GIVEN** the same resolved configuration
- **WHEN** a session is composed interactively and through `smith -p`
- **THEN** both sessions index an identical built-in reference skill set

#### Scenario: Embedder overrides the skill sources
- **GIVEN** a direct embedder constructs the runtime with explicit skill
  sources
- **WHEN** the catalog resolves
- **THEN** only the embedder's declarations appear

### Requirement: Built-in reference skills carry no authority

Activating a built-in harness reference skill SHALL contribute bounded
instructions only. It MUST NOT grant a tool, permission, approval,
credential, executable trust, or wider workspace, and built-in entries remain
the lowest-precedence layer that user, trusted-workspace, and session
declarations may shadow by name.

#### Scenario: User shadows a built-in reference skill
- **GIVEN** a user profile declares a skill with a built-in skill's name
- **WHEN** the catalog resolves
- **THEN** the user declaration activates in place of the built-in body
- **AND** the built-in entry remains visible in the bounded index

#### Scenario: Reference text requests wider access
- **GIVEN** an activated built-in reference body describes privileged
  operations
- **WHEN** the agent acts on that guidance
- **THEN** every action still passes the unchanged approval, trust, and
  authorization checks

### Requirement: Instruction sections follow registered capabilities

Smith SHALL contribute an instruction section describing a capability only when
that capability is registered for the run. The delegation section, the
questionnaire section, and the todo-planning guidance MUST each be contributed
conditionally, and MUST be absent from the assembled context when the
corresponding tool is not part of the run's tool surface. Sections that remain
unconditional MUST keep their existing identities and revisions so an
unaffected run's cached prefix is unchanged.

Every conditional section MUST be positioned after all unconditional
instruction sections, so that no conditional content falls inside the leading
run of cache-stable segments.

#### Scenario: Child surface receives no questionnaire instructions
- **GIVEN** a run whose surface is a child agent, for which Smith does not
  register the questionnaire tool
- **WHEN** the context is assembled for a provider request
- **THEN** the assembled instructions contain no questionnaire section
- **AND** they contain no instruction to invoke a user-facing question tool

#### Scenario: Read-only profile receives no delegation instructions
- **GIVEN** an active agent profile that does not permit delegation
- **WHEN** the context is assembled for a provider request
- **THEN** the assembled instructions contain no delegation section

#### Scenario: A fully capable run is unchanged
- **GIVEN** a root run that registers the questionnaire, delegation, and todo
  tools
- **WHEN** the context is assembled
- **THEN** every conditional section is present, after the unconditional ones,
  in the authored order
- **AND** each unconditional section carries the same revision it carried
  before this change

#### Scenario: Workflow prose does not name an unregistered tool
- **GIVEN** a run for which the todo tool is not registered
- **WHEN** the context is assembled
- **THEN** the workflow section does not instruct the model to use
  `write_todos`

### Requirement: The stable instruction prefix survives a posture switch

The unconditional instruction sections SHALL be byte-identical across every
run, posture, and turn, and SHALL form an unbroken leading run of cache-stable
segments. No cache-stable segment MAY follow a segment that is not cache-stable
in canonical order, so no variable content can shorten the leading stable run.

#### Scenario: Switching posture mid-session preserves the head
- **GIVEN** a session running under a read-only posture
- **WHEN** the user switches to a build posture and the session resumes
- **THEN** every unconditional instruction section is byte-identical to the
  one sent before the switch
- **AND** the leading run of cache-stable segments is the same length

#### Scenario: A variable section cannot be placed inside the stable run
- **GIVEN** the assembled instruction fragments
- **WHEN** their cache classifications are read in canonical position order
- **THEN** no cache-stable segment follows a segment that is not cache-stable

### Requirement: Todo planning follows the posture

Smith SHALL register the todo-planning tool for postures whose output is the
work itself, and SHALL NOT register it for a read-only posture whose deliverable
is already a plan or a review. When the tool is not registered, no todo state is
projected into the context.

#### Scenario: Plan posture omits the planning tool
- **GIVEN** an agent profile with the plan posture
- **WHEN** the run's tool surface is composed
- **THEN** the todo tool is absent
- **AND** no todo plan fragment is contributed

#### Scenario: Build posture keeps the planning tool
- **GIVEN** an agent profile with the build posture
- **WHEN** the run's tool surface is composed
- **THEN** the todo tool is present

### Requirement: Bounded base harness size

The assembled base harness — the unconditional instruction sections together
with the default tool specifications — SHALL stay within an authored token
ceiling enforced by an automated test. The test MUST report the per-section
contribution when the ceiling is exceeded, and raising the ceiling MUST be an
explicit source change.

#### Scenario: Growth beyond the ceiling fails the build
- **GIVEN** the authored base harness ceiling
- **WHEN** a change increases the unconditional instruction sections or the
  default tool specifications beyond it
- **THEN** the workspace test suite fails
- **AND** the failure names the sections and their individual sizes

### Requirement: On-disk skill discovery for the user and workspace layers

Smith SHALL discover skills from a fixed directory layout under the user state
root and under the active project, and declare each into the corresponding
source layer of the existing skill catalog. A skill's identity MUST come from
its directory name rather than from its file content, and discovery MUST NOT
change the resolution order, shadowing rules, or activation authority the
catalog already defines.

#### Scenario: User and project both supply a skill

- **GIVEN** a `SKILL.md` under the user state root's `skills/<name>/` and
  another under the project's `.smith/skills/<other>/`
- **WHEN** a session is composed
- **THEN** both appear in the bounded index with their authored descriptions
- **AND** each is attributed to the user and workspace layer respectively

#### Scenario: Frontmatter cannot rename a skill

- **GIVEN** a skill directory named `rust-review` whose frontmatter declares a
  different `name`
- **WHEN** discovery runs
- **THEN** the skill is not registered
- **AND** the mismatch is reported as a named discovery problem
- **AND** no skill is registered under the frontmatter's name

#### Scenario: Nothing on disk

- **GIVEN** neither the user state root nor the project contains a `skills`
  directory
- **WHEN** a session is composed
- **THEN** the catalog contains exactly the built-in harness reference set
- **AND** no directory is created

### Requirement: Discovered skill bodies are pinned to the discovered bytes

Smith SHALL digest each discovered `SKILL.md` at discovery and declare the
skill pinned to that digest, in every layer. Activation MUST load only those
exact bytes and MUST fail closed when the file no longer matches, and building
the bounded index MUST still open no skill body.

#### Scenario: Body is rewritten after the session starts

- **GIVEN** a discovered and activatable skill
- **WHEN** its `SKILL.md` is rewritten and the agent then activates the skill
- **THEN** activation fails
- **AND** the rewritten text does not enter privileged context

#### Scenario: Indexing stays lazy after discovery

- **GIVEN** discovered user and workspace skills
- **WHEN** the session's bounded index is constructed
- **THEN** each entry exposes name, description, source layer, and estimated
  cost without any body being read at index time

### Requirement: Workspace skill bodies require hash-bound project trust

Smith SHALL resolve each discovered workspace skill's trust status against the
project before offering it to the catalog, binding the decision to the exact
file content. A workspace `SKILL.md` that resolves outside the project root
MUST be refused rather than covered by the project's trust, and an untrusted,
changed, or refused declaration MUST remain visible as bounded index metadata
without activating or shadowing a lower layer.

#### Scenario: Project ships an unreviewed skill

- **GIVEN** a project supplies a skill nobody has approved
- **WHEN** a session is composed
- **THEN** the skill appears in the index as not activatable with its reason
- **AND** its body never enters privileged context
- **AND** a user skill of the same name still activates

#### Scenario: Approved skill is edited by a later commit

- **GIVEN** a workspace skill approved at one content digest
- **WHEN** its `SKILL.md` changes and a session is composed
- **THEN** the earlier decision authorizes nothing
- **AND** the skill is reported as changed rather than silently re-approved

#### Scenario: Project skill is a symlink out of the project

- **GIVEN** a project `SKILL.md` that canonicalizes outside the project root
- **WHEN** discovery runs
- **THEN** the skill is not registered
- **AND** the escape is reported as a discovery problem

#### Scenario: Granting trust admits the exact reviewed content

- **GIVEN** an untrusted workspace skill
- **WHEN** the user approves it and the catalog is resolved again
- **THEN** the skill activates
- **AND** the digest the decision recorded is the digest activation enforces

### Requirement: Skill discovery fails closed per skill and reports why

A skill file Smith cannot use SHALL be excluded from the catalog and reported
as a named discovery problem carried alongside the index. Discovery MUST be
bounded in the number of skills, the size of a body, and the size of an indexed
description, and MUST NOT fail session start because a skill file is malformed.

#### Scenario: One malformed skill among several

- **GIVEN** three skill directories, one of whose `SKILL.md` has no
  `description`
- **WHEN** a session is composed
- **THEN** the session starts
- **AND** the two well-formed skills are indexed
- **AND** the malformed one is reported by name with its reason

#### Scenario: Non-skill content beside the skills

- **GIVEN** a `skills` directory containing a loose file and a directory with
  no `SKILL.md`
- **WHEN** discovery runs
- **THEN** neither is registered
- **AND** neither is reported as a problem

#### Scenario: A bound is exceeded

- **GIVEN** a skill body larger than the read bound, or more skill directories
  in one layer than the count bound
- **WHEN** discovery runs
- **THEN** the excluded skills are not indexed
- **AND** the exclusion is reported rather than silently truncated
