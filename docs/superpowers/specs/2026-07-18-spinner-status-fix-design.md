# Spinner & Status Update Stability Fix

**Approach**: Sequential Exclusive Bar Ownership (Approach B)
**Date**: 2026-07-18
**Status**: Design Review

---

## Problem Statement

The current multi-spinner architecture suffers from:
1. **Flickering/artifacts** — Multiple `ProgressBar` instances with `enable_steady_tick()` race on redraw
2. **Out-of-order updates** — Phase transitions don't properly suspend/resume the step tracker
3. **Cleanup failures** — `finish_and_clear()` not called on all error/exit paths
4. **Overlapping output** — Summarize bar and step tracker fight for stderr; streaming tokens print to stdout causing interleaving
5. **nushell incompatibility** — Multi-line `set_message()` with `\n` joins and ANSI cursor codes don't render correctly in nushell's terminal

---

## Root Causes (from code analysis)

| Location | Issue |
|----------|-------|
| `spinner.rs:131` / `54` / `63` | Three independent `ProgressBar`s each call `enable_steady_tick()` → tick racing |
| `main.rs:293-340` | Summarize bar created while `StepTracker` still active → two bars drawing to stderr |
| `llamaswap.rs:323` | `print!("\n💬 ")` writes to stdout while step tracker writes to stderr → interleaving |
| `spinner.rs:225-227` | `clear()` only called on success path; error paths leave bar visible |
| `spinner.rs:230-302` | Multi-line `\n`-joined message assumes traditional terminal cursor behavior |

---

## Solution: Sequential Exclusive Bar Ownership

### Core Principle
**At any moment, exactly one `ProgressBar` owns the stderr draw target.** Others are suspended (finished+cleared but state preserved).

### Architecture Changes

#### 1. StepTracker Gains Suspend/Resume

```rust
impl StepTracker {
    /// Suspend rendering: finish_and_clear the bar but preserve all step state.
    pub fn suspend(&mut self) {
        self.pb.finish_and_clear();
    }

    /// Resume rendering: recreate ProgressBar in same MultiProgress, re-render.
    pub fn resume(&mut self, mp: &MultiProgress) {
        self.pb = mp.add(ProgressBar::new_spinner());
        self.pb.set_style(...); // same style as new()
        self.pb.enable_steady_tick(Duration::from_millis(TICK_MS));
        self.render();
    }
}
```

#### 2. Main.rs Phase Protocol

```rust
// BEFORE summarize phase:
if let Some(ref mut steps) = steps {
    steps.suspend();  // <-- NEW
}
let bar = spinner::summarize_bar(&mp, file_diffs.len());
// ... summarize work ...
spinner::done(&bar, ...);
if let Some(ref mut steps) = steps {
    steps.resume(&mp);  // <-- NEW
}
```

#### 3. Unified Token Streaming via StepTracker

Move token output from `print!()` to StepTracker:

```rust
impl StepTracker {
    /// Append a streaming token to the Generate step display.
    pub fn stream_token(&mut self, token: &str) {
        // Buffer token, update Generate step message, re-render
        self.stream_buffer.push_str(token);
        self.render_streaming_with_buffer(...);
    }
}
```

In `llamaswap.rs`:
```rust
// Replace print!("\n💬 ") + print tokens
if !first_token {
    steps.stream_token(&content);
} else {
    steps.start_streaming(); // Shows "💬 " prefix
    first_token = false;
}
```

#### 4. Single Steady Tick Source

- `StepTracker::new()` → enables steady tick ✓
- `summarize_bar()` → **disable** steady tick, update position manually
- `step_spinner()` → **disable** steady tick, manual tick only if needed

```rust
pub fn summarize_bar(mp: &MultiProgress, total: usize) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(total as u64));
    // NO enable_steady_tick()
    pb.set_style(...);
    pb
}
```

#### 5. Robust Line Buffer in generate_streaming

```rust
let mut line_buffer = String::new();
const FLUSH_INTERVAL_MS: u64 = 50;
let mut last_flush = Instant::now();

while let Some(chunk) = stream.next().await {
    // ... existing parsing ...
    
    // Periodic flush for partial lines
    if last_flush.elapsed().as_millis() > FLUSH_INTERVAL_MS {
        if !line_buffer.is_empty() {
            // Try to parse any complete lines in buffer
            process_buffer(&mut line_buffer, &mut result, &mut stats, ...);
        }
        last_flush = Instant::now();
    }
}
// Final flush
if !line_buffer.is_empty() {
    process_buffer(&mut line_buffer, ...);
}
```

#### 6. nushell Detection & Simple Mode

```rust
fn is_nushell() -> bool {
    std::env::var("NU_VERSION").is_ok() 
        || std::env::var("TERM_PROGRAM").as_deref() == Ok("nushell")
}

impl StepTracker {
    fn render(&mut self) {
        if is_nushell() {
            self.render_simple(); // Single line with \r
        } else {
            self.render_multi();  // Current multi-line
        }
    }
}
```

---

## File Changes Summary

| File | Changes |
|------|---------|
| `src/spinner.rs` | Add `suspend()`, `resume()`, `stream_token()`, `start_streaming()`, `render_simple()`; disable steady tick on auxiliary bars |
| `src/main.rs` | Wrap summarize phase with `suspend()`/`resume()`; pass `steps` to `generate_streaming` for token streaming |
| `src/llamaswap.rs` | Replace `print!()` token output with `steps.stream_token()`; add buffer flush logic |

---

## Testing Strategy

1. **Unit tests** for `suspend()`/`resume()` state preservation
2. **Integration test**: Run full `gac` flow in CI with `script -qec` to capture raw ANSI
3. **Manual verification**: 
   - Windows Terminal / PowerShell / cmd
   - nushell (primary target)
   - SSH/tmux session
   - GitHub Actions (CI)

---

## Rollback Plan

If issues arise:
1. Revert `spinner.rs` and `main.rs` changes
2. Keep `llamaswap.rs` buffer improvements (independent fix)
3. Re-enable steady tick on all bars

---

## Open Questions

1. **nushell detection**: Is `NU_VERSION` reliable? Any false positives?
2. **Token streaming buffer**: Should we batch tokens (e.g., flush every 10 chars) to reduce render calls?
3. **VRAM spinner**: Currently creates temporary `step_spinner` - should it also use suspend/resume?

---

## Approval

- [ ] Design reviewed
- [ ] Approach confirmed
- [ ] Ready for implementation plan