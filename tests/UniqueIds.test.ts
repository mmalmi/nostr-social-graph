import { describe, it, expect } from 'vitest';
import { UniqueIds, ID, STR } from '../src/UniqueIds';

describe('UniqueIds', () => {
  it('should convert string to unique ID and back', () => {
    const str = 'abcdef1234567890';
    const id = ID(str);
    expect(STR(id)).toBe(str);
  });

  it('should serialize and deserialize correctly', () => {
    const str = 'abcdef1234567890';
    const id = ID(str);
    const serialized = UniqueIds.serialize();
    UniqueIds.deserialize(serialized);
    expect(STR(id)).toBe(str);
  });

  it('should throw error for invalid ID', () => {
    expect(() => STR(9999)).toThrow('pub: invalid id 9999');
  });
});