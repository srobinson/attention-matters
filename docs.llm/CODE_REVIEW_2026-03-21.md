# Code Review: attention-matters

**Date:** 2026-03-21
**Version:** 0.1.18 (commit `44ea679`)
**Scope:** Full workspace - 46 files, 21,065 LOC across `am-core` (9,632), `am-cli` (7,255), `am-store` (4,062)
**Status:** 418 tests passing, clippy clean, zero warnings

## Overall Assessment

Well-engineered codebase. Math foundations are correct and well-tested. Store layer uses parameterized queries throughout (zero SQL injection surface). Clean separation of pure computation from I/O. Property-based tests for quaternion invariants are a standout.

No blockers. All high findings are about future-proofing, not current bugs.

## Findings

### HIGH

**H1: SLERP normalize is load-bearing for chained drift**
`am-core/src/quaternion.rs:229` - The trailing `.normalize()` after SLERP interpolation prevents accumulated floating-point error over chained drift cycles (`query.rs:280`). Currently correct. Consider adding a comment noting it is intentionally load-bearing, not cosmetic. If removed in a refactor, chained drift would silently accumulate error.

**H2: angular_distance conflates S3 and SO(3) semantics**
`am-core/src/quaternion.rs:159-163` - `abs(dot)` collapses antipodal quaternions so `q.angular_distance(-q) == 0`. The doc comment (lines 130-141) correctly explains this is SO(3) rotation distance. For a system positioning memories on S3, antipodal points read as distance zero. Safe because `random_near` never produces antipodal points, but imported data could. Well-documented tradeoff.

**H3: unchecked_transaction throughout Store**
`am-store/src/store.rs` (12 call sites) - All transactions use `unchecked_transaction()`. Safe under the single-mutex design. If multi-client or connection pooling is added, nested BEGIN would fail. No current nesting exists (helper queries like `occurrence_count()` don't open their own transactions).

### MEDIUM

**M1: total_activation() overflow risk**
`am-core/src/neighborhood.rs:112-114` - `Iterator::sum()` for u32 wraps on overflow in release. Extremely unlikely in practice (activation counts are single digits). Individual `Occurrence::activate()` already uses `saturating_add`. Consider `saturating_add` fold for defense in depth.

**M2: Server flush_orphaned_buffer uses unwrap()**
`am-cli/src/server.rs:159` - `episodes.last().unwrap()` after `add_episode`. Logically safe but fragile to refactoring. Consider `expect("just pushed episode")` for clarity.

**M3: GC timestamp parsing via SQL string manipulation**
`am-store/src/store.rs:822-824` - REPLACE-based ISO-8601 to SQLite datetime conversion. Works for internally-generated `Z` timestamps. Would silently produce wrong results for timezone offsets or fractional seconds. Consider a comment noting the assumed format.

**M4: Missing #[must_use] on mutation methods with diagnostic returns**
`system.rs:279`, `neighborhood.rs:145`, `store.rs:479` - Return values carry diagnostic info that callers might ignore. Current callers use them correctly.

### LOW

**L1: is_vivid threshold boundary** - `neighborhood.rs:141` strict `>` is intentional and consistent.
**L2: Box-Muller discards one sample** - `quaternion.rs:383-388` standard simplicity tradeoff.
**L3: SmallRng not crypto-secure** - Correct choice for geometric positioning.

## Positive Observations

1. **Zero SQL injection surface.** Every query uses parameterized `?1` / `params![]`. No string interpolation into SQL.
2. **Property-based testing** covers geometric invariants: unit normalization, SLERP endpoints, triangle inequality, Hamilton product identity.
3. **NaN-safe sorting** via `total_cmp` throughout `compose.rs` with explicit regression test.
4. **EpisodeRef enum** replaces former `usize::MAX` sentinel, eliminating off-by-one bugs.
5. **Input size limits** (1MB cap) applied consistently across all MCP tool endpoints.
6. **Empty-system overwrite guard** (store.rs:107-120) prevents failed loads from destroying data.
7. **saturating_add** on activation counts prevents u32 overflow.
8. **AmStore trait** is a clean hexagonal port with well-documented design decisions.
9. **Signal handling** closes stdin to unblock the stdio loop, ensuring WAL checkpoint on clean shutdown.
10. **418 tests** across unit, integration, property-based, and doc-tests. Key edge cases explicitly covered.

## Summary

| Severity | Count | Theme |
|----------|-------|-------|
| High | 3 | Defensive normalize, antipodal semantics, unchecked transactions |
| Medium | 4 | Overflow, unwrap in server, timestamp parsing, must_use |
| Low | 3 | Threshold edge, Box-Muller waste, SmallRng |
| Positive | 10 | Patterns worth preserving |
