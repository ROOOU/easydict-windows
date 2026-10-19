/**
 * The settings content pane's scroll offset.
 *
 * The pane is one scroller shared by every tab, so switching tabs — or stepping
 * into a service's detail panel and back out — otherwise keeps the previous
 * offset and lands halfway down the new content. Upstream's SwiftUI
 * `SettingsView` swaps its detail view inside a fresh `ScrollView`, so each
 * navigation starts at the top.
 *
 * The pane registers itself once, which keeps the reset callable from a tab that
 * has no reference to the window's layout.
 */

let pane: { scrollTop: number } | null = null;

/** Called by the pane's `ref`; `null` detaches it on unmount. */
export function attachScrollPane(node: { scrollTop: number } | null): void {
  pane = node;
}

/** Puts the content back at the top; a no-op when the pane is not mounted. */
export function resetScroll(): void {
  if (pane !== null) {
    pane.scrollTop = 0;
  }
}
