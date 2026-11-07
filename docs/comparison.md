# Comparison: GPUI vs Avalonia

Same PC, release builds, same accounts. Fill in after each milestone.

| Measure | Avalonia 0.13.3 | GPUI | How measured |
|---|---|---|---|
| Startup to tray | | ~230 ms (spike) | In-process, start to tray ready |
| Idle memory (tray only) | 77 MB WS / 129 MB private | 46 MB WS / 58 MB private (spike) | 2026-11-06, both running; spike reads 1 account once |
| Idle memory (window open) | | ~65 MB (spike popover) | |
| Download size | 42.4 MB | 8.3 MB (spike, no LTO) | Release ZIP |
| Idle CPU in tray | 94 ms CPU / 30 s | 172 ms CPU / 30 s (spike, 2 s timer) | Over 30 s; 10 min run pending |
| Look and feel | | | Owner's notes |
| Ease of change | | | Notes from real changes |

Spike numbers are early: the spike does far less work than 0.13.3. Re-measure with the core slice.
