import { describe, it, expect } from 'vitest';
import { UniqueIds } from '../src/UniqueIds';

describe('UniqueIds', () => {
  it('should convert string to unique ID and back', () => {
    const str = 'abcdef1234567890';
    const ids = new UniqueIds();
    const id = ids.id(str);
    expect(ids.str(id)).toBe(str);
  });

  it('should serialize and deserialize correctly', () => {
    const str = 'abcdef1234567890';
    const ids = new UniqueIds();
    const id = ids.id(str);
    const serialized = ids.serialize();
    const ids2 = new UniqueIds(serialized);
    expect(ids2.str(id)).toBe(str);
  });

  it('should throw error for invalid ID', () => {
    const ids = new UniqueIds();
    expect(() => ids.str(9999)).toThrow('pub: invalid id 9999');
  });

  // New test case
  it('should return the same ID for the same input', () => {
    const str = 'abcdef1234567890';
    const ids = new UniqueIds();
    const id1 = ids.id(str);
    const id2 = ids.id(str);
    expect(id1).toBe(id2);
  });
});