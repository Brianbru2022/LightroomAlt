export type SelectionState = {
  activeId: string | null;
  anchorId: string | null;
  selectedIds: ReadonlySet<string>;
};

export type SelectionModifiers = { toggle?: boolean; range?: boolean; additiveRange?: boolean };

export const emptySelection = (): SelectionState => ({ activeId: null, anchorId: null, selectedIds: new Set() });

/**
 * Applies desktop selection semantics against the caller's exact visible order.
 * Hidden IDs are retained unless the user performs a replacing selection.
 */
export function selectId(state: SelectionState, id: string, visibleOrder: readonly string[], modifiers: SelectionModifiers = {}): SelectionState {
  if (modifiers.range && state.anchorId) {
    const anchor = visibleOrder.indexOf(state.anchorId);
    const target = visibleOrder.indexOf(id);
    if (anchor >= 0 && target >= 0) {
      const range = visibleOrder.slice(Math.min(anchor, target), Math.max(anchor, target) + 1);
      const selectedIds = modifiers.additiveRange ? new Set(state.selectedIds) : new Set<string>();
      range.forEach(value => selectedIds.add(value));
      return { activeId: id, anchorId: state.anchorId, selectedIds };
    }
  }
  if (modifiers.toggle) {
    const selectedIds = new Set(state.selectedIds);
    if (selectedIds.has(id) && selectedIds.size > 1) selectedIds.delete(id); else selectedIds.add(id);
    const activeId = selectedIds.has(id) ? id : (state.activeId === id ? selectedIds.values().next().value ?? null : state.activeId);
    return { activeId, anchorId: id, selectedIds };
  }
  return { activeId: id, anchorId: id, selectedIds: new Set([id]) };
}

export function selectAll(ids: readonly string[], activeId: string | null): SelectionState {
  const selectedIds = new Set(ids);
  const active = activeId && selectedIds.has(activeId) ? activeId : ids[0] ?? null;
  return { activeId: active, anchorId: active, selectedIds };
}

export function clearSelection(): SelectionState { return emptySelection(); }

export function navigateSelection(state: SelectionState, visibleOrder: readonly string[], delta: number): SelectionState {
  if (!visibleOrder.length) return state;
  const current = state.activeId ? visibleOrder.indexOf(state.activeId) : -1;
  const next = visibleOrder[Math.min(visibleOrder.length - 1, Math.max(0, current < 0 ? 0 : current + delta))];
  return next ? { activeId: next, anchorId: next, selectedIds: new Set([next]) } : state;
}

export function visibleSelectionCount(selectedIds: ReadonlySet<string>, visibleIds: readonly string[]): number {
  let count = 0;
  for (const id of visibleIds) if (selectedIds.has(id)) count += 1;
  return count;
}
