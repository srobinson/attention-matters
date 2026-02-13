# Prompting AI Agents to Use External Memory Effectively

Research findings on session lifecycle, query strategies, storage decisions, integration
naturalness, cold start, and coding-agent-specific memory patterns.

Compiled February 2026 for the DAE geometric memory system (`am-*` MCP tools).

---

## Table of Contents

1. [Theoretical Foundations](#1-theoretical-foundations)
2. [Session Lifecycle Patterns](#2-session-lifecycle-patterns)
3. [Query Strategies](#3-query-strategies)
4. [Storage Strategies](#4-storage-strategies)
5. [Integration Naturalness](#5-integration-naturalness)
6. [The Cold Start Problem](#6-the-cold-start-problem)
7. [Coding Agent Specifics](#7-coding-agent-specifics)
8. [Proposed Prompt Architecture for DAE](#8-proposed-prompt-architecture-for-dae)
9. [Anti-Patterns](#9-anti-patterns)
10. [Sources](#10-sources)

---

## 1. Theoretical Foundations

### 1.1 The MemGPT / LLM-as-OS Paradigm

The foundational work is Packer et al.'s **MemGPT** (2023), which reframes the LLM as an
operating system managing a memory hierarchy. The key insight: just as an OS provides
virtual memory by moving data between fast RAM and slow disk, an LLM agent can manage
virtual context by moving information between in-context memory (the prompt) and
out-of-context storage (external databases).

MemGPT introduces three tiers:
- **Core memory** (always in context) -- system prompt, persona, key facts
- **Recall memory** (searchable conversation history) -- past exchanges retrievable by query
- **Archival memory** (long-term storage) -- documents, notes, arbitrary data

The critical design decision: **the agent itself decides what to move in and out of
context via tool calls**. This "self-editing memory" is more flexible than static RAG
because the agent can write, update, and delete memories, not just read them.

The Letta framework (the production evolution of MemGPT) demonstrates that this pattern
works at scale: all agent state is persisted in a database, surviving server restarts
and enabling "agents-as-a-service."

### 1.2 Episodic Memory as the Missing Piece

Pink et al. (2025) argue in "Episodic Memory is the Missing Piece for Long-Term LLM
Agents" that current LLM memory systems over-index on semantic/factual retrieval and
underinvest in **episodic memory** -- the ability to recall specific experiences anchored
in time and context. They identify five key properties:

1. **Single-shot encoding** -- learning from a single experience without rehearsal
2. **Contextual binding** -- associating events with their spatial-temporal context
3. **Temporal ordering** -- maintaining the sequence of events
4. **Associative retrieval** -- recalling episodes triggered by partial cues
5. **Reconsolidation** -- updating memories when re-accessed in new contexts

This maps directly to DAE's geometric model: occurrences on the S3 manifold are
single-shot encoded with quaternion positions (contextual binding), organized into
episodes (temporal ordering), retrieved via geodesic distance (associative retrieval),
and subject to drift/activation dynamics (reconsolidation).

### 1.3 The Generative Semantic Workspace

Rajesh et al. (2025) propose the **Generative Semantic Workspace (GSW)**, a
neuro-inspired framework that builds structured, interpretable representations of
evolving narratives -- tracking entities through episodic events rather than just
retrieving isolated facts. This aligns with DAE's neighborhood/episode structure,
where words cluster into neighborhoods and neighborhoods compose into episodes that
represent coherent narrative units.

### 1.4 From RAG to Agent Memory

Monigatti (2025) traces the evolution clearly:

- **Vanilla RAG**: read-only retrieval at query time. The system fetches documents, the
  LLM never writes back.
- **Agentic RAG**: the agent decides *when* and *what* to retrieve, using tools. Still
  primarily read-only.
- **Agent Memory**: full read-write. The agent both retrieves and persists information.
  Memory becomes a first-class system component, not just a retrieval trick.

The key transition: memory stops being "extra text added to a prompt" and becomes a
managed resource with admission policies, lifecycle controls, and scoping rules.

### 1.5 Human-Like Remembering and Forgetting

Honda & Fujita (2026) propose an **ACT-R-inspired memory architecture** that emulates
human memory dynamics -- specifically, the way humans both remember and forget. Their
system uses activation-based decay (memories lose strength over time unless reinforced)
and interference (similar memories compete for activation).

This is strikingly parallel to DAE's design: occurrence activation decays over time,
IDF weighting creates interference between common and rare terms, and Kuramoto phase
coupling allows related memories to synchronize and reinforce each other. The research
validates the neuroscience-inspired approach: agents with human-like forgetting
outperform those with perfect recall, because selective forgetting improves signal-to-
noise ratio in retrieval.

Additionally, Wu et al. (2025) in "How Memory Management Impacts LLM Agents" provide
empirical evidence that **experience-following behavior** -- agents that selectively
retrieve and follow past experiences rather than reasoning from scratch -- significantly
improves task performance, but only when the memory management strategy filters for
quality experiences.

### 1.6 Memory Layering (Production Consensus)

Multiple independent sources converge on a 4-layer memory architecture:

| Layer | Contents | Lifetime | Storage |
|-------|----------|----------|---------|
| **Working memory** | Current task state, extracted constraints, partial plans | Single turn | In prompt/state |
| **Conversation memory** | Recent exchanges + rolling summary | Single session | Buffer + summary |
| **Task/artifact memory** | Decisions made, files generated, commands run | Per task | Structured logs |
| **Long-term memory** | User preferences, architectural patterns, learned insights | Indefinite | Persistent store |

Furmanets (2026): "Most teams say 'memory' and mean 'vector DB'. That's only one piece.
Vector retrieval is great for docs and long conversation recall. It's not great for
critical facts (use a DB), permissions/policies (use config), or money/transactions
(never)."

### 1.7 Context Engineering as the Successor to Prompt Engineering

The field is converging on "context engineering" as the broader discipline that
subsumes prompt engineering. The Manus team's engineering blog (2025) articulates this
shift: "The real challenge is not writing a good prompt; it's building a system that
assembles the right context at the right time." This means memory is not a feature
bolted onto an agent -- it is the primary mechanism by which context is engineered
across sessions.

Deepset (2026) formalizes this: context engineering is about curating data, memory,
and tools to create an informational environment where the LLM can respond effectively.
Memory is one of three pillars (alongside retrieval and tool outputs) that compose
the context at inference time.

---

## 2. Session Lifecycle Patterns

### 2.1 The Optimal Memory Loop

Research and production systems converge on a three-phase lifecycle:

```
SESSION START          DURING SESSION           SESSION END
     |                      |                       |
     v                      v                       v
  [Query]              [Buffer + Act]           [Consolidate]
  Retrieve relevant    Buffer exchanges,        Summarize session,
  context from         activate on responses,   mark salient insights,
  previous sessions    query when topic shifts  update long-term store
```

**Phase 1: Session Start -- "Recall"**

The agent should query memory at session start, using the user's first message (or
project context) as the query. This is the single highest-leverage memory operation.

From the MemGPT architecture: the agent's first action in any session is to compile its
context -- loading core memory blocks, searching recall memory for relevant conversation
history, and optionally searching archival memory for relevant documents.

The Letta framework makes this explicit: at each reasoning step, the system performs
"context compilation" -- assembling the prompt from memory blocks, message history, and
retrieved context.

Practical recommendation for DAE:
```
On first user message:
  1. am_query(user_message) -- search geometric memory
  2. Weave recalled context into response naturally
  3. am_buffer(user_message, response) -- begin buffering
```

**Phase 2: During Session -- "Engage"**

Not every exchange needs a memory operation. The research suggests:

- **Buffer substantive exchanges** -- conversations that contain decisions, preferences,
  insights, or problem-solving. Skip trivial confirmations and mechanical operations.
- **Query on topic shifts** -- when the conversation moves to a new domain, query memory
  again. The user's new topic might connect to something from a previous session.
- **Activate on responses** -- after generating a substantive response, strengthen the
  memory connections to the concepts involved. This is analogous to MemGPT's
  "heartbeat" mechanism, where the agent periodically processes and consolidates.

**Phase 3: Session End -- "Consolidate"**

The most underutilized phase. Most systems just stop. Better systems:

- Mark key insights as salient (promoting to long-term memory)
- Generate a session summary for future retrieval
- Identify unresolved questions or next steps

For coding agents specifically, this is where architectural decisions, bug patterns,
and project context should be crystallized.

### 2.2 The Heartbeat Pattern (MemGPT)

MemGPT introduced the concept of "inner thoughts" and "heartbeats" -- the agent
periodically gets processing time between user messages to:

1. Reflect on recent interactions
2. Update core memory with new information
3. Search archival memory proactively
4. Consolidate or clean up state

This is relevant for DAE: the `am_activate_response` tool serves a similar function
to the heartbeat -- it processes the agent's own output to strengthen relevant memory
connections.

### 2.3 Lifecycle for Coding Sessions Specifically

```
SESSION START:
  1. Read CLAUDE.md / project context (static)
  2. am_query(first_message + project_name) -- dynamic recall
  3. Integrate: "Last time on this project, we were working on X..."

DURING SESSION:
  - am_buffer() after substantive problem-solving exchanges
  - am_query() when user raises a topic that might have history
  - am_activate_response() after generating significant code or explanations

SESSION END (if detectable):
  - am_salient() for architectural decisions, patterns discovered, gotchas learned
  - am_buffer() final exchange summary
```

---

## 3. Query Strategies

### 3.1 When to Query

The research reveals three schools of thought:

**Always-query (MemGPT/Letta approach):**
Every session starts with a memory query. Every reasoning step includes context
compilation from memory. This is thorough but expensive.

**Query-on-relevance (Zep/Mem0 approach):**
The system uses heuristics or a lightweight classifier to decide whether a query is
likely to benefit from memory. Zep, for example, uses a graph-based approach where
facts and relationships are extracted and queried only when the topic overlaps with
stored knowledge.

**User-triggered (Claude CLAUDE.md approach):**
Memory is static -- loaded at session start from files. No dynamic querying. Simple
and predictable but limited.

**Recommended hybrid for DAE:**

| Situation | Action |
|-----------|--------|
| Session start | Always query with first user message |
| Topic shift within session | Query with new topic |
| User references past work | Query with reference context |
| Routine mechanical task | Skip query |
| User asks "do you remember..." | Always query |

### 3.2 Composing Good Queries

The quality of memory retrieval depends heavily on query composition. Research from
the GSW and episodic memory papers suggests:

- **Use the user's actual words** -- don't over-abstract. The user's specific phrasing
  may match stored episodic memories better than a sanitized summary.
- **Include project/domain context** -- "authentication bug in the payments service"
  retrieves better than just "authentication bug."
- **Temporal hints help** -- "what we discussed yesterday about caching" activates
  temporal associations in episodic memory.

For DAE's geometric model specifically, the query text gets tokenized and matched
against the manifold. Using the user's natural language preserves the token-level
associations that were encoded during ingestion.

### 3.3 What to Do with Recall Results

DAE's `am_query` returns three tiers -- conscious recall, subconscious recall, and
novel connections. Each should be handled differently:

- **Conscious recall** (high activation, strong match): Use directly. These are the
  most relevant memories. Integrate them into the response.
- **Subconscious recall** (moderate activation): Hold in working memory. Don't force
  these into the response, but let them inform reasoning. If they become relevant,
  surface them.
- **Novel connections** (unexpected associations): These are the "serendipity" layer.
  Occasionally mention these as "this reminds me of..." -- but sparingly. They're most
  valuable when they create genuine "aha" connections the user wouldn't have made.

---

## 4. Storage Strategies

### 4.1 The Admission Problem

The single most important lesson from production memory systems:

> "Memory is a product decision disguised as an engineering task." -- Velorum (2026)

Storing everything is worse than storing nothing. The failure mode is an agent that
"gets slower, weirder, and occasionally invasive" -- dredging up irrelevant preferences,
repeating outdated facts, or creating a creepy sense of surveillance.

### 4.2 The Three-Question Filter

Before persisting anything, apply three tests (synthesized from Sundar 2026, Furmanets
2026, and Mem0 best practices):

1. **Is it durable?** Will this still matter in a future session? Temporary instructions,
   session-specific decisions, and intermediate reasoning fail this test.

2. **Is it reusable?** Will this influence future decisions meaningfully? A one-time
   code fix is not reusable. A pattern for solving a class of problems is.

3. **Is it safe?** Does it avoid sensitive data, PII, secrets, and credentials?

Only information passing all three checks deserves long-term storage.

### 4.3 What to Buffer vs. What to Mark Salient

For DAE's two storage mechanisms:

**am_buffer (conversation exchanges):**
Buffer when the exchange contains:
- Problem-solving dialogue (diagnosis, debugging, architecture discussion)
- Decisions with rationale ("we chose X because Y")
- User preferences expressed naturally ("I prefer functional style")
- Teaching moments (user corrects the agent, explains domain knowledge)

Skip buffering for:
- Mechanical operations ("run the tests", "format this file")
- Routine confirmations ("yes", "looks good", "continue")
- Boilerplate code generation without discussion
- Error output / stack traces (the *diagnosis* matters, not the trace itself)

**am_salient (conscious-layer insights):**
Mark salient only for durable, high-signal insights:
- Architectural decisions ("this project uses event sourcing, not CRUD")
- Learned patterns ("this codebase wraps all DB access in repository traits")
- User working style preferences ("Stuart prefers three similar lines over premature abstraction")
- Project-specific gotchas ("the auth middleware must be registered before route handlers")
- Cross-project insights ("geometric memory queries work better with natural language than keywords")

**What makes a bad salient entry:**
- Anything time-bound ("currently debugging the login page")
- Implementation details without principle ("added a retry to line 47")
- Emotional/social observations ("user seemed frustrated")

### 4.4 The Summarize-Then-Store Pattern

From the ENGRAM paper (Patel & Patel, 2025) and production Mem0 implementations:
rather than storing raw conversation, extract and store the *insight*:

```
Raw exchange:
  User: "This keeps failing because the DB connection pool exhausts"
  Agent: "The pool is set to 5 connections but the service makes 8 concurrent queries..."
  User: "Bump it to 20 and add connection timeout of 30s"

Stored insight (salient):
  "Project X DB pool: 20 connections, 30s timeout. Exhaustion was caused by
   concurrent query count exceeding pool size."
```

This is lossy by design. The principle: **store the lesson, not the conversation.**

### 4.5 Forgetting and Decay

An underappreciated aspect of memory systems. DAE's activation/drift model handles
this naturally -- memories that are never re-activated drift and lose salience. This
is a feature, not a bug.

Furmanets (2026) recommends "retention windows" -- memories should expire unless
still useful. DAE's IDF-weighted drift and activation decay serve this purpose:
memories naturally fade unless reinforced by retrieval and use.

The `am_activate_response` tool is the mechanism for reinforcement: when the agent
uses recalled context in a response, activating on that response tells the memory
system "this was useful, keep it alive."

---

## 5. Integration Naturalness

### 5.1 The Spectrum from Invisible to Explicit

How should recalled context appear in the agent's responses? Research and practitioner
experience reveal a spectrum:

**Fully invisible (silent use):**
The agent uses recalled context to inform its response without ever acknowledging it.
The user experiences a smarter agent but doesn't know why.

- Pro: Feels natural, no cognitive overhead for user
- Con: User can't verify, correct, or build on the memory. Can feel "creepy" if the
  agent acts on information the user doesn't expect it to have.

**Lightly signaled:**
The agent uses phrases like "building on what we discussed" or "given your preference
for X" -- acknowledging memory without making it the focus.

- Pro: Transparent without being intrusive. User knows the agent remembers and can
  correct if needed.
- Con: Can feel formulaic if overused.

**Explicitly cited:**
"I recall from a previous session that we decided to use event sourcing for this
service. Should we continue with that approach?"

- Pro: Maximum transparency. User can confirm, update, or override.
- Con: Can feel robotic or like the agent is showing off.

### 5.2 Academic Evidence: Explicit Retrieval Outperforms Silent Integration

The **ReMemR1** paper (2025) provides empirical evidence that explicit memory
presentation -- where the agent's retrieved context is structured and clearly bounded
in the prompt -- outperforms silently weaving context into the response. In long-
document QA and multi-hop reasoning tasks, explicit retrieval cues improved accuracy
and reduced hallucination.

However, this finding is about *how* memory enters the agent's reasoning context
(explicit prompt structure), not necessarily *how* the agent communicates with the
user. The optimal design: memory is retrieved and presented explicitly to the agent
(structured recall blocks in the prompt), but the agent chooses how to surface it
to the user based on context.

### 5.3 Recommended Approach: Context-Dependent Transparency

The best practitioners use different levels depending on the situation:

| Situation | Transparency Level | Example |
|-----------|-------------------|---------|
| Continuing interrupted work | Explicit | "Last session we were refactoring the auth module. Shall we continue?" |
| Applying user preferences | Invisible | Just write code in their preferred style |
| Recalling architectural decisions | Lightly signaled | "Since this project uses repository pattern for DB access..." |
| Novel cross-session connection | Explicit | "This issue looks similar to the connection pool problem we solved in project Y." |
| Contradicting recalled information | Explicit | "Previously we decided X, but the current situation suggests Y. Want to revisit?" |

### 5.4 The "Eeriness" Threshold

Research on AI personalization consistently finds an "uncanny valley" for memory:

- Too little memory: frustrating ("I already told you this")
- Right amount of memory: delightful ("it just gets me")
- Too much memory: unsettling ("how does it know that?")

The threshold varies by context. For a coding agent:
- Remembering project architecture: expected and welcome
- Remembering debugging preferences: useful and appreciated
- Remembering an offhand remark from weeks ago: potentially creepy

Rule of thumb: **remember professional context aggressively, personal context
conservatively.**

### 5.5 Handling Memory Conflicts

When recalled information conflicts with current context:

1. **Always prioritize current session information** -- the user may have changed their
   mind, the codebase may have evolved, or the memory may be stale.
2. **Surface the conflict** -- "I recall we previously used approach X, but it looks
   like the code now uses approach Y. Should I follow the current pattern?"
3. **Update memory after resolution** -- use `am_salient` to record the new decision,
   which will naturally supersede the old memory through activation dynamics.

---

## 6. The Cold Start Problem

### 6.1 The Empty Memory Experience

First session with an empty memory system. No context to recall. Every query returns
nothing. The agent is identical to one without memory.

This is both a UX problem and a bootstrapping problem.

### 6.2 Strategies from Production Systems

**Graceful absence (recommended):**
Don't mention memory at all during cold start. The agent should work perfectly without
memory -- memory enhances, it doesn't enable. As context accumulates naturally through
conversation, the memory system populates organically.

From the Claude-Mem project (2026): "The system automatically captures what the agent
does, compresses it intelligently, and injects only relevant context when needed."
No special cold-start handling -- just normal operation that happens to build memory
as a side effect.

**Seed with project context:**
If project documentation exists, ingest it as an episode. This gives the memory system
initial content to work with:

```
am_ingest(readme_content, "project-readme")
am_ingest(architecture_doc, "architecture-decisions")
```

This is the equivalent of a new team member reading the project docs before starting
work.

**Progressive disclosure:**
In early sessions, the agent is a capable tool. Over time, as memory accumulates,
it becomes a knowledgeable collaborator. Don't try to simulate knowledge you don't
have -- let it emerge.

### 6.3 The Transition Curve

```
Session 1-3:    Agent works well, memory builds silently
Session 4-8:    First useful recalls appear ("Oh, it remembered!")
Session 10+:    Memory becomes reliably useful
Session 20+:    Agent has genuine project expertise
Session 50+:    Cross-project insights emerge (novel connections)
```

The key insight: **don't front-load the value proposition.** Memory is a compounding
asset. Its value grows superlinearly with use.

### 6.4 What to Capture in Early Sessions

In the first few sessions, be slightly more aggressive about storage to bootstrap
the memory system:

- Buffer all problem-solving exchanges (not just exceptional ones)
- Mark project architecture as salient on first encounter
- Mark user preferences as salient when first expressed
- Ingest any existing project documentation

After 5-10 sessions, relax to the normal admission filter described in section 4.

---

## 7. Coding Agent Specifics

### 7.1 What's Worth Remembering for a Coding Agent

Synthesis from Ma (2025), Zaninotto (2026), Furmanets (2026), and practitioner reports:

**Highest value (always store):**

1. **Architecture decisions with rationale** -- "We use event sourcing because the audit
   trail is a business requirement." These are the hardest to reconstruct and the most
   damaging when forgotten.

2. **Project conventions and patterns** -- "All DB access goes through repository traits."
   "Error types implement `From` for automatic conversion." These prevent the agent from
   introducing inconsistent patterns.

3. **User's working style and preferences** -- "Prefers functional style." "Wants three
   similar lines over premature abstraction." "Runs `just check` before committing."
   These are the difference between a generic agent and a personalized collaborator.

4. **Known gotchas and footguns** -- "The auth middleware order matters: CORS before auth
   before routes." "Tests hang if the mock DB isn't cleaned up." These prevent
   re-discovering the same problems.

**Medium value (store when significant):**

5. **Bug patterns and their solutions** -- "When the connection pool exhausts, it's
   usually because concurrent query count exceeds pool size." Patterns generalize across
   sessions.

6. **Code review feedback themes** -- "Reviewer consistently asks for error handling on
   external API calls." These encode team standards that aren't in the style guide.

7. **Dependencies and their quirks** -- "SQLite in WAL mode needs `busy_timeout` set."
   "The `serde` derive macro requires the feature flag." Technical knowledge that's
   hard to look up.

8. **Project-specific vocabulary** -- "In this codebase, 'occurrence' means a word
   instance on the manifold, not a general event." Domain terminology prevents
   misunderstanding.

**Low value (usually skip):**

9. Specific line numbers or file paths (these change constantly)
10. Exact error messages (the *pattern* matters, not the specific trace)
11. Mechanical operations (formatting, running tests, git operations)
12. Temporary workarounds that will be removed

### 7.2 The Issue Tracker as External Memory

Ma (2025) identifies a powerful pattern: use the repository's issue tracker as an
organized external memory system. When the agent discovers something worth remembering
but not worth persisting in the memory system, create a GitHub issue:

> "When you have plans you don't want to act on immediately, ask the agent to post
> them as GitHub issues using the `gh` CLI. This prevents losing track of ideas and
> creates a backlog you can return to."

This complements geometric memory: the issue tracker stores actionable items, while
DAE stores experiential knowledge and patterns.

### 7.3 The `/remember` Pattern

Ma (2025) uses a slash command that writes to AGENTS.md:

```
/remember -- "Remember what you just learned by writing it into AGENTS.md"
```

For DAE, the equivalent is `am_salient`:

```
am_salient("In this project, repository traits wrap all DB access.
Never call SQLite directly from business logic.")
```

The difference: AGENTS.md is a flat file that grows without bound and requires manual
curation. DAE's geometric memory has natural activation/decay dynamics -- useful
memories get reinforced, stale ones drift.

### 7.4 Architecture Decision Records (ADRs) as Memory Seeds

Zaninotto (2026) recommends ADRs committed alongside code. For DAE integration:

When an architectural decision is made during a coding session:
1. The decision gets captured in an ADR file (standard practice)
2. The decision also gets marked salient: `am_salient("ADR-007: We use CQRS for the
   order service because read/write patterns diverge significantly")`
3. Future sessions can recall this without reading the ADR file

This creates redundancy (good) -- the decision exists in both structured documentation
and associative memory. The ADR is the source of truth; the memory is the fast-access
path.

### 7.5 Code SEO for Memory-Augmented Agents

Zaninotto's "Code SEO" concept applies to memory queries too. When code uses
descriptive names, domain vocabulary, and cross-references, the geometric memory
system can match queries to recalled episodes more effectively because the same
vocabulary appears in both the stored context and the query.

---

## 8. Proposed Prompt Architecture for DAE

### 8.1 System Prompt Integration

The following is a proposed CLAUDE.md section for instructing Claude Code to use
DAE memory tools. The design principles:

- Memory operations should feel natural, not ceremonial
- The agent should think about *what* to remember, not *how* the system works
- Failures should be silent (memory is enhancement, not dependency)

```markdown
## Memory (DAE Geometric Memory via MCP)

You have access to a geometric memory system that persists knowledge across sessions.
Use it naturally -- like your own long-term memory, not like a database.

### Session Start
On your first response in any session, query memory with the user's message:
  am_query(user_message_text)

If recall returns relevant context, weave it into your response naturally.
Don't announce "I searched my memory" -- just use what you know.

### During the Session
- After substantive problem-solving exchanges, buffer them:
  am_buffer(user_text, your_response)
- After generating significant responses, strengthen connections:
  am_activate_response(your_response_text)
- When the topic shifts significantly, query again:
  am_query(new_topic_text)

### What to Remember (am_salient)
Mark insights that will matter in future sessions:
- Architecture decisions with rationale
- Project conventions and patterns
- User preferences and working style
- Gotchas, footguns, and non-obvious constraints
- Cross-project patterns and insights

Do NOT mark salient:
- Temporary state or work-in-progress
- Specific line numbers or file paths
- Mechanical operations
- Error traces (store the lesson, not the trace)

### What to Buffer (am_buffer)
Buffer exchanges that contain:
- Problem-solving dialogue
- Decisions with reasoning
- User-expressed preferences
- Teaching moments

Skip buffering for:
- Routine confirmations
- Mechanical operations
- Boilerplate generation

### Integration Style
- Use recalled context silently for preferences and conventions
- Lightly signal when building on previous discussions: "Since this project uses..."
- Be explicit when continuing interrupted work: "Last session we were working on..."
- Be explicit when surfacing cross-session connections: "This reminds me of..."
- Always prioritize current session information over recalled context
- Surface conflicts: "Previously we decided X, but the code now shows Y..."
```

### 8.2 The Memory Loop in Practice

A concrete session walkthrough:

```
User: "Let's work on the query engine. I want to add fuzzy matching."

Agent thinks:
  1. Query memory: am_query("query engine fuzzy matching")
  2. Recall returns: "Project uses geodesic distance for similarity.
     Previous discussion about SLERP threshold 0.9995 for near-parallel cases."
  3. Response incorporates this context naturally.
  4. Buffer the exchange: am_buffer(user_msg, response)

User: "Actually, let's use Levenshtein distance as a fallback."

Agent thinks:
  1. This is a decision with rationale -- buffer it.
  2. This is an architectural decision -- mark salient.
  3. am_salient("Query engine: use Levenshtein distance as fallback
     when geodesic similarity is below threshold")
  4. am_buffer(user_msg, response)
  5. am_activate_response(response) -- strengthen the connections

User: "Run the tests."

Agent thinks:
  1. Mechanical operation -- no buffer needed.
  2. No query needed.
  3. Just run the tests.
```

### 8.3 Tool Usage Frequency Guidelines

| Tool | Frequency | Trigger |
|------|-----------|---------|
| `am_query` | 1-3x per session | Session start, topic shifts, "do you remember" |
| `am_buffer` | Every 2-5 exchanges | After substantive exchanges, skip trivial ones |
| `am_activate_response` | After significant responses | Problem-solving, architecture, explanations |
| `am_salient` | 0-3x per session | Durable insights, decisions, patterns |
| `am_ingest` | Rarely | New project docs, imported context |
| `am_stats` | On request | Diagnostics, debugging memory issues |

---

## 9. Anti-Patterns

### 9.1 The Eager Rememberer

Storing everything. Every exchange buffered, every response activated, every minor
observation marked salient. Result: memory becomes noisy, retrieval quality degrades,
and the agent surfaces irrelevant context.

Fix: Apply the three-question filter (durable? reusable? safe?) strictly.

### 9.2 The Announcing Archivist

"I have searched my memory and found 3 relevant results. Let me integrate these
findings into my response." This breaks conversational flow and draws attention to
the mechanism rather than the value.

Fix: Use memory silently or with light signals. The user should experience a
knowledgeable collaborator, not a search engine with a personality.

### 9.3 The Context Hoarder

Dumping all recalled context into the response regardless of relevance. The agent
retrieves 10 memories and tries to work all of them into the conversation.

Fix: Recall is a menu, not a mandate. Use only what's relevant to the current
exchange. Let subconscious and novel layers inform rather than dominate.

### 9.4 The Stale Fact Repeater

Confidently asserting recalled information that's no longer accurate. "This project
uses Express.js" when it was migrated to Fastify two weeks ago.

Fix: Always prioritize current evidence (code, files, user statements) over recalled
context. When there's a conflict, surface it and update memory.

### 9.5 The Cold Start Apologizer

"I don't have any memories from previous sessions yet, so I'll be starting fresh."
This draws attention to absence and sets a negative frame.

Fix: Just work normally. Memory is invisible when empty and valuable when populated.
Never apologize for not having memories.

### 9.6 The Over-Personalizer

Recalling personal details, emotional states, or social observations. "I noticed
you were frustrated in our last session." This crosses the professional/personal
boundary.

Fix: For coding agents, remember professional context aggressively, personal context
not at all.

---

## 10. Sources

### Academic Papers

- Packer, C., Wooders, S., Lin, K., Fang, V., Patil, S.G., Stoica, I., & Gonzalez,
  J.E. (2023). "MemGPT: Towards LLMs as Operating Systems." arXiv:2310.08560.
  https://arxiv.org/abs/2310.08560

- Pink, M., Wu, Q., Vo, V.A., Turek, J., Mu, J., Huth, A., & Toneva, M. (2025).
  "Position: Episodic Memory is the Missing Piece for Long-Term LLM Agents."
  arXiv:2502.06975. https://arxiv.org/abs/2502.06975

- Rajesh, S., Holur, P., Duan, C., Chong, D., & Roychowdhury, V. (2025).
  "Beyond Fact Retrieval: Episodic Memory for RAG with Generative Semantic
  Workspaces." arXiv:2511.07587. https://arxiv.org/html/2511.07587v1

- Patel, D. & Patel, S. (2025). "ENGRAM: Effective, Lightweight Memory
  Orchestration for Conversational Agents." arXiv:2511.12960.
  https://arxiv.org/abs/2511.12960

- Ahn, K. (2025). "HEMA: A Hippocampus-Inspired Extended Memory Architecture for
  Long-Context AI Conversations." arXiv:2504.16754.
  https://arxiv.org/abs/2504.16754

- Sridhar, K., Dutta, S., Jayaraman, D., & Lee, I. (2024). "REGENT: A
  Retrieval-Augmented Generalist Agent That Can Act In-Context in New Environments."
  arXiv:2412.04759. https://arxiv.org/abs/2412.04759

- Honda & Fujita. (2026). "Human-Like Remembering and Forgetting in LLM Agents: An
  ACT-R-Inspired Memory Architecture." ACM.
  https://dl.acm.org/doi/10.1145/3765766.3765803

- Wu et al. (2025). "How Memory Management Impacts LLM Agents: An Empirical Study
  of Experience-Following Behavior." arXiv:2505.16067.
  https://arxiv.org/html/2505.16067v2

- MemInsight. (2025). "Autonomous Memory Augmentation for Conversational Tasks."
  EMNLP 2025. https://aclanthology.org/2025.emnlp-main.1683.pdf

- AriGraph. (2025). "Integrating Semantic and Episodic Memories in Knowledge Graphs
  for Reasoning and Planning." IJCAI 2025.
  https://www.ijcai.org/proceedings/2025/0002.pdf

- ReMemR1. (2025). "Look Back to Reason Forward: Revisitable Memory for Long-Context
  LLM Agents." arXiv:2509.23040. https://arxiv.org/html/2509.23040v1

- "Towards Large Language Models with Human-Like Episodic Memory." (2025). Trends
  in Cognitive Sciences. https://doi.org/10.1016/j.tics.2025.06.016

- Zep Team. (2025). "Zep: A Temporal Knowledge Graph Architecture for Agent Memory."
  arXiv:2501.13956. https://arxiv.org/abs/2501.13956

### Production Systems and Documentation

- Letta (formerly MemGPT) documentation. "Understanding Memory Management."
  https://docs.letta.com/advanced/memory-management/

- Letta. "MemGPT Agents -- Agent Memory & Architecture."
  https://docs.letta.com/guides/agents/architectures/memgpt

- Letta. "Rearchitecting Letta's Agent Loop: Lessons from ReAct, MemGPT, & Claude
  Code." October 2025. https://www.letta.com/blog/letta-v1-agent

- Mem0. "The Memory Layer for AI Apps." https://mem0.ai/

- Mem0. "Graph Memory for AI Agents." January 2026.
  https://mem0.ai/blog/graph-memory-solutions-ai-agents

- Zep. "Memory Documentation." https://help.getzep.com/v2/memory

- Claude-Mem. "Persistent Memory for Claude Code Across Sessions." February 2026.
  https://yuv.ai/blog/claude-mem

### Practitioner Articles and Guides

- Ma, E.J. (2025). "How to Use Coding Agents Effectively."
  https://ericmjl.github.io/blog/2025/10/14/how-to-use-coding-agents-effectively/

- Zaninotto, F. (2026). "Agent Experience: Best Practices for Coding Agent
  Productivity." Marmelab. https://marmelab.com/blog/2026/01/21/agent-experience.html

- Furmanets, A. (2026). "AI Agents in 2026: Practical Architecture for Tools, Memory,
  Evals, and Guardrails."
  https://andriifurmanets.com/blogs/ai-agents-2026-practical-architecture-tools-memory-evals-guardrails

- Velorum. (2026). "Agent Memory Done Right: Store Less, Ship More." Medium.
  https://medium.com/@1nick1patel1/agent-memory-done-right-store-less-ship-more-a6ab86d52be3

- Sundar, N. (2026). "How to Build AI Agents That Remember User Preferences (Without
  Breaking Context)." freeCodeCamp.
  https://www.freecodecamp.org/news/how-to-build-ai-agents-that-remember-user-preferences-without-breaking-context/

- Monigatti, L. (2025). "The Evolution from RAG to Agentic RAG to Agent Memory."
  https://www.leoniemonigatti.com/blog/from-rag-to-agent-memory.html

- Bessi, L. (2025). "Deep dive into 'Memory for LLMs' architectures." Machine
  Learning at Scale.
  https://machinelearningatscale.substack.com/p/deep-dive-into-memory-for-llms-architectures

- Ahmed, S. (2025). "Engineering Memory for AI Agents: A Practical Guide." Medium.
  https://medium.com/@sahin.samia/engineering-memory-for-ai-agents-a-practical-guide-115a8966e673

- Deepset Team. (2026). "Context Engineering: The Next Frontier Beyond Prompt
  Engineering." https://www.deepset.ai/blog/context-engineering-the-next-frontier-beyond-prompt-engineering

- Pant, R. (2025). "How Claude's Memory Actually Works (And Why CLAUDE.md Matters)."
  https://rajiv.com/blog/2025/12/12/how-claude-memory-actually-works-and-why-claude-md-matters/

- Xu, Y. (2025). "6 Open-Source AI Memory Tools to Give Your Agents Long-Term Memory."
  Medium.
  https://medium.com/@jununhsu/6-open-source-ai-memory-tools-to-give-your-agents-long-term-memory-39992e6a3dc6

- Manus Team. (2025). "Context Engineering for AI Agents: Lessons from Building
  Manus." https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus

- OpenAI. (2025). "Context Engineering - Short-Term Memory Management with Sessions
  (Agents SDK)."
  https://developers.openai.com/cookbook/examples/agents_sdk/session_memory

- Keywood, M. (2025). "Memory Management in LLM Agents: How I Stopped My Agents
  from Becoming Goldfish." Medium.
  https://medium.com/@martinkeywood/memory-management-in-llm-agents-how-i-stopped-my-agents-from-becoming-goldfish-00afc2c5d420

- Ruan, J.T. (2025). "Context Engineering in LLM-Based Agents." Medium.
  https://jtanruan.medium.com/context-engineering-in-llm-based-agents-d670d6b439bc

- Galileo AI. (2025). "Deep Dive into Context Engineering for Agents."
  https://galileo.ai/blog/context-engineering-for-agents
