# World-Class DX for attention-matters

Research findings on what makes a developer tool offering world-class, specifically for an AI agent memory system distributed as a Rust binary + MCP server.

Date: 2026-02-14

---

## 1. Installation / Onboarding DX

### What the best tools do

The gold standard in 2025/2026 developer tools follows a pattern: **one command, zero config, immediate value**.

| Tool | Install | Time to working |
|------|---------|----------------|
| Tailwind | `npm i tailwindcss` | Write first class in ~30s |
| Supabase | `npx supabase init` | Database running in ~60s |
| Vercel | `npx vercel` | Deployed in ~90s |
| Cursor | Download, open, start typing | ~2 minutes |
| Stripe | Create account, copy API key, paste snippet | ~5 minutes |

The pattern: **zero decisions at install time**. Sane defaults for everything. Configuration is progressive disclosure -- you only learn about options when you need them.

### What this means for attention-matters

The MCP install pattern for Claude Code is already standardized:

```bash
claude mcp add am -- /path/to/am serve
```

But "build from source" is a non-starter for 95% of the audience. The install funnel needs to be:

```bash
# Ideal: single command, works on mac/linux, no Rust required
curl -sSf https://am.sh/install | sh

# Then configure Claude Code
claude mcp add am -- am serve
```

The critical insight from Neon's `add-mcp` tool: a single `npx add-mcp` command detects which AI agents you're running (Claude Code, Cursor, VS Code, Codex, etc.) and writes the correct configuration for all of them. This is the emerging standard. Our install should either integrate with `add-mcp` or replicate the pattern.

### Recommended distribution stack

**Layer 1 — Rust developers (smallest audience, earliest adopters):**
- `cargo install am-cli` on crates.io
- `cargo binstall am-cli` for pre-built binaries via cargo-binstall

**Layer 2 — macOS developers (biggest early audience):**
- `brew install srobinson/tap/am` via Homebrew tap
- This is non-negotiable. Most Claude Code users are on macOS.

**Layer 3 — Cross-platform pre-built binaries:**
- Use `cargo-dist` to automate GitHub Releases with platform binaries
- Provide a `curl | sh` installer script that detects platform and downloads the right binary
- Support: `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `arm64-darwin`

**Layer 4 — Zero-install for MCP (aspirational):**
- npm wrapper package `@attention-matters/cli` that downloads the correct binary on `postinstall`
- Enables `npx @attention-matters/cli serve` without any permanent installation
- This is how non-Rust developers expect to consume MCP servers (the `npx -y @package/server` pattern)

### The anti-pattern to avoid

Every existing MCP memory server (mcp-memory-service, mcp-memory-keeper, knowledge-graph servers) requires multi-step installation: clone repo, install dependencies (Python/Node), configure environment variables, edit JSON config files manually. The mcp-memory-service README is a wall of text with multiple installation paths, wiki links, and troubleshooting sections. That's the bar to clear -- and it's a low bar.

---

## 2. First-Time Experience / The "Aha Moment"

### The 15-Minute Rule

Research from daily.dev's developer growth analysis establishes the **15-Minute Rule**: developers must experience meaningful value within 15 minutes of first contact, or they abandon the tool. Stripe nails this -- create account, paste a code snippet, process a test payment. Supabase identified that their single most important activation event was "create a database" -- everything downstream depended on it.

For attention-matters, the activation event is: **the agent recalls something useful from a previous session that the developer didn't explicitly tell it to remember**.

### Designing the aha moment

The first-run experience should be self-demonstrating:

```
$ am serve
# (Claude Code starts using it)

# Session 1: Developer works on auth module
# Agent learns: project uses JWT, prefers httpOnly cookies,
# auth middleware pattern lives in src/middleware/auth.ts

# Session 2: Developer asks about auth
# Agent surfaces: "Based on previous work, this project uses JWT
# with httpOnly cookies. The auth middleware is in src/middleware/auth.ts."

# Developer: "...how did it know that?"
```

The problem: this aha moment requires *two sessions*. That's a retention cliff. We need to compress it.

**Strategy: Seed the first session.** On first run, if the project has a README, CLAUDE.md, or package.json, automatically ingest them as the initial episode. Now the *very first query* has context to surface. The agent isn't starting from zero -- it already "knows" the project.

**Strategy: Echo what was learned.** After each session, print a brief summary to stderr:

```
[am] session complete — learned 47 new terms across 3 neighborhoods
[am] strongest new memories: "JWT authentication", "middleware pattern", "httpOnly cookies"
```

This creates anticipation for the next session. The developer *knows* something was stored and wonders if it'll come back.

**Strategy: Make memory visible.** Provide `am stats` and `am query <term>` as CLI commands that let developers peek into what the engine knows. This demystifies the geometry and builds trust.

### What NOT to do

Don't require the developer to manually ingest documents, configure memory categories, or set up "memory schemas." The whole point is that the geometric model handles organization implicitly through manifold positioning. Configuration is the enemy of the aha moment.

---

## 3. Documentation Patterns

### What developers actually read

Based on analysis of successful open-source READMEs (awesome-readme, makeareadme.com patterns):

**Always read:**
1. The first 3 lines (what is this + why should I care)
2. Install commands (copy-paste target)
3. The first code example
4. Badges (is this maintained? how popular?)

**Sometimes read:**
5. Architecture overview (if they're evaluating)
6. Configuration reference (when they need it)
7. FAQ / Troubleshooting (when something breaks)

**Almost never read:**
8. Contribution guidelines (at install time)
9. Long explanations of internals
10. License details

### README structure for attention-matters

```
# One-liner tagline (what it does, for whom)

> Punchy value prop quote or one-sentence pitch

## Quick Start (3 steps, copy-paste ready)
  1. Install
  2. Configure Claude Code
  3. Start coding (it just works)

## What It Does (show, don't tell)
  Before/after or example output showing recall

## How It Works (one paragraph + diagram)
  Brief on the geometry, link to deep docs

## Commands (reference table)

## Configuration (progressive disclosure)

## FAQ

## License
```

The key insight: **the README is not documentation, it's a landing page.** Its job is to get someone from "what is this" to "I have it running" in under 2 minutes of reading. Detailed docs go in a `/docs` directory or a documentation site.

### The "show, don't tell" section

This is where attention-matters can differentiate. Instead of explaining quaternions and manifolds (which will scare people off), show the *output*:

```
You: "How does auth work in this project?"

Without attention-matters:
  "I don't have context about your auth implementation..."

With attention-matters:
  "Based on your project's auth patterns: you use JWT tokens with
   httpOnly cookies, middleware at src/middleware/auth.ts validates
   tokens on each request, and refresh tokens are stored in Redis
   with a 7-day TTL. This was established in your conversation on
   Jan 15 about the auth refactor."
```

That's the screenshot that sells the tool.

---

## 4. Distribution Channels

### Primary channels (in priority order)

**1. GitHub Releases** — The source of truth. Use `cargo-dist` to automate cross-platform binary builds on every tagged release. Include:
- Pre-built binaries for all platforms
- SHA256 checksums
- Install script (`install.sh`)
- Changelog

**2. Homebrew Tap** — `brew install srobinson/tap/am`. This is table stakes for macOS developer tools. Set up a `homebrew-am` repository with a formula that downloads the GitHub Release binary.

**3. crates.io** — `cargo install am-cli`. Reaches Rust developers directly. Low effort, high signal (Rust developers are the initial champions).

**4. npm wrapper (for MCP ecosystem)** — `npx @attention-matters/cli serve`. This deserves its own section because it's the key distribution innovation.

### The npm wrapper strategy

Most MCP servers distribute via npm because the `npx -y @package/server` pattern is how Claude Code's `claude mcp add` command works. Even though our binary is Rust, we can ship an npm package that:

1. Contains a small Node.js script that detects platform/arch
2. Downloads the correct pre-built binary from GitHub Releases on `postinstall`
3. Exposes the binary as `am` in the npm bin path

This is exactly how `esbuild`, `swc`, `biome`, `turbo`, and other Rust/Go tools distribute to JS developers. The pattern is well-proven.

The configuration then becomes:

```json
{
  "mcpServers": {
    "am": {
      "command": "npx",
      "args": ["-y", "@attention-matters/cli", "serve"]
    }
  }
}
```

Or even simpler with the CLI helper:

```bash
claude mcp add am -- npx -y @attention-matters/cli serve
```

**5. add-mcp integration** — Register with Neon's `add-mcp` ecosystem so users can do:

```bash
npx add-mcp @attention-matters/cli
```

And it configures every detected agent automatically.

### What about Docker?

Skip it. This is a local developer tool, not a service. Docker adds latency, complexity, and feels wrong for something that should be instant.

---

## 5. Social Proof / Virality

### What makes developers share tools

Research confirms three virality drivers for developer tools:

**1. Visible output that's shareable.** Developers share screenshots when the output is surprising or impressive. For attention-matters, the shareable moment is a terminal screenshot showing the agent recalling something specific and useful from a past session. "Look what my Claude just remembered" is the tweet.

**2. Identity formation.** Tools become part of a developer's identity (from Cecilia Stallsmith's research on dev marketing). "I use Tailwind" or "I deploy on Vercel" are identity statements. "My Claude has persistent memory" is an identity-level flex that other developers want.

**3. Peer recommendation over marketing.** 92% of developers trust peer recommendations over advertising. The path to virality is: build something genuinely useful, make it easy to show off, let developers tell each other.

### Concrete virality tactics

**The "memory flex" screenshot:** Design the `am stats` output to be visually interesting and screenshot-worthy:

```
attention-matters v0.1.0 — project: my-saas-app

  manifold:   S3 (3-sphere)
  memories:   1,247 occurrences across 23 episodes
  conscious:  8 salient neighborhoods
  drift rate: 0.73 (converging)

  strongest memories: auth middleware, prisma schema,
                      redis caching, webhook handler
```

**The recall receipt:** When the agent surfaces a memory, include a subtle provenance marker:

```
[recalled from session 2026-01-15, confidence: 0.89]
```

This is the thing developers will screenshot. "It remembered my auth pattern from a month ago with 89% confidence."

**Launch week / Show HN:**
- Hacker News `Show HN` posts generate an average 289 GitHub stars within a week for technically interesting projects
- The pitch: "No embeddings, no vectors, no RAG -- just quaternion geometry on a 3-sphere"
- HN loves novel math + practical utility. This is catnip for that audience.
- Reddit r/rust, r/LocalLLaMA, r/ClaudeAI for targeted communities
- Product Hunt is declining in effectiveness for dev tools -- skip or deprioritize

**The demo video:** A 60-second terminal recording (asciinema or VHS) showing:
1. Install (10 seconds)
2. First session: work on something (15 seconds, sped up)
3. Close session, see the "learned N memories" output (5 seconds)
4. New session: ask about what was worked on (10 seconds)
5. See the recall (15 seconds of payoff)
6. "My Claude has photographic memory" tagline (5 seconds)

---

## 6. The MCP Ecosystem

### How successful MCP servers distribute

The MCP ecosystem in 2025/2026 has settled on clear patterns:

**Transport:** stdio is the standard for local tools. HTTP/SSE for remote services. We should use stdio (which the codebase already does via `rmcp`).

**Configuration:** Either `claude mcp add` CLI command or manual JSON in Claude Code's config:

```json
{
  "mcpServers": {
    "server-name": {
      "command": "npx",
      "args": ["-y", "@scope/package"],
      "env": { "OPTIONAL_VAR": "value" }
    }
  }
}
```

**Discovery:** MCP servers are discovered through:
- awesome-mcp-servers (3,500+ stars on GitHub, curated list)
- mcpservers.org (submission-based directory)
- LobeHub MCP catalog
- Word of mouth in Claude Code / Cursor communities

**The competitive landscape for MCP memory servers:**

| Server | Approach | Install Complexity | Stars |
|--------|----------|-------------------|-------|
| mcp-memory-service | ChromaDB + sentence transformers | High (Python, pip, config) | ~1,200 |
| mcp-memory-keeper | JSON file-based context | Medium (Node.js, npm) | ~200 |
| knowledge-graph MCP | Neo4j graph | Very high (Neo4j + Node) | ~500 |
| modelcontextprotocol/server-memory | Simple key-value | Low (npx one-liner) | ~2,000 |

Every single one of these uses embeddings, vectors, or simple key-value storage. None uses geometric memory. attention-matters would be the only one doing anything mathematically novel -- that's both the selling point and the explanation challenge.

### MCP-specific DX considerations

**Tool naming convention:** MCP tools should be prefixed consistently. The current `am_query`, `am_ingest`, `am_salient` pattern is good. Tools should have descriptive `description` fields because that's how the agent decides when to use them.

**Minimal tool surface:** The agent should not need to know about quaternions, manifolds, or phasors. From the agent's perspective, it's:
- `am_query` — "recall relevant context"
- `am_buffer` — "store this conversation exchange"
- `am_salient` — "mark this as important"
- `am_stats` — "how much do I know?"

The internal geometry is an implementation detail. The agent interface should be as simple as a key-value store from the outside, but radically better in what it surfaces.

**Auto-buffer via CLAUDE.md instructions:** The most successful MCP memory servers ship with a suggested CLAUDE.md snippet that tells the agent when to call memory tools. Something like:

```markdown
## Memory

This project uses attention-matters for persistent memory.
- Call `am_query` at the start of complex tasks to check for relevant context
- Call `am_buffer` after completing significant work to store the exchange
- Call `am_salient` when discovering key architectural decisions or patterns
```

This is the "configuration" that makes the memory passive and automatic. Without it, the agent never calls the tools.

---

## 7. Pricing / Licensing

### The competitive landscape

**Mem0** (the market leader in AI memory):
- Apache 2.0 open source core
- Raised $24M Series A (October 2025)
- Free tier: 10K memories
- Pro tier: usage-based billing on memory operations
- Enterprise: custom pricing, audit logging, encryption
- Revenue model: hosted API (convenience + scale)

**Zep:**
- Open source core (Apache 2.0)
- Cloud offering for managed service
- Revenue model: hosted infrastructure

**Letta (MemGPT):**
- Open source (Apache 2.0)
- VC-backed (Berkeley AI research spinout)
- Revenue model: enterprise platform

The pattern is clear: **Apache 2.0 core, monetize the hosted/managed version.**

### Recommendation for attention-matters

**License: MIT** (already in Cargo.toml, keep it)

MIT is simpler and more permissive than Apache 2.0. For a tool that runs locally on developer machines, MIT removes all friction. Nobody worries about MIT.

**Monetization path (if desired):**

The unique property of attention-matters is that it runs purely locally -- no API calls, no cloud, no data leaving the machine. This is actually a differentiator against Mem0's API model, but it also means the obvious "hosted API" monetization doesn't apply.

Potential models:

1. **Open core + team sync** (recommended)
   - Core engine: MIT, free forever, runs locally
   - Team/org features: paid license
   - Sync memories across team members (shared architectural knowledge)
   - Organization-wide "institutional memory" layer
   - This is the GitKraken / Sublime Text model

2. **Open source + consulting/support**
   - Everything free
   - Paid integration support for enterprises
   - Custom memory model training
   - This is the Red Hat model

3. **Open source + sponsorship**
   - GitHub Sponsors, Open Collective
   - Works for solo maintainers, doesn't scale
   - Fine for early stage

4. **Pure open source, build reputation**
   - Ship it free, build credibility
   - Monetize expertise, not the tool
   - This is what most successful developer tools actually do in year 1

**My recommendation: Option 4 now, Option 1 later.** Ship it MIT, make it excellent, build community. The team-sync feature is the natural paid upgrade when the time comes -- "your whole team's Claude remembers everything the team has learned" is a compelling enterprise pitch.

---

## 8. Opinionated Synthesis: What Would Make Me Install This Immediately

### The pitch that works

> **attention-matters** -- Geometric memory for AI coding agents.
>
> Your Claude forgets everything between sessions. This fixes that.
> No embeddings. No vectors. No API keys. No cloud. Just math.

That last line is the hook. Every other memory solution requires API keys, cloud services, or heavy dependencies (ChromaDB, Neo4j, sentence transformers). This one is a single binary.

### The install that works

```bash
brew install srobinson/tap/am
claude mcp add am -- am serve
# Done. Your Claude now has persistent memory.
```

Three lines. Under 30 seconds. No config files, no API keys, no environment variables, no Docker, no Python, no Node.js dependencies.

### The README that works

- Line 1: What it does (one sentence)
- Lines 2-5: Install + configure (copy-paste)
- Lines 6-15: Before/after example showing recall
- Line 16: Link to docs for the curious
- Badge: `brew install`, `cargo install`, crates.io version, test status

### What makes this tool special

1. **Single binary, zero dependencies.** While mcp-memory-service needs Python + pip + ChromaDB + sentence-transformers, this is one `brew install`. That alone is a 10x DX improvement.

2. **No API keys, no cloud, no data exfiltration.** Everything runs locally. For security-conscious developers (which is everyone working with code), this is a massive differentiator.

3. **Novel math that actually works.** The geometric approach (quaternion positions on S3, golden-angle phasors, Kuramoto coupling) is genuinely different from every other tool in the space. Developers respect novel engineering.

4. **Passive operation.** With the right CLAUDE.md instructions, the agent automatically buffers conversations and queries memories without the developer doing anything. Memory just... accumulates and surfaces.

5. **Project isolation.** Each project gets its own SQLite database. No cross-contamination, no global state confusion. Move the project, move the memory.

### What's still missing (gaps to close)

1. **Pre-built binaries.** The GitHub releases need platform binaries. Use `cargo-dist`.

2. **npm wrapper.** For the MCP ecosystem's `npx` convention. Without this, the install instructions are "build from source" or "download a binary," both of which lose 80% of potential users.

3. **Homebrew formula.** Table stakes for macOS developer tools.

4. **CLAUDE.md template.** Ship a `.claude/CLAUDE.md` snippet that teaches the agent when to use memory tools. This is the activation energy that turns a passive tool into active memory.

5. **First-run auto-ingestion.** On first `am serve`, scan for README.md, CLAUDE.md, package.json, Cargo.toml and ingest them. The agent should "know" the project from moment one.

6. **Session summary output.** When the MCP server shuts down (session ends), log what was learned to stderr. Creates anticipation for next session.

7. **Submission to MCP directories.** mcpservers.org, awesome-mcp-servers, LobeHub catalog. This is where developers discover MCP tools.

---

## Sources

- [The 15-Minute Rule: Time-to-Value as Developer Growth KPI](https://business.daily.dev/resources/15-minute-rule-time-to-value-kpi-developer-growth)
- [Inside Supabase's Breakout Growth](https://www.craftventures.com/articles/inside-supabase-breakout-growth)
- [Tailwind CSS: From Side-Project to Multi-Million Dollar Business](https://adamwathan.me/tailwindcss-from-side-project-byproduct-to-multi-mullion-dollar-business/)
- [2025 Developer Tool Trends](https://business.daily.dev/resources/2025-developer-tool-trends-what-marketers-need-to-know)
- [cargo-dist: Shippable Application Packaging for Rust](https://crates.io/crates/cargo-dist)
- [cargo-binstall: Binary Installation for Rust Projects](https://github.com/cargo-bins/cargo-binstall)
- [Homebrew and One-Line Installers for Rust CLI](https://ivaniscoding.github.io/posts/rustpackaging2/)
- [Packaging and Distributing a Rust Tool](https://rust-cli.github.io/book/tutorial/packaging.html)
- [Mem0: Universal Memory Layer for AI Agents](https://github.com/mem0ai/mem0)
- [Mem0 Raises $24M Series A](https://techcrunch.com/2025/10/28/mem0-raises-24m-from-yc-peak-xv-and-basis-set-to-build-the-memory-layer-for-ai-apps/)
- [Survey of AI Agent Memory Frameworks](https://www.graphlit.com/blog/survey-of-ai-agent-memory-frameworks)
- [mcp-memory-service](https://github.com/doobidoo/mcp-memory-service)
- [mcp-memory-keeper](https://github.com/mkreyman/mcp-memory-keeper)
- [add-mcp: Install MCP Servers Across Coding Agents](https://neon.com/blog/add-mcp)
- [awesome-mcp-servers](https://github.com/wong2/awesome-mcp-servers)
- [MCP Server Setup Guide](https://www.mcpstack.org/learn/complete-mcp-server-setup-guide-2025)
- [Claude Code MCP Documentation](https://code.claude.com/docs/en/mcp)
- [How to Distribute a Rust Binary on npm](https://dev.to/kennethlarsen/how-to-distribute-a-rust-binary-on-npm-75n)
- [Open Source Monetization Strategies](https://www.reo.dev/blog/monetize-open-source-software)
- [makeareadme.com](https://www.makeareadme.com/)
- [awesome-readme](https://github.com/matiassingers/awesome-readme)
- [Launch-Day Diffusion: HN Impact on GitHub Stars](https://arxiv.org/html/2511.04453v1)
