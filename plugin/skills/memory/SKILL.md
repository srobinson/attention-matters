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

Call `am_buffer` with substantive exchange pairs after meaningful technical exchanges.

- **user**: The user's message text
- **assistant**: Your response text
- Skip trivial exchanges (greetings, yes/no, confirmations)
- Include: architecture discussions, debugging sessions, design decisions, code reviews
- After 3 buffered exchanges, a memory episode is created automatically
- Any leftover buffer is flushed at the start of the next session

### 3. STRENGTHEN (after meaningful responses)

Call `am_activate_response` with your response text after giving a substantive
technical answer.

- This consolidates related memories via drift and phase coupling on the manifold
- Not needed for every response — only meaningful technical ones

### 4. MARK INSIGHTS (hard-won knowledge)

Call `am_salient` for architecture decisions, user preferences, recurring patterns,
and debugging breakthroughs.

- These persist globally across ALL projects as conscious memory
- Be selective — only genuinely reusable knowledge

## Explicit Commands

When the user invokes `/memory`, offer these operations:

- **stats** — `am_stats` shows memory system statistics (episodes, conscious memories, occurrences)
- **query `<text>`** — `am_query` runs a manual memory query and shows results
- **export** — `am_export` exports the full memory state as JSON
- **import** — `am_import` imports a previously exported state
- **ingest `<text>`** — `am_ingest` stores a document as a searchable memory episode

## Principles

- Memory should be invisible to the user. Don't mention the memory system unless asked.
- Be selective with `am_salient` — mark genuinely reusable insights, not routine facts.
- Novel connections in query results are lateral associations — use them for creative leaps.
- The memory system uses IDF weighting, so common words carry less signal than rare technical terms.
