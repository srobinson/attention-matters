# Skills vs Instructions vs MCP: Where Should DAE Memory Behavior Live?

Research into how Claude Code skills work, how existing memory tools instruct agents,
and the best mechanism for teaching Claude Code WHEN and HOW to use its geometric
memory system.

Date: 2026-02-14

---

## 1. Claude Code Skills: What They Are and How They Work

### Definition

A skill is a markdown file (`SKILL.md`) with YAML frontmatter that Claude Code
discovers automatically. Skills provide **specialized domain knowledge** that Claude
loads on demand based on trigger conditions in the description field. Think of them as
"onboarding guides" that transform Claude from a general-purpose agent into a
specialist.

### File Format

```
~/.claude/skills/<name>/SKILL.md       # Global skills
.claude/skills/<name>/SKILL.md         # Project-local skills
<plugin>/skills/<name>/SKILL.md        # Plugin-bundled skills
```

Required YAML frontmatter:

```yaml
---
name: skill-name
description: Third-person description with trigger phrases. "This skill should be
  used when the user asks to 'query', 'remember', 'recall'..."
---
```

Optional frontmatter fields:

| Field                       | Effect                                           |
| --------------------------- | ------------------------------------------------ |
| `disable-model-invocation`  | Only user can invoke (for side effects)           |
| `user-invocable: false`     | Only Claude can invoke (background knowledge)     |
| `allowed-tools`             | Restrict which tools the skill can use            |
| `context: fork`             | Run in isolated subagent                          |

### Discovery and Loading (Three-Level Progressive Disclosure)

1. **Metadata (always loaded)**: `name` + `description` (~100 words) -- sits in
   context at all times so Claude knows the skill exists.
2. **SKILL.md body (on trigger)**: Loaded when Claude decides the skill is relevant
   based on the description's trigger phrases. Target <2000 words.
3. **Bundled resources (on demand)**: `references/`, `scripts/`, `examples/`,
   `assets/` -- Claude reads these only when it needs detailed info.

### Invocation

- **Model-invoked**: Claude reads the description, decides it's relevant to the
  current task, and loads the skill body automatically.
- **User-invoked**: User types `/skill-name` (or `/skill-name args`). The `$ARGUMENTS`
  variable captures anything after the name.
- **Both** (default): Either party can trigger it.

### Character Budget

Claude Code has a **15,000 character limit** for combined skill content loaded at
session start (metadata only). The body is loaded on-demand, so it doesn't count
against this budget until triggered.

### Key Insight from Existing Skills

Looking at the skills on this machine (`~/.claude/skills/`), the pattern is clear:
skills are best for **procedural workflows** -- multi-step recipes that Claude should
follow when a specific task type is detected. The nancy orchestrator skills
(`check-directives`, `send-message`, `orchestrator`) are good examples: they define
communication protocols between agents.

---

## 2. Comparison: Skill vs CLAUDE.md vs MCP Server Instructions

### CLAUDE.md

| Property          | Details                                                       |
| ----------------- | ------------------------------------------------------------- |
| **When loaded**   | Always, at session start. Part of system prompt.              |
| **Priority**      | High -- treated as authoritative instructions.                |
| **Best for**      | Persistent rules, conventions, architecture, commands.        |
| **Scope**         | `~/.claude/CLAUDE.md` (global), `./CLAUDE.md` (project).     |
| **Who writes it** | Human (manual maintenance).                                   |
| **Character cost** | Every character loaded every session. Expensive.              |

### Skills

| Property          | Details                                                       |
| ----------------- | ------------------------------------------------------------- |
| **When loaded**   | Description always; body on-demand when triggered.            |
| **Priority**      | Medium -- guidance, not authority.                             |
| **Best for**      | Procedural workflows, domain expertise, repeatable tasks.     |
| **Scope**         | Global, project-local, or plugin-bundled.                     |
| **Who writes it** | Developer/user.                                               |
| **Character cost** | ~100 words always; body only when relevant.                   |

### MCP Server Instructions

| Property          | Details                                                       |
| ----------------- | ------------------------------------------------------------- |
| **When loaded**   | At MCP server connection (session start).                     |
| **Priority**      | Varies by client -- Claude Code injects into system prompt.   |
| **Best for**      | Cross-tool relationships, operational patterns, constraints.  |
| **Scope**         | Per-server. Travels with the MCP server wherever it's used.   |
| **Who writes it** | Server developer (us). Embedded in code.                      |
| **Character cost** | Loaded once at connection. Keep concise.                      |

### MCP Tool Descriptions

| Property          | Details                                                       |
| ----------------- | ------------------------------------------------------------- |
| **When loaded**   | Always available once server connects.                        |
| **Priority**      | High for tool selection -- Claude reads these to decide which  |
|                   | tool to call.                                                 |
| **Best for**      | Per-tool usage guidance, parameter semantics.                 |
| **Scope**         | Per-tool.                                                     |
| **Who writes it** | Server developer (us). In the `#[tool(description = "...")]`. |
| **Character cost** | Always in context. Keep each description tight.               |

### Claude Code Hooks

| Property          | Details                                                       |
| ----------------- | ------------------------------------------------------------- |
| **When loaded**   | Event-driven: SessionStart, PreToolUse, Stop, etc.            |
| **Priority**      | Deterministic -- code runs, not suggestions.                  |
| **Best for**      | Automated actions that must happen reliably.                  |
| **Scope**         | Settings-level or plugin-level.                               |
| **Who writes it** | Developer/user.                                               |
| **Character cost** | Zero context cost (runs externally).                          |

### Claude Code Auto Memory & Session Memory

| Property          | Details                                                       |
| ----------------- | ------------------------------------------------------------- |
| **When loaded**   | Auto: MEMORY.md first 200 lines at session start.             |
|                   | Session: past session summaries recalled at start.            |
| **Priority**      | Background reference, not authoritative.                      |
| **Best for**      | Continuity between sessions for a single project.             |
| **Scope**         | `~/.claude/projects/<project>/memory/`                        |
| **Who writes it** | Claude itself (automatic). User can edit.                     |
| **Character cost** | 200 lines loaded every session.                               |

---

## 3. How Existing Memory Tools Handle Agent Instructions

### Mem0 MCP Server

**Approach**: Pure MCP tools + CLAUDE.md instructions.

Mem0 provides tools (`add_memory`, `search_memory`, `get_memories`, `update_memory`,
`delete_memory`) and relies on CLAUDE.md to teach the agent when to use them:

```markdown
## Memory Protocol
- At the start of each session, search for relevant memories
- When the user shares preferences, store them as memories
- When the user corrects you, update the relevant memory
```

Mem0's blog post (Feb 2026) documents the approach: configure `.mcp.json` for the
server, then optionally add CLAUDE.md guidance. The agent is expected to call
`search_memory` proactively when it detects topics it might have prior context for.
The result: **10x faster task completion, 90% token reduction** vs re-explaining
everything.

Key observation: Mem0 does NOT use a skill. It relies on the tool descriptions being
clear enough that Claude figures out when to use them, with CLAUDE.md as a nudge.

### Letta / MemGPT

**Approach**: Self-managing memory via tool calls, baked into the agent architecture.

Letta's MemGPT architecture gives the agent explicit memory tools:
- `core_memory_append` / `core_memory_replace` -- edit the agent's own system prompt
- `archival_memory_insert` / `archival_memory_search` -- long-term storage
- `conversation_search` -- search past messages

The agent's **system prompt itself** contains instructions about when to use these:

> "You have a limited context window. To remember things long-term, use
> archival_memory_insert. To recall past conversations, use conversation_search.
> Your core memory blocks are always visible -- update them as you learn new facts."

As of February 2026, Letta introduced "Context Repositories" -- git-backed memory
files that agents manage using their normal file/terminal tools. The insight: **files
are the most universal memory primitive**. Agents can use `grep`, `cat`, `sed` on
their own memory.

Key observation: Letta embeds memory behavior into the agent's system prompt. It's
not an opt-in feature; memory management IS the agent's operating mode.

### myAI Memory Sync

**Approach**: MCP server that synchronizes memory templates across Claude interfaces.

Stores preferences in local files, serves them via MCP. No special instructions
needed -- the server's tool descriptions are self-explanatory (`get_preferences`,
`update_preferences`).

### Generic Memory MCP Servers (basic-memory, etc.)

**Approach**: Hierarchical time horizons (daily/weekly/monthly/quarterly/yearly) with
automatic association. CLAUDE.md instructions tell Claude to query at session start
and store periodically.

### The "Memory Engineering" Pattern (2026 Consensus)

From the Medium article on Memory Engineering for AI Agents (Jan 2026), the emerging
consensus is that production memory systems need:

1. **Explicit tiers**: Working memory (in-context), short-term (session), long-term
   (persistent store).
2. **Controlled write policies**: Not everything should be remembered. Salience
   filtering is critical.
3. **Robust retrieval**: The agent needs to know WHEN to query, not just HOW.
4. **Safety boundaries**: Memory can drift, hallucinate, or become stale.

---

## 4. What Makes Agent Memory Instructions Effective

### The Core Challenge

The agent needs to know:

1. **WHEN to query memory** -- At session start? Before every response? When topics
   change?
2. **WHEN to store** -- After every exchange? Only for salient insights? On user
   request?
3. **HOW to integrate recalled context** -- Silently blend it in? Explicitly cite it?
   Ask the user to confirm?

### Patterns That Work

**Pattern 1: Session-Start Query ("Always Recall")**

```
At the start of each session, call am_query with a summary of the user's
first message to load relevant context. Use the returned context to inform
your responses without explicitly mentioning the memory system.
```

This is what Mem0 recommends. Simple, predictable. Downside: wastes a tool call on
sessions where memory isn't relevant.

**Pattern 2: Topic-Triggered Query ("Smart Recall")**

```
When the conversation touches a topic you might have prior context for,
query memory. Signs: the user references past work, uses project-specific
terminology, or asks you to continue something.
```

More efficient but harder to implement reliably. Requires the agent to recognize
"maybe I know about this" signals.

**Pattern 3: Continuous Buffering + Salient Marking**

```
After each exchange, buffer the user/assistant messages. When you identify
an insight, decision, or preference worth remembering long-term, mark it
as salient.
```

This is what our DAE system is designed for. The buffer accumulates conversation
naturally; salient marking is the quality gate.

**Pattern 4: Explicit Memory Commands ("On Demand")**

```
The user can say "remember this" or "what do you recall about X" to
explicitly trigger store/recall operations.
```

Lowest friction for the user when they want control. But misses the "ambient memory"
that makes the system feel intelligent.

### What Doesn't Work

- **Too-aggressive storage**: Storing everything creates noise. The LOCOMO benchmark
  shows that indiscriminate storage hurts retrieval accuracy.
- **Silent memory use**: If the agent uses recalled context but doesn't indicate it,
  the user can't correct stale memories.
- **Over-explaining**: "I found in my memory that you prefer tabs..." on every
  response gets annoying fast.
- **Query-on-every-turn**: Expensive and slow. Memory queries should be strategic.

---

## 5. Always-On vs On-Demand: The UX Pattern

### The Spectrum

```
Fully Automatic ────────────────────── Fully Manual
     |                                       |
  Letta/MemGPT                        "/remember X"
  (memory IS the agent)          (user explicitly invokes)
     |                                       |
  Session-start recall              Only when user asks
  + continuous buffering            "what do you recall?"
  + auto-salient marking
```

### Recommendation: Hybrid (Automatic Recall + Intentional Storage)

Based on the research, the pattern that works best for a coding agent:

**Recall: Semi-automatic**
- Query memory at session start with project context
- Query again when the user's message suggests prior context exists
- Never query on pure code-execution tasks (no benefit)

**Storage: Intentional with automation**
- Buffer every exchange automatically (cheap, lossless)
- Let the agent mark salient insights when it recognizes them
- Let the user explicitly say "remember this" for high-value items
- The buffer auto-creates episodes after N exchanges (our BUFFER_THRESHOLD = 5)

**Integration: Subtle but inspectable**
- Weave recalled context into responses naturally
- Don't announce "I recalled from memory that..." unless the user asks
- Make memory state inspectable via `am_stats` when the user wants it

---

## 6. Recommendation: The Layered Approach for DAE

A skill alone is wrong. CLAUDE.md alone is too expensive. MCP instructions alone are
too limited. The answer is a **layered strategy** where each mechanism handles what
it's best at.

### Layer 1: MCP Server Instructions (ship with the server)

The `instructions` field in `ServerInfo` is the right place for **operational
guidance** that should travel with the server regardless of which client uses it.

Current (too minimal):

```rust
instructions: Some(
    "DAE (Daemon Attention Engine) geometric memory system. \
     Query memories, strengthen connections, mark salient insights, \
     buffer conversations, ingest documents, and manage state."
        .into(),
),
```

Proposed (add workflow guidance):

```rust
instructions: Some(
    "DAE geometric memory system providing persistent context across sessions. \
     Workflow: 1) On session start, call am_query with the user's initial \
     topic to recall relevant context. 2) After each substantive exchange, \
     call am_buffer with user+assistant text. 3) When you identify a key \
     insight, decision, or user preference, call am_salient to promote it \
     to conscious memory. 4) After generating a response that references \
     recalled context, call am_activate_response to strengthen those \
     connections. Do not announce memory operations to the user unless asked."
        .into(),
),
```

This is concise, actionable, and travels with the server. It follows the MCP blog
post's guidelines: capture cross-tool relationships, document operational patterns,
keep it factual and model-agnostic.

### Layer 2: Tool Descriptions (already good, minor improvements)

The existing tool descriptions are solid. One improvement: make `am_query` explicitly
note that it should be the first call in a session:

```rust
#[tool(
    description = "Query the DAE geometric memory system. Call at session start \
    with the user's topic to load relevant context. Returns composed context \
    with conscious (salient insights), subconscious (related episodes), and \
    novel (unique connections) recall sections."
)]
```

And make `am_buffer` clarify the accumulation pattern:

```rust
#[tool(
    description = "Buffer a conversation exchange (user + assistant messages). \
    Call after each substantive exchange to accumulate conversation history. \
    After 5 exchanges, automatically creates a new episode from the buffer \
    and places it on the geometric manifold."
)]
```

### Layer 3: CLAUDE.md (project-level, lightweight)

Add a small block to the project's CLAUDE.md wherever DAE is configured as an MCP
server. This acts as the user's explicit instruction to Claude:

```markdown
## Memory (DAE)

This project has geometric memory via the `am` MCP server. At session start,
query memory with the current topic. Buffer exchanges as you go. Mark salient
insights. Don't narrate memory operations.
```

This is ~40 words. Cheap in context, clear in intent. It reinforces the MCP
instructions with the user's authority.

### Layer 4: Skill (NOT recommended as primary, but useful as opt-in)

A skill would be appropriate IF we wanted to provide a detailed "memory management
workflow" that Claude loads on-demand. But for DAE, the workflow is simple enough to
fit in the MCP instructions. A skill adds complexity without proportional value here.

**Exception**: A skill WOULD be valuable for advanced memory operations -- a
`/memory-audit` skill that guides Claude through inspecting, pruning, and organizing
the memory manifold. That's a procedural workflow (skill territory) rather than
ambient behavior (instruction territory).

### Layer 5: Hook (for guaranteed automation)

A `SessionStart` hook could automatically call `am_query` without relying on Claude
to remember to do it. This is the most reliable approach for "always recall at
session start":

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "am query \"session start\" 2>/dev/null | head -50",
            "timeout": 5
          }
        ]
      }
    ]
  }
}
```

The hook output gets injected into Claude's context as a system message. This
guarantees memory recall at session start even if Claude doesn't proactively call
`am_query`. The downside: it's a fixed query ("session start") rather than being
tailored to the user's first message.

A better hook approach for the future: a `UserPromptSubmit` hook on the first message
that injects memory context based on what the user actually asked about:

```json
{
  "UserPromptSubmit": [
    {
      "matcher": "*",
      "hooks": [
        {
          "type": "command",
          "command": "am query \"$USER_PROMPT\" 2>/dev/null | head -100",
          "timeout": 10
        }
      ]
    }
  ]
}
```

This is strictly better: it waits for the user's actual first message, queries memory
with that specific topic, and injects the results before Claude sees the prompt.

---

## 7. Summary: Decision Matrix

| Mechanism         | What to Put There                                      | Cost     |
| ----------------- | ------------------------------------------------------ | -------- |
| MCP instructions  | Workflow: when to query, buffer, mark salient           | Low      |
| Tool descriptions | Per-tool semantics and usage patterns                   | Low      |
| CLAUDE.md         | 2-3 lines: "This project has memory. Query on start."  | Minimal  |
| Skill             | Not for core behavior. Maybe `/memory-audit` later.    | On-demand |
| Hook              | SessionStart or UserPromptSubmit for guaranteed recall  | Zero     |

### The Minimum Viable Integration

For immediate implementation, the highest-impact changes are:

1. **Improve MCP `instructions`** in `server.rs` -- add the workflow guidance (Layer 1).
2. **Improve tool descriptions** -- make `am_query` say "call at session start" (Layer 2).
3. **Add 3 lines to CLAUDE.md** in projects using DAE (Layer 3).

These three changes require no new files, no new infrastructure, and no new concepts
for the user to learn. They work within the mechanisms already built.

### The Optimal Integration (Future)

Add the above, plus:

4. **A `UserPromptSubmit` hook** that queries memory with the user's first message
   and injects the recalled context. This makes memory truly automatic -- zero effort
   from the user OR from Claude's reasoning.
5. **A `/memory-audit` skill** for when the user wants to inspect, prune, or
   reorganize their memory manifold.
6. **A `PreCompact` hook** that calls `am_buffer` with the conversation summary
   before context compaction, ensuring nothing is lost when the context window fills.

---

## 8. What DAE Does Differently (and Why It Matters)

Most memory MCP servers (Mem0, basic-memory, etc.) are essentially key-value stores
with vector search. They store text, retrieve by similarity, done.

DAE's geometric memory system operates on fundamentally different principles:

- **S3 manifold positioning**: Words aren't just vectors; they have quaternion
  positions on a hypersphere with geodesic distance relationships.
- **Activation and drift**: Frequently co-activated words drift closer together on
  the manifold, creating organic clustering.
- **Kuramoto phase coupling**: Related concepts synchronize their phase oscillations,
  creating interference patterns that surface novel connections.
- **Conscious vs subconscious recall**: The system distinguishes between explicitly
  marked salient memories (conscious) and organically related context (subconscious).
- **IDF weighting**: Common words have less influence; distinctive terms carry more
  weight in recall.

This means the instructions need to teach Claude about the **qualitative difference**
between DAE recall and simple search. When `am_query` returns context with
"conscious", "subconscious", and "novel" sections, Claude should understand that:

- **Conscious**: things the user explicitly marked as important
- **Subconscious**: organically related context that surfaced through geometric
  proximity
- **Novel**: surprising connections that emerged from interference patterns

This understanding helps Claude weight the recalled context appropriately -- conscious
memories are high-confidence, novel connections should be mentioned tentatively.

---

## Sources

- MCP Server Instructions blog post: https://blog.modelcontextprotocol.io/posts/2025-11-03-using-server-instructions/
- Claude Code Skills reference: `~/.claude/plugins/marketplaces/claude-plugins-official/plugins/plugin-dev/skills/skill-development/SKILL.md`
- Mem0 Claude Code integration: https://mem0.ai/blog/claude-code-memory
- Letta Context Repositories: https://www.letta.com/blog/context-repositories
- Letta MemGPT architecture: https://docs.letta.com/guides/agents/architectures/memgpt
- Memory Engineering for AI Agents: https://medium.com/@mjgmario/memory-engineering-for-ai-agents
- Claude Code Session Memory: https://claudefa.st/blog/guide/mechanics/session-memory
- Claude Code Auto Memory: https://yuanchang.org/en/posts/claude-code-auto-memory-and-hooks/
- Adding Memory to Claude Code with MCP: https://medium.com/@brentwpeterson/adding-memory-to-claude-code-with-mcp
- Existing skill definitions: `~/.claude/skills/` (ralph orchestrator system)
- DAE MCP server: `crates/am-cli/src/server.rs`
