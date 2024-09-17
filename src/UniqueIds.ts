export type UID = number;

export type SerializedUniqueIds = [string, UID][];

/**
 * Save memory and storage by mapping repeatedly used long strings such as public keys to internal unique ID numbers.
 */
export class UniqueIds {
  private strToUniqueId = new Map<string, UID>();
  private uniqueIdToStr = new Map<UID, string>();
  private currentUniqueId = 0;

  constructor(serialized?: SerializedUniqueIds) {
    if (serialized) {
      for (const [str, id] of serialized) {
        this.strToUniqueId.set(str, id);
        this.uniqueIdToStr.set(id, str);
        this.currentUniqueId = Math.max(this.currentUniqueId, id + 1);
      }
    }
  }

  id(str: string): UID {
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
    return this.strToUniqueId.has(str);
  }

  serialize(): SerializedUniqueIds {
    return Array.from(this.strToUniqueId.entries());
  }
}
