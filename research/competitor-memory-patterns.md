# Competitor Memory Patterns: How AI Agent Memory Products Work

Research date: 2026-02-14

---

## Table of Contents

1. [Mem0 (OpenMemory)](#1-mem0-openmemory)
2. [Zep](#2-zep)
3. [Letta (formerly MemGPT)](#3-letta-formerly-memgpt)
4. [Claude's Built-in Memory](#4-claudes-built-in-memory)
5. [MCP-Based Memory Servers](#5-mcp-based-memory-servers)
6. [The Fundamental Problem (Dan Giannone's Critique)](#6-the-fundamental-problem)
7. [Synthesis: What This Means for DAE](#7-synthesis-what-this-means-for-dae)

---

## 1. Mem0 (OpenMemory)

**What it is:** A memory-as-a-service platform ($24M funded) that provides persistent memory for AI agents via API or MCP server. Open-source core with hosted platform.

### How Storage Works

Mem0 uses a **vector-store + optional knowledge graph** approach:
- Text snippets are extracted from conversations
- Embedded into vectors and stored in a vector DB
- Optional "graph memory" mode builds entity relationships
- Memories are scoped by `userId`, `agentId`, `appId`, and `sessionId`

### MCP Server Integration

Mem0 exposes **9 MCP tools** to any compatible client:

| Tool | Purpose |
|------|---------|
| `add_memory` | Store conversations or facts |
| `search_memories` | Find relevant memories with filters |
| `get_memories` | List memories with pagination |
| `update_memory` | Modify existing memory content |
| `delete_memory` | Remove specific memories |
| `delete_all_memories` | Bulk delete memories |
| `delete_entities` | Remove user/agent/app entities |
| `get_memory` | Retrieve single memory by ID |
| `list_entities` | View stored entities |

### How the Agent Knows When to Store/Recall

**The critical insight: Mem0 delegates entirely to the LLM.** From their docs:

> "The agent automatically decides when to use memory tools based on context."

There is no explicit prompt injection template in the MCP integration. The approach relies on:

1. **Tool descriptions** -- The MCP tool names and descriptions are sufficient for modern LLMs to figure out when to call them
2. **System prompt recommendations** -- They suggest users add instructions like:
   ```
   When creating memories, use:
   - agentId: "my-assistant"
   - appId: "my-project"
   - sessionId: "current-conversation-id"
   ```
3. **"alwaysAllow" config** -- Their MCP config recommends auto-approving memory tools so the agent does not need user permission each time

### What Works Well

- **Zero-infrastructure MCP setup** -- `uvx mem0-mcp-server` and you are running
- **Cross-client memory sharing** -- Same memories accessible from Claude Desktop, Cursor, any MCP client
- **OpenMemory local mode** -- Full local storage with embeddings, no data leaves machine
- **Simple mental model** -- "Remember this" / "What do you know about X?" patterns feel natural

### What Users Complain About

- **Agent over-stores** -- Without guidance, agents store every trivial detail ("user said hello")
- **Retrieval noise** -- Cosine similarity returns tangentially related memories that confuse the LLM
- **No staleness handling** -- Old memories persist with equal weight; no decay, no invalidation
- **The "I remember" problem** -- Agents unnaturally announce "Based on my memories of you..." which feels robotic
- **Token cost** -- Every interaction triggers memory search, burning tokens even when unnecessary

### Awkward Memory Handling

Mem0 does not address this directly. The agent decides how to use retrieved memories, and many users report the pattern of agents saying things like "I recall from our previous conversations that you prefer Python" which feels unnatural. There is no guidance in their docs about graceful integration of recalled context.

---

## 2. Zep

**What it is:** A "context engineering platform" that builds temporal knowledge graphs from conversations. Published a research paper (Jan 2025) showing it outperforms MemGPT on the Deep Memory Retrieval benchmark.

### Architecture: Temporal Knowledge Graph

Zep's core differentiator is that it does NOT use a simple vector store. Instead:

1. **Messages go in** -- You call `thread.add_messages()` with every chat turn
2. **Zep builds a knowledge graph** -- Entities become nodes, relationships become edges
3. **Facts are timestamped** -- Every fact has `valid_at` and `invalid_at` timestamps
4. **Summaries aggregate** -- Entity nodes get high-level summary text
5. **Context blocks come out** -- Pre-formatted text ready to inject into system prompt

### How the Agent is Instructed

**Zep's approach is fundamentally different from Mem0: the agent does NOT manage memory.**

The developer's application code handles all memory operations. The pattern:

```python
# 1. On every chat turn, send messages to Zep
zep_client.thread.add_messages(thread_id, messages=messages)

# 2. Before calling the LLM, retrieve context
user_context = zep_client.thread.get_user_context(thread_id=thread_id)
context_block = user_context.context

# 3. Inject into system prompt
system_prompt = f"""
You are a helpful assistant.

{context_block}
"""
```

The agent never calls memory tools. Memory is handled by infrastructure, not by the LLM.

### Context Block Format

Zep's automatically assembled context block looks like:

```
<<USER_SUMMARY>>
Emily Painter is a user with account ID Emily0e62 who uses digital art
tools for creative work. She maintains an active account with the service.
<</USER_SUMMARY>>

<FACTS>
- Emily is experiencing issues with logging in. (2024-11-14 - present)
- User account Emily0e62 has a suspended status. (2024-11-14 - present)
- The failed transaction used a card ending 1234. (2024-09-15 - present)
- Account made a failed transaction of 99.99. (2024-07-30 - 2024-08-30)
</FACTS>
```

### Fact Extraction Pattern

Zep extracts facts automatically from conversation history using LLM calls in their backend. Key features:

- **Temporal validity** -- Facts have date ranges, can be invalidated
- **Fact rating** -- Developers define what "important" means for their domain:
  ```python
  fact_rating_instruction = """Rate the facts by poignancy. Highly poignant
  facts have a significant emotional impact or relevance to the user."""
  fact_rating_examples = FactRatingExamples(
      high="The user received news of a family member's serious illness.",
      medium="The user completed a challenging marathon.",
      low="The user bought a new brand of toothpaste.",
  )
  ```
- **Min rating filter** -- `client.memory.get(session_id, min_rating=0.7)` to skip low-value facts
- **Custom entity/edge types** -- Pydantic-like classes to define domain-specific graph structure

### What Works Well

- **Temporal awareness** -- Facts expire and get superseded, solving the staleness problem
- **Developer control** -- App code manages memory, not the flaky LLM
- **Low latency** -- P95 < 200ms for context retrieval
- **Knowledge graph structure** -- Entities and relationships, not just text snippets
- **Fact relevance rating** -- Domain-specific importance filtering
- **No awkward memory announcements** -- Context is injected as system prompt, agent just "knows" things naturally

### What Users Complain About

- **Ingestion latency** -- Takes "a few minutes" to process messages into the graph; last few messages may not be in the context block yet
- **Hosted only** -- Zep Cloud is the primary product; self-hosting is limited
- **Complexity** -- Setting up fact rating, custom entities, templates requires significant upfront work
- **Cost** -- Enterprise pricing for the full platform

### Awkward Memory Handling

Zep handles this **extremely well** by design. Because context is injected into the system prompt (not retrieved by the agent), the agent simply "knows" the information the same way it knows its system instructions. There is no "I remember..." moment. The agent just naturally incorporates the knowledge, the same way a human who already knows something does not announce "I recall from my memory that..."

This is the strongest design pattern observed in this research.

---

## 3. Letta (formerly MemGPT)

**What it is:** An open-source framework (21K GitHub stars) built on the MemGPT research paper. The core idea: treat the LLM like an operating system that manages its own virtual memory.

### The MemGPT Architecture

MemGPT introduced the concept of **self-editing memory via tool calling**:

```
Context Window
  |-- System Instructions (read-only)
  |-- Core Memory Blocks (read-write, always visible)
  |     |-- "persona" block (2K chars)
  |     |-- "human" block (2K chars)
  |     |-- [custom blocks...]
  |-- Recent Messages

External Storage
  |-- Recall Memory (searchable conversation history)
  |-- Archival Memory (long-term semantic storage)
```

### How the Agent Manages Memory

**The agent is given explicit tools to edit its own memory.** This is the defining characteristic:

**Core memory editing tools:**
- `memory_insert` -- Insert text into a named memory block
- `memory_replace` -- Search-and-replace within a block
- `memory_rethink` -- Completely rewrite a block

**External memory tools:**
- `conversation_search` -- Search past message history
- `archival_memory_insert` -- Store facts long-term
- `archival_memory_search` -- Query semantic storage

### The System Prompt Pattern

The system prompt tells the agent it IS a memory-managing entity. The key elements:

1. **The agent knows about its memory structure** -- It is told about core memory blocks, their labels, and their contents
2. **The agent is told to actively maintain memory** -- "You should update your memory blocks when you learn new information about the user"
3. **Memory blocks are literally part of the context** -- The agent sees them every turn:
   ```
   [persona]
   My name is Sam, the all-knowing sentient AI.

   [human]
   The human's name is Chad. They like vibe coding.
   ```
4. **The agent decides what goes where** -- Core memory for always-visible facts, archival for long-term but not immediately needed

### Context Window Management

When the context window fills up:
1. Older messages are **compacted** into a recursive summary
2. Full message history moves to **recall storage**
3. Agent can search recall with `conversation_search`
4. This happens transparently -- agent maintains continuity

### What Works Well

- **Elegant OS metaphor** -- Virtual memory for LLMs is a genuinely clever abstraction
- **Agent autonomy** -- The agent decides what matters, not a heuristic
- **Persistent state** -- Everything saved to DB, survives restarts
- **Memory blocks as structured context** -- Better than raw text snippets
- **Academic rigor** -- Backed by a real research paper with benchmarks
- **Custom memory blocks** -- Developers define arbitrary labeled sections (not just persona/human)

### What Users Complain About

- **Requires smart models** -- Smaller/local LLMs struggle with the memory management protocol; they forget to update memory, make incorrect edits, or get confused by the tool calling
- **Verbose internal monologue** -- Early versions had agents constantly narrating their memory operations: "I should save this to core memory..."
- **Slow** -- Every response potentially involves multiple tool calls (edit memory, search archival, etc.) adding latency
- **Over-engineering for simple cases** -- If you just want a chatbot that remembers your name, the full MemGPT stack is massive overhead
- **Memory block size limits** -- 2K characters per block forces aggressive summarization

### Awkward Memory Handling

MemGPT/Letta partially addresses this through their system prompt design. The agent "thinks" about memory in its internal reasoning (which can be hidden from the user), but its visible responses should not mention memory operations. However, in practice:

- Agents sometimes say "Let me update my memory about you" out loud
- The heartbeat/multi-step mechanism (now deprecated in newer versions) caused visible "thinking" delays
- Modern Letta has moved to native model reasoning (hidden) which is much better

---

## 4. Claude's Built-in Memory

### Claude.ai Memory (Consumer/Team/Enterprise)

Launched September 2025 (Teams/Enterprise), expanded to Pro/Max in October 2025.

**How it works:**
- Claude maintains a **memory summary** -- a single document capturing what it knows about you
- Memory is **project-scoped** -- each Claude Project has separate memory
- Claude decides what to remember based on conversation content
- Users can view, edit, and delete memories in Settings
- **Incognito mode** available for conversations that should not affect memory

**Key design decisions:**
- Focused on **professional/work context** -- job details, project specs, team processes
- Memory is a **summary document**, not individual snippets
- Users have **full control** -- can edit the summary directly
- Safety testing included checks for reinforcing harmful patterns and over-accommodation

**Strengths:**
- Extremely simple UX -- just enable in Settings
- Project scoping prevents context bleeding between workstreams
- User can see and edit exactly what Claude remembers

**Weaknesses:**
- Users report inconsistent recall -- sometimes Claude "knows" things, sometimes it does not
- No API access to memory -- purely a consumer/team feature
- Memory summary can get stale or accumulate irrelevant details
- Power users report turning it off because stale context made responses worse

### Claude Code Memory

Claude Code has a completely different, file-based memory system:

**Two types of memory:**

1. **CLAUDE.md files** (human-written instructions):
   - Project-level: `./CLAUDE.md` (shared with team via git)
   - User-level: `~/.claude/CLAUDE.md` (personal, all projects)
   - Local: `./CLAUDE.local.md` (personal, per project, gitignored)
   - Organization: system-level managed policy files
   - Rules directory: `.claude/rules/*.md` with glob-pattern scoping

2. **Auto memory** (Claude writes for itself):
   - Stored in `~/.claude/projects/<project>/memory/`
   - `MEMORY.md` acts as an index (first 200 lines loaded at session start)
   - Topic files (`debugging.md`, `api-conventions.md`) loaded on demand
   - Claude saves: project patterns, debugging insights, architecture notes, user preferences
   - User can say "remember that we use pnpm, not npm" to trigger explicit saves

**How store/recall works:**
- Store: Claude writes to MEMORY.md or topic files during the session using file tools
- Recall: MEMORY.md index is loaded at every session start; topic files read on demand
- User trigger: "remember that..." or "save to memory that..."
- Automatic: Claude decides to save insights as it discovers them

**Key design insight:** Claude Code's memory is just **markdown files on disk**. No vector DB, no embeddings, no semantic search. Just files that get loaded into context. This is deliberately simple and transparent.

**Strengths:**
- Fully transparent -- users can read and edit memory files directly
- Hierarchical scoping -- organization > project > user > local
- Path-specific rules -- rules that only apply when touching certain files
- Git-friendly -- project CLAUDE.md shared with team, local files gitignored
- No infrastructure -- just files

**Weaknesses:**
- 200 line limit on auto-loaded memory
- No semantic search -- topic files require Claude to know they exist
- Memory quality depends on Claude's judgment about what to save
- Can fill up with noise if Claude over-saves

---

## 5. MCP-Based Memory Servers

### Overview of the Ecosystem

Dozens of MCP memory servers exist on GitHub. The major ones:

### mcp-memory-service (doobidoo) -- 1,307 stars

The most popular general-purpose MCP memory server.

**Architecture:**
- SQLite-vec for local vector storage (5ms reads)
- Optional Cloudflare hybrid backend for cloud sync
- ONNX embeddings for local operation
- BM25 + vector hybrid search

**Tools exposed:**
- `store_memory` / `recall_memory` / `search_by_tag`
- `recall_by_timeframe` / `retrieve_with_quality_boost`
- `memory_health` / knowledge graph tools

**Unique features:**
- **Memory type ontology** -- 5 base types (observation, decision, learning, error, pattern) with 21 subtypes
- **Knowledge graph** -- Typed relationships between memories (causes, fixes, supports, follows)
- **Quality scoring** -- AI-driven automatic quality assessment using ONNX models
- **Session hooks** -- Auto-captures context at session start/end for Claude Code
- **Web dashboard** -- Visual memory management at localhost:8000
- **Emotional metadata** -- emotion, valence, arousal fields on memories
- **Dream-inspired consolidation** -- Decay scoring, compression, archival

**The instruction problem:** This server relies on the LLM discovering and using the tools based on their descriptions. No explicit prompt injection. The session-start hook is the closest thing -- it automatically surfaces relevant memories at the beginning of each session.

### memorious-mcp (cedricvidal) -- 7 stars, but clean design

**Philosophy:** Three operations only: `store`, `recall`, `forget`.

100% local with ChromaDB. Simple and private. The minimalist approach -- no dashboards, no cloud, no ontologies. Just semantic memory.

### Memory-Plus (Yuchen20) -- 51 stars

Lightweight local RAG memory store. Notable for having a **viewer web app** for visualizing memories. Targets the multi-AI-coder use case (share memories across Windsurf, Cursor, Copilot).

### mcp-memory-keeper (mkreyman) -- 90 stars

Focused specifically on **Claude Code context management**. SQLite-backed, simple store/retrieve. The key insight: it is designed to persist the specific context that Claude Code needs between sessions.

### Common Patterns Across MCP Memory Servers

1. **Tool-based recall** -- All expose `store`/`search`/`recall` as MCP tools
2. **No prompt templates** -- None provide system prompt instructions; they rely on tool descriptions
3. **Session hooks** -- The more sophisticated ones (mcp-memory-service) inject context at session start
4. **Local-first** -- Almost all prioritize local storage for privacy
5. **Vector search dominant** -- Most use embeddings + cosine similarity as primary retrieval
6. **Complexity creep** -- mcp-memory-service has grown to 10.11.0 with knowledge graphs, quality scoring, consolidation, dashboards, OAuth -- significant feature bloat

---

## 6. The Fundamental Problem

Dan Giannone's November 2025 article "The Problem with AI Agent Memory" crystallizes the issues that every product above struggles with. He disabled memory in all his AI tools after 2+ years of use. His critique:

### The Fragile Pipeline

Every memory system requires a chain of things to go right:

1. **Importance detection** must correctly flag what matters
2. **Retrieval must trigger** at the right time
3. **Similarity search** must surface relevant (not stale) information
4. **The LLM must correctly integrate** old snippets with new context

If any link breaks, memory does not just fail silently -- it **actively makes things worse**.

### Four Fundamental Problems

1. **Snippets, Not Understanding** -- Vector DBs store text fragments with no structure or relationships. "I have two kids, ages 7 and 9" is a sentence, not a family model. Later mentioning "my older daughter" may or may not connect.

2. **Retrieval Is Bolted On, Not Integrated** -- Memory retrieval happens as an external process, separate from reasoning. The retriever optimizes for embedding similarity; the reasoner optimizes for task completion. No feedback loop.

3. **No Time, Consolidation, or Forgetting** -- Every snippet persists with equal weight forever. No mechanism for staleness, no decay, no reconsolidation. A timestamp band-aid does not fix a broken model.

4. **Failure Is Confidence-Amplifying** -- When memory fails, the agent confidently uses stale/wrong context. "I know you work at Acme Corp" (you changed jobs 3 months ago). The personalization that should increase trust actually delivers the wrong answer.

### The Stale Context Problem (Concrete)

> You told ChatGPT three months ago: "I work at Acme Corp as a product manager."
> Today you have switched jobs but haven't told the AI.
> You ask: "Help me draft an email to my CEO about our Q2 roadmap."
> The agent references Acme Corp context. The response feels subtly wrong.
> You can't tell if the error is the model or injected stale context.

### What a Real Memory System Would Need

Giannone outlines five requirements:

1. **Working state for current tasks** -- Not just context window, but structured representation of active goals, intermediate results, next steps
2. **Episodic history** -- Events in order, with cause/effect understanding, not just "user mentioned X"
3. **Structured knowledge representation** -- Knowledge graph with entities, relationships, confidence levels, recency markers
4. **Policies, habits, and reusable behaviors** -- Learned patterns (always wants Python, prefers concise), extractable and editable
5. **Vectors as supporting index, not primary structure** -- Embeddings for search, graphs and tables for everything else

### His Verdict

> "Under no circumstances should you be trying to engineer your own long-term memory system on top of a vector store. If Anthropic, OpenAI, and the rest haven't made this work reliably yet, your internal IT team is not going to solve it."

---

## 7. Synthesis: What This Means for DAE

### The Landscape at a Glance

| System | Store Trigger | Recall Trigger | Memory Model | Awkward Memory |
|--------|--------------|----------------|--------------|----------------|
| **Mem0** | Agent decides via tool call | Agent decides via tool call | Vector store + optional graph | Not addressed (agent announces) |
| **Zep** | App code sends messages | App code requests context block | Temporal knowledge graph | Solved (system prompt injection) |
| **Letta/MemGPT** | Agent uses memory tools | Agent uses search tools | Hierarchical: core blocks + archival | Partially addressed (hidden reasoning) |
| **Claude.ai** | Claude decides during conversation | Memory summary in system prompt | Summary document | Mostly natural (summary-based) |
| **Claude Code** | Claude writes markdown files | Files loaded at session start | Markdown files on disk | N/A (code assistant context) |
| **MCP servers** | Agent calls store tool | Agent calls recall tool / session hook | Vector store (usually) | Not addressed |

### Key Insights

#### 1. The Two Paradigms

There are fundamentally two approaches:

**Agent-managed memory** (Mem0, Letta, MCP servers):
- The LLM decides when to store and recall
- Uses tool calls to interact with memory
- More flexible but more fragile
- Prone to over-storing, under-recalling, and awkward announcements

**Infrastructure-managed memory** (Zep, Claude.ai summary):
- Application code handles all memory I/O
- Memory is injected into system prompt as context
- More reliable but less flexible
- Agent naturally "knows" things without announcing them

#### 2. The Awkward Memory Problem is a Design Problem

The products that solve this share one pattern: **memory appears in the system prompt, not as tool call results.** When an agent retrieves a memory via tool call, it tends to announce it. When it receives context in the system prompt, it treats it as background knowledge.

Zep solves this perfectly. Claude.ai's summary approach also handles it well. Letta's latest architecture (hidden reasoning) is getting there. Mem0 and most MCP servers do not address it at all.

#### 3. Vector Stores Are Necessary but Not Sufficient

Every serious product is moving beyond pure vector similarity:
- **Zep**: Temporal knowledge graph with entity/relationship structure
- **Letta**: Structured memory blocks + archival search
- **mcp-memory-service**: Added knowledge graph, typed relationships, BM25 hybrid search
- **Claude Code**: Skipped vectors entirely, uses structured markdown files

The trajectory is clear: **structured representation > text snippets**.

#### 4. Temporal Awareness Is the Unsolved Frontier

Only Zep seriously addresses temporal validity with `valid_at`/`invalid_at` timestamps on facts. Everyone else treats memories as eternally valid, which is the root cause of the stale context problem.

#### 5. Simplicity Wins for Developer Experience

Claude Code's approach (just markdown files) is the most loved by developers despite being the least sophisticated. The lesson: **transparency and editability matter more than algorithmic sophistication.** Users want to see what the system "knows" and fix it when it is wrong.

#### 6. The Quality Signal Problem

Everyone struggles with: what is worth remembering? Solutions attempted:
- **Zep**: Custom fact rating instructions with domain-specific examples
- **mcp-memory-service**: AI-driven quality scoring with ONNX models
- **Letta**: Agent decides (with mixed results)
- **Mem0**: Agent decides (with mixed results)
- **Claude Code**: Claude decides + user can say "remember this"

No one has cracked this reliably.

### What DAE Could Do Differently

Based on this research, the opportunities for a geometric memory system:

1. **Activation-based salience instead of flat vector similarity** -- DAE's activation/plasticity model could provide the temporal decay and importance weighting that every competitor lacks

2. **Interference patterns instead of snippet retrieval** -- Rather than retrieving individual memories, DAE's Kuramoto coupling could surface *constellations* of related concepts, solving the "snippets not understanding" problem

3. **Drift as natural forgetting** -- OpenClaw drift provides the consolidation/forgetting mechanism that Giannone identifies as missing from all current systems

4. **Surface computation instead of retrieval** -- DAE's vivid neighborhoods/episodes model is closer to how human memory surfaces relevant context based on current activation, not keyword similarity

5. **The instruction-free approach** -- If DAE memory naturally surfaces as context (like Zep's approach), the agent never needs to "decide" to recall. The relevant context is simply present, weighted by geometric salience. No tools, no prompts, no awkward announcements.

The biggest insight from this research: **the best memory systems are invisible.** They do not require the agent to manage them, and the agent does not announce that it remembered something. DAE's geometric approach could achieve this naturally -- activation levels determine what surfaces, not explicit retrieval decisions.

---

## Sources

- Mem0 MCP Integration docs: https://docs.mem0.ai/platform/features/mcp-integration
- Mem0 OpenMemory MCP: https://mem0.ai/openmemory
- pinkpixel-dev/mem0-mcp: https://github.com/pinkpixel-dev/mem0-mcp
- Zep Architecture Paper: https://arxiv.org/html/2501.13956v1
- Zep Retrieving Context: https://help.getzep.com/retrieving-memory
- Zep Facts and Summaries: https://help.getzep.com/v2/facts
- Zep Adding Memory: https://help.getzep.com/adding-memory
- Letta/MemGPT Architecture: https://docs.letta.com/guides/agents/architectures/memgpt
- Letta Memory Management: https://docs.letta.com/advanced/memory-management/
- Letta Agent Memory Guide: https://docs.letta.com/guides/agents/memory/
- Letta GitHub: https://github.com/letta-ai/letta
- Claude Code Memory docs: https://docs.claude.com/en/docs/claude-code/memory
- Claude.ai Memory announcement: https://www.claude.com/blog/memory
- mcp-memory-service: https://github.com/doobidoo/mcp-memory-service
- memorious-mcp: https://github.com/cedricvidal/memorious-mcp
- Memory-Plus: https://github.com/Yuchen20/Memory-Plus
- mcp-memory-keeper: https://github.com/mkreyman/mcp-memory-keeper
- Dan Giannone, "The Problem with AI Agent Memory": https://medium.com/@DanGiannone/the-problem-with-ai-agent-memory-9d47924e7975
- Mem0 Research Paper: https://www.emergentmind.com/papers/2504.19413
