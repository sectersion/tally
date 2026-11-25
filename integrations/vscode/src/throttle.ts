// Sends the first heartbeat per path, every write, and anything at least windowMs after the last sent one.
export class Throttle {
  private last = new Map<string, number>();
  constructor(private windowMs: number) {}
  shouldSend(path: string, isWrite: boolean, nowMs: number): boolean {
    const prev = this.last.get(path);
    if (prev !== undefined && !isWrite && nowMs - prev < this.windowMs) return false;
    this.last.set(path, nowMs);
    return true;
  }
}
