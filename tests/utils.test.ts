import { describe, it, expect } from 'vitest';
import { pubKeyRegex, NostrEvent } from '../src/utils';

describe('utils', () => {
  it('should validate pubKeyRegex correctly', () => {
    expect(pubKeyRegex.test('abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890')).toBe(true);
    expect(pubKeyRegex.test('invalid_pubkey')).toBe(false);
  });
});