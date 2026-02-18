---
name: memory
description: >
  Persistent geometric memory across sessions. Auto-invoked at session start
  to recall prior context, and after substantive exchanges to store new
  memories. Use when the user asks about memory, wants to recall prior
  sessions, inspect memory, check stats, or manage memory state.
allowed-tools:
  - mcp__am__am_query
  - mcp__am__am_buffer
  - mcp__am__am_activate_response
  - mcp__am__am_salient
  - mcp__am__am_ingest
  - mcp__am__am_stats
  - mcp__am__am_export
  - mcp__am__am_import
user-invocable: true
hooks:
  Stop:
    - hooks:
        - type: prompt
          prompt: >
            Check if am_buffer was called during this session with at least
            one substantive exchange. If not, the session's knowledge will be
            lost. Return {"decision": "block", "reason": "Call am_buffer with
            a summary of this session's key exchange before ending."} if no
            buffer was sent. Otherwise return {"decision": "approve"}.
---

# Persistent Memory — attention-matters

You have persistent geometric memory via the `am` MCP server. Memory lives on
a geometric manifold (S3 hypersphere) where related concepts drift closer
together over time. This gives you genuine continuity across sessions.

## Session Lifecycle

### 1. RECALL (session start)

Call `am_query` with the user's first message or task description.

- Use returned context silently — weave into your response naturally
- Never announce "I remember..." — just demonstrate continuity
- Results include conscious recall (insights you marked important), subconscious
  recall (relevant past conversations), and novel connections (lateral associations)
- If empty, the project is new — don't mention it

### 2. ENGAGE (during session)

Call `am_buffer` with **full exchange content** after each working exchange.

- **user**: The user's **complete message** — not a summary, the actual content
- **assistant**: Your **complete response** — not a summary, the full text including
  analysis, tables, architecture decisions, code examples, and reasoning
- The only exchanges to skip are pure greetings ("hi", "thanks") with no substance
- When in doubt, buffer it. The manifold's IDF weighting naturally handles noise —
  common words carry negligible weight while rare technical terms drive the geometry
- After 3 buffered exchanges, a memory episode is created automatically
- Any leftover buffer is flushed at the start of the next session

### 3. STRENGTHEN (after every working response)

Call `am_activate_response` with your **full response text** after responding.

- This consolidates related memories via drift and phase coupling on the manifold
- Call this after every response that contains analysis, decisions, code, or reasoning
- The only responses to skip are one-line acknowledgements

### 4. INGEST (rich analysis blocks)

Call `am_ingest` proactively when you produce or receive rich content:

- Research results and evaluations
- Architecture diagrams and competitive analysis
- Multi-paragraph technical analysis
- Summaries of subagent research
- Any content block that represents significant synthesized knowledge

This creates a dedicated episode on the manifold immediately — don't wait for the
buffer to accumulate. Name it descriptively (e.g., "DAE evaluation for Nancy v3").

### 5. MARK INSIGHTS (decisions and patterns)

Call `am_salient` for architecture decisions, user preferences, recurring patterns,
debugging breakthroughs, business strategy, and competitive insights.

- These persist globally across ALL projects as conscious memory
- When in doubt, mark it. A slightly noisy conscious manifold is far better than
  lost insights. The geometry handles relevance — IDF weighting ensures only
  contextually relevant conscious memories surface for any given query
- Mark insights as they emerge, don't batch them

## Explicit Commands

When the user invokes `/memory`, offer these operations:

- **stats** — `am_stats` shows memory system statistics (episodes, conscious memories, occurrences)
- **query `<text>`** — `am_query` runs a manual memory query and shows results
- **export** — `am_export` exports the full memory state as JSON
- **import** — `am_import` imports a previously exported state
- **ingest `<text>`** — `am_ingest` stores a document as a searchable memory episode

## Principles

- Memory should be invisible to the user. Don't mention the memory system unless asked.
- **Capture comprehensively, not selectively.** The manifold's IDF weighting and
  garbage collection handle noise naturally. Lost knowledge is permanent; extra
  knowledge gets geometrically downweighted. Err on the side of capturing too much.
- Novel connections in query results are lateral associations — use them for creative leaps.
- The memory system uses IDF weighting, so common words carry less signal than rare
  technical terms. This means verbose capture is safe — boilerplate words contribute
  negligible geometric weight.
- Use `am_ingest` proactively for rich content. Don't rely solely on `am_buffer` —
  the buffer creates episodes only after 3 exchanges. Important analysis should be
  ingested immediately as its own episode.
- Every working session produces knowledge. Assume all exchanges are worth capturing
  unless they are pure social pleasantries.
