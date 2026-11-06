export type UID = number;

export function canonicalNodeId(value: string): string {
  return /^[0-9a-fA-F]{64}$/.test(value) ? value.toLowerCase() : value;
}

/**
 * Save memory and storage by mapping repeatedly used long strings such as public keys to internal unique ID numbers.
 */
export class UniqueIds {
  private strToUniqueId = new Map<string, UID>();
  private uniqueIdToStr = new Map<UID, string>();
  private currentUniqueId = 0;

  constructor() {
  }

  id(str: string): UID {
    str = canonicalNodeId(str);
    // Prevent empty strings from being stored
    if (!str || str.trim() === '') {
      throw new Error('Cannot store empty or whitespace-only strings');
    }
    
    const existing = this.strToUniqueId.get(str);
    if (existing !== undefined) {
      return existing;
    }
    const newId = this.currentUniqueId++;
    this.strToUniqueId.set(str, newId);
    this.uniqueIdToStr.set(newId, str);
    return newId;
  }

  str(id: UID): string {
    const pub = this.uniqueIdToStr.get(id);
    if (!pub) {
      throw new Error('pub: invalid id ' + id);
    }
    return pub;
  }

  has(str: string): boolean {
    return this.strToUniqueId.has(canonicalNodeId(str));
  }

  *[Symbol.iterator]() {
    yield* this.uniqueIdToStr.entries();
  }

  remove(id: UID): void {
    const str = this.uniqueIdToStr.get(id);
    if (str !== undefined) {
      this.uniqueIdToStr.delete(id);
      this.strToUniqueId.delete(str);
    }
  }
}
