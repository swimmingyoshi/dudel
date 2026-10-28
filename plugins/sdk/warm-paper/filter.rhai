// SPDX-License-Identifier: GPL-3.0-only
// Channels are integers 0..255. Return [r, g, b, a].
fn filter(r, g, b, a) {
    [min(255, r + 12), min(255, g + 4), max(0, b - 12), a]
}
