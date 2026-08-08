---
created_at: 2026-08-07T00:00:00Z
updated_at: 2026-08-07T00:00:00Z
---

## Why

Delegation is the only way Smith runs work in parallel, and a child is a
different agent: a different system prompt, a different tool view, and an empty
conversation. That is right for "go find out how the UI layer is wired" and
wrong for "try this same thing two ways from where we already are".

Two costs follow from having only the child primitive.

**Everything is re-derived.** A child starts knowing nothing. The parent may
have spent thirty tool calls establishing what the codebase looks like; the
child reads the same files again, because the only thing that crossed over was
a paragraph of task text the parent had to write from memory.

**Nothing is cached.** The provider prompt cache keys on a prefix — system
prompt, then tools, then messages. A child differs in the first two before a
single message is compared: `prepare_child_profile_routes` builds its prompt
with `delegation: false`, and the coordinator strips the `agent` tool from
every child view to hold the depth-one invariant. So a child is a guaranteed
full-price cold read of an entire prefix the parent has warm.

What is actually wanted is a *branch of this conversation*: the same agent, at
the same point, carrying the same history, continuing down a second path. That
is not a child with better arguments — it is a different primitive.

## What Changes

### The fork primitive

- Add a fork to the shared runtime's delegation vocabulary: a session built
  from the parent's own resolved policy, seeded with the parent's canonical
  history as of the fork point, running one task and reporting a result the way
  a child does.
- A fork's system prompt, tool declarations, model, and context policy are the
  parent's, unmodified. That identity is the whole point: it is what makes the
  shared prefix cache-eligible.
- Forks reuse the existing child lifecycle — spawn, wait, result, stop, the
  inbox delivery at a safe boundary, the panel row, the inspector — so a fork
  is a new origin for a delegated session, not a second lifecycle to maintain.

### Holding depth-one without breaking the prefix

A fork inherits the parent's tool declarations, so it inherits the `agent` tool
— which is exactly what the coordinator strips from children to keep delegation
one level deep. Stripping it from a fork would change the tool block and
forfeit the cache hit the fork exists for.

- Keep the `agent` tool *declared* in a fork's view and refuse it at invoke
  time with a typed error. The prefix stays byte-identical; the invariant is
  enforced where it can be enforced without touching the prefix.
- A fork therefore cannot spawn children, cannot fork again, and cannot deepen
  the tree, exactly as a child cannot.

### Reaching it

- Expose fork as an `agent` action — `agent(fork, task)` — so the model can
  branch when it recognizes two paths worth trying from the same state.
- Report the branch point on the fork's transcript row and panel row, so a fork
  is visibly a branch of this conversation rather than another sub-agent.
- Leave a user-facing branch command out of this change. The same primitive
  would serve one, and that is a reason to get the primitive right first, not a
  reason to build both at once.

### Honest cache reporting

- Attribute a fork's cache-read tokens to the fork, so the saving is visible
  rather than asserted. A fork that missed the cache must be as legible as one
  that hit it.

## Impact

- Affected specs: `child-agents`, `prompt-cache`, `usage-accounting`,
  `client-surfaces`
- Affected code: `agent-runtime` — `ChildSpec`/`DurableChildSpec` gain a fork
  origin, `DelegationCoordinator` gains fork spawn and the declared-but-refused
  delegation path, the child monitor gains branch-point attribution;
  `crates/smith-runtime` — `SmithChildFactory` gains a fork builder that reuses
  the parent's runtime policy rather than a child route, `AgentTool` gains the
  action; `crates/smith-tui` — fork rows on the transcript and panel
- **This requires a shared-runtime change.** `agent-runtime` is pinned by git
  rev in the workspace manifest, so this lands upstream first and arrives here
  as a rev bump. Nothing in this change is implementable in this repository
  alone.
- Compatibility: additive. No existing spawn, child, or resumed session changes
  behavior, and a runtime without fork support keeps working unchanged.
- Durability: a fork seeded from parent history has a materially larger
  checkpoint than a child. Retention and the durable-child store need a
  deliberate answer before this ships, not after.

## Open Questions

These are design decisions this proposal deliberately does not settle, because
they change what gets built:

1. **Branch point.** Always the parent's current head, or an addressable
   earlier point? Head-only is far simpler and covers "try this two ways from
   here". An addressable point turns fork into conversation history editing.
2. **Fork concurrency.** Do forks share the child concurrency cap
   (`DEFAULT_MAX_RUNNING_CHILDREN`, currently 4), or get their own? A fork is
   cheaper per token but heavier per checkpoint.
3. **Write access.** A fork has the parent's exact tool declarations, which for
   a build-posture root means write tools. Is a fork allowed to write to the
   shared workspace concurrently with its parent, or is it read-only regardless
   of what it declares?
4. **Result shape.** Does a fork return prose like a child, or does the parent
   adopt the branch — replacing its own head with the fork's? The second is
   powerful and much harder to make safe.

## Approval Boundary

This proposal is not approvable as implementation. It asks for agreement on the
primitive — a branch of this conversation, seeded with its history, sharing its
prefix, refusing delegation at invoke time to hold depth-one — and for answers
to the four open questions. Implementation tasks and spec deltas follow once
those are settled and the upstream runtime change has a shape.

Approving the primitive does not authorize weakening the depth-one invariant,
letting a fork hold any permission the parent does not, editing or rewriting
parent history, or presenting an unverified cache hit as a saving.
