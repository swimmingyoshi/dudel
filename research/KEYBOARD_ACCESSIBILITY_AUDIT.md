# Keyboard and accessibility source audit, 2026-10-17

Scope: current source and installed egui/eframe 0.33.3. This is a source audit,
not Windows Narrator, actual keyboard, contrast or full focus-order acceptance.

## Confirmed implementation facts

- eframe's default features include AccessKit; Dudel leaves those defaults enabled.
  This establishes inclusion of the framework, not usable labels or screen-reader
  coverage for every custom-painted control.
- `Dudel::raw_input_hook` calls `workspace::take_panel_toggle` before the egui pass.
- With no focused widget, that helper removes every Tab event, including Shift+Tab
  and Ctrl+Tab. Only a nonrepeated plain Tab press toggles panels.
- With any widget focused, `ctx.wants_keyboard_input()` returns true and the helper
  leaves events to egui. This predicate means any focus, not only text editing;
  the helper comment currently describes it too narrowly.
- Existing `tab_toggles_panels_without_moving_keyboard_focus` deliberately verifies
  removal of modified Tab along with ordinary Tab. Shortcut handling also defers
  while a widget has focus.

## Open finding and corrective work

There is no explicit keyboard entry action in this helper for moving from an
unfocused canvas into the controls. Ordinary Tab is reserved for panel visibility,
so the release needs a documented entry mechanism that preserves that drawing
shortcut. Verify native alternatives before making a broader keyboard-only claim.

Prepared fix: F6 enters control focus using a balanced egui Tab event pair, then
egui owns Tab/Shift+Tab while focused. The shortcut is listed in the in-app help.
New egui regressions exercise forward/backward navigation, Space activation,
text entry, Escape and returning to the panel-toggle shortcut. A second regression
uses the actual workspace to enter File, traverse controls and open its menu.
The workflow harness now calls the production raw-input hook before each pass.

Formatting passed. Low-priority, single-job compilation and behavioral verification
are now **in progress** under `target/qa/keyboard-diagnostic-verification-20261017`.
Their overlap with the engineering soak is recorded in its `CONTENTION.json`;
that run cannot establish isolated timing acceptance. Do not treat this
implementation as accepted or package it before the new tests, affected workflows
and required checks pass. The running study's frozen executable is unaffected by
these working-tree changes.

G3 remains open for common-action reachability, custom-control names, focus order,
short-window/DPI layouts, visible focus/contrast and actual Windows screen-reader
operation. Framework feature inclusion does not close those checks.

Default verification passed: 788 tests, zero failed, two intentionally ignored.
Both the actual workspace menu test and the control/text/navigation test pass.
The latter waits for egui's documented next-pass backward-focus change. The full
workflow harness now uses the production raw-input hook. Log:
`target/qa/keyboard-diagnostic-verification-20261017/default-tests.log`.
All-feature verification also passed: 797 tests, zero failed, two ignored. Both
warnings-denied Clippy configurations pass. The isolated-target release build is
still running; `STATE.json` is the authoritative checkpoint. Compilation began at
below-normal priority; the release compiler was later returned to normal priority.
The engineering soak's contention record retains that change.
