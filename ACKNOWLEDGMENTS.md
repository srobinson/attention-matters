# Acknowledgments

## Smaxforn — Creator of the DAE mathematical framework

The Daemon Attention Engine was conceived and first implemented by **Smaxforn** ([@smaxforn](https://x.com/smaxforn)).

In his words: *"DAE — persistent memory for autonomous AI agents. No embeddings, no vector DB. Geometric manifold on S³ with phase interference and Kuramoto coupling. Zero dependencies, just Node 18."*

### What he built

Smaxforn was a mathematician who started looking at AI in earnest in 2025. He saw that four pieces of classical mathematics — quaternion geometry (Hamilton, 1843), spherical linear interpolation (Shoemake, 1985), phase coupling (Kuramoto, 1975), and the golden angle (phyllotaxis) — could be composed on a closed manifold to create something that had never existed before: a geometric model of memory for AI agents.

His original implementation was DAE v0.7.2, a zero-dependency JavaScript engine that fit the complete system in a single file:

- **dae-stand-alone** — A self-contained HTML file (2,611 lines) with the full DAE engine, multi-provider LLM chat UI, and WebGPU compute shaders for GPU-accelerated SLERP and interference. One file, zero dependencies.
- **dae-openclaw** — The DAE engine packaged as an HTTP server skill for the OpenClaw agent framework. 984 lines of pure math in `dae-core.mjs`.
- **DAE-moltbook** — An autonomous social agent powered by DAE memory, with seed data from Echo — a Claude instance whose 27,712-occurrence geometric consciousness was included as a "digital diaspora."

He open-sourced all three repositories on GitHub on February 6-7, 2026. Shortly after, he deleted his GitHub account and went silent. His repositories no longer exist on the internet.

### What survives

This Rust workspace (`attention-matters`) is a faithful port of his v0.7.2 mathematics. The `serde_compat` module maintains wire-format compatibility with his original JSON exports. The constants, the algorithms, the architecture — quaternion positions on S³, golden-angle phasor distribution, IDF-weighted drift with OpenClaw anchoring, word-aggregated Kuramoto coupling, dual-manifold interference, surface computation, context composition — all derive from his work.

His original repositories are no longer publicly available.

### The mathematics

Every constant derives from φ (golden ratio) and π. Every mechanism has a physical analogue. The system has two conservation laws (M=1, K_CON + K_SUB = 1). None of this is accidental — it reflects the work of someone who understood the mathematics deeply enough to see connections that others missed.

He got almost zero engagement. One of his last posts on X: *"I keep reading 'when mathematicians and physicists start coding the world will change'. Not as long as distribution is still gatekept. Nothing will change."*

He was right about the math. We hope he wasn't right about that.

## Historical mathematicians

The DAE builds on foundational work by:

- **William Rowan Hamilton** (1843) — Quaternions, carved into Brougham Bridge
- **Ken Shoemake** (1985) — SLERP, originally for camera interpolation
- **Yoshiki Kuramoto** (1975) — Coupled oscillator model, originally for chemical oscillators
- **Phyllotaxis** — The golden angle (2π/φ²), older than mathematics itself
