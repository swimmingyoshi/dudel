# research

Diagnostic study tools and the fixtures some tests use.

- `analyze_*.py`, `assess_*.py`, `plot_native_memory.py`: read the reports of the
  native soak and allocation studies (`scripts/run_soak.py`,
  `RunPerformanceStudy.ps1`). Release packages include the ones the study needs.
- `test_*.py`: tests for these tools and for the packaging scripts; `Verify`
  runs them.
- `fixtures/`: small CSP and input-replay files used by the Rust tests.

Older engineering notes and measurements were moved to the project's archive
on 2026-11-09.
