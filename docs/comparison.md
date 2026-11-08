# Comparison: GPUI vs Avalonia

The GPUI rewrite (ADR 0001) against the last Avalonia build, same PC. Avalonia numbers are from
2026-11-06 with both apps running; GPUI numbers from 2026-11-08, release build, demo data, run
hidden (`--demo --demo-hidden`: no tray icon or taskbar badges are drawn, so real use adds a little).

| Measure | Avalonia 0.13.3 | GPUI 0.1.7 | How measured |
|---|---|---|---|
| Download size | 42.4 MB (ZIP) | 23.2 MB (one exe) | Release asset |
| Idle memory, tray only | 77 MB WS / 129 MB private | 53 MB WS / 59 MB private | After 15 s, then 60 s idle |
| Idle CPU, tray only | 94 ms / 30 s | 100-200 ms / 30 s (203-406 ms / 60 s; old settings 344 ms) | `TotalProcessorTime` over 60 s |
| Memory, main window open | – | 69 MB WS / 104 MB private | Statistics window, Overview |
| CPU, main window open | – | 720-780 ms / 30 s (old settings: 530-630 ms) | Same, over 30 s, 2 runs each |

Release build settings (2026-11-08): full LTO, `opt-level = "s"`, stripped (`panic = "abort"` would give 15.2 MB, but a panic in a background task would close the app; owner chose to keep unwinding). The CPU and memory numbers in the table are from the 15.2 MB build; the 23.2 MB build differs only in panic handling.
Same demo run with other settings: thin LTO 32.3 MB, full LTO 31.3 MB, full LTO + "s" 23.2 MB,
+ "z" 20.8 MB, "z" + abort 13.3 MB; memory stayed the same; CPU moved within run-to-run noise (main window open: "s" ~20 % higher in two short runs; tray only: "s" lower).

Notes: idle CPU in the tray is still about twice the Avalonia app's; where it goes has not been
looked at yet (timers such as the 3 s theme check are candidates).
