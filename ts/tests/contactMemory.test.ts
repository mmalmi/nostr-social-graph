import { describe, expect, it } from "vitest";
import portableMemory from "../../fixtures/contact-memory.json";
import {
  approveContactName,
  emptyContactMemory,
  observeContactName,
  pendingContactName,
  setContactFavorite,
} from "../src/contactMemory";

describe("private contact memory", () => {
  it("remembers the first real name without accepting later profile changes", () => {
    const empty = emptyContactMemory();
    expect(observeContactName(empty, null)).toBe(empty);
    expect(observeContactName(empty, "  ")).toBe(empty);
    const saved = observeContactName(empty, "Alice");
    expect(observeContactName(saved, "Bob")).toBe(saved);
    expect(saved.first_seen_name).toBe("Alice");
    expect(saved.accepted_name).toBe("Alice");
    expect(pendingContactName(saved, "Bob")).toBe("Bob");
    expect(pendingContactName(saved, "Alice")).toBeNull();
    expect(pendingContactName(saved, " ")).toBeNull();
    expect(pendingContactName(saved, null)).toBeNull();
    expect(pendingContactName(empty, "Alice")).toBeNull();
  });

  it("requires approval of the exact current name and retains accepted history", () => {
    const original = observeContactName(emptyContactMemory(), "Alice");
    expect(approveContactName(original, "Bob", "Carol", 100)).toBe(original);
    expect(approveContactName(original, "Bob", null, 100)).toBe(original);
    expect(approveContactName(original, " ", " ", 100)).toBe(original);
    expect(approveContactName(original, "Alice", "Alice", 100)).toBe(original);
    expect(approveContactName(emptyContactMemory(), "Bob", "Bob", 100).accepted_name).toBeNull();
    const approved = approveContactName(original, "Bob", "Bob", 100);
    const next = approveContactName(approved, "Alice", "Alice", 101);
    expect(next).toEqual(portableMemory);
    expect(original.name_changes).toEqual([]);
    expect(approved.name_changes).toHaveLength(1);
    expect(JSON.parse(JSON.stringify(next))).toEqual(next);
    for (const invalidTime of [-1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
      expect(approveContactName(original, "Bob", "Bob", invalidTime)).toBe(original);
    }
  });

  it("keeps favorites independent of names and public follows", () => {
    const memory = setContactFavorite(emptyContactMemory(), true);
    expect(memory).toEqual({
      first_seen_name: null, accepted_name: null, favorite: true, name_changes: [],
    });
    const named = observeContactName(memory, "Alice");
    expect(named.favorite).toBe(true);
    const removed = setContactFavorite(named, false);
    expect(removed.accepted_name).toBe("Alice");
    expect(removed.favorite).toBe(false);
    expect(named.favorite).toBe(true);
  });
});
