// The berga protocol in the browser (docs/BERGA_USB.md). berga-core (Rust, tools/berga/wasm) encodes and decodes
// every frame, applies the 15 s timeout and runs the fake phone of the demo mode; this file only moves bytes:
// WebUSB transferOut/transferIn on the phone's vendor interface (class ff/62/01, kernel patch 1037), or the fake.
// Nothing leaves the computer: the page talks to the phone directly, there is no server.

interface Exports {
  memory: WebAssembly.Memory;
  bg_alloc(n: number): number;
  bg_free(p: number, n: number): void;
  bg_result_ptr(): number;
  bg_result_len(): number;
  bg_info(): number;
  bg_reset(): void;
  bg_request(op: number, hp: number, hn: number, bp: number, bn: number, now: number): number;
  bg_feed(p: number, n: number): number;
  bg_expire(now: number): number;
  bg_next(): number;
  bg_body_ptr(): number;
  bg_body_len(): number;
  bg_check(kind: number, p: number, n: number): number;
  bg_fake_reset(): void;
  bg_fake_feed(p: number, n: number): void;
  bg_fake_tick(now: number): void;
  bg_fake_take(): number;
}

export const OP = {
  HELLO: 0x0001,
  REBOOT: 0x0002,
  LOG_START: 0x0003,
  LOG_STOP: 0x0004,
  STATUS: 0x0100,
  METRICS: 0x0101,
  RUN: 0x0102,
  PUSH: 0x0103,
  PULL: 0x0104,
  SCREENSHOT: 0x0105,
  MEDIA_IMPORT: 0x0106,
} as const;

export interface Info {
  vid: number;
  pid: number;
  class: number;
  subclass: number;
  protocol: number;
  maxFrame: number;
  chunk: number;
  timeoutMs: number;
  rebootModes: string[];
  run: string[];
  partitions: string[];
}

export interface Reply {
  head: any; // eslint-disable-line @typescript-eslint/no-explicit-any -- the phone's JSON
  body: Uint8Array;
}

interface Item {
  kind: "reply" | "failed" | "event" | "timeout" | "protocol";
  tag?: number;
  op?: number;
  head?: unknown;
  error?: string;
}

export class BergaError extends Error {}

/** One instance of the wasm module: berga-core's client (and the fake phone). */
export class Core {
  private x: Exports;
  private enc = new TextEncoder();
  private dec = new TextDecoder();
  readonly info: Info;

  private constructor(x: Exports) {
    this.x = x;
    x.bg_info();
    this.info = JSON.parse(this.text());
  }

  static async load(source: BufferSource | Response | Promise<Response>): Promise<Core> {
    const { instance } =
      source instanceof Response || source instanceof Promise
        ? await WebAssembly.instantiateStreaming(source, {})
        : await WebAssembly.instantiate(source, {});
    return new Core(instance.exports as unknown as Exports);
  }

  private bytes(p: number, n: number): Uint8Array {
    return new Uint8Array(this.x.memory.buffer, p, n).slice();
  }
  result(): Uint8Array {
    return this.bytes(this.x.bg_result_ptr(), this.x.bg_result_len());
  }
  text(): string {
    return this.dec.decode(this.result());
  }
  /** Copies `data` into module memory for one call. */
  private with<T>(data: Uint8Array, f: (p: number, n: number) => T): T {
    const p = this.x.bg_alloc(data.length);
    new Uint8Array(this.x.memory.buffer, p, data.length).set(data);
    try {
      return f(p, data.length);
    } finally {
      this.x.bg_free(p, data.length);
    }
  }

  reset(): void {
    this.x.bg_reset();
  }
  request(op: number, head: unknown, body: Uint8Array, now: number): { tag: number; bytes: Uint8Array } {
    const h = head === undefined || head === null ? new Uint8Array() : this.enc.encode(JSON.stringify(head));
    const tag = this.with(h, (hp, hn) => this.with(body, (bp, bn) => this.x.bg_request(op, hp, hn, bp, bn, now)));
    if (tag < 0) throw new BergaError(this.text());
    return { tag, bytes: this.result() };
  }
  feed(data: Uint8Array): void {
    this.with(data, (p, n) => this.x.bg_feed(p, n));
  }
  expire(now: number): void {
    this.x.bg_expire(now);
  }
  next(): { item: Item; body: Uint8Array } | null {
    if (!this.x.bg_next()) return null;
    return { item: JSON.parse(this.text()), body: this.bytes(this.x.bg_body_ptr(), this.x.bg_body_len()) };
  }
  /** Same allowlists the phone enforces: throws with the reason. */
  check(kind: "push" | "pull" | "run", value: string): void {
    const rc = this.with(this.enc.encode(value), (p, n) => this.x.bg_check(kind === "push" ? 0 : kind === "pull" ? 1 : 2, p, n));
    if (rc < 0) throw new BergaError(this.text());
  }

  fakeReset(): void {
    this.x.bg_fake_reset();
  }
  fakeFeed(data: Uint8Array): void {
    this.with(data, (p, n) => this.x.bg_fake_feed(p, n));
  }
  fakeTick(now: number): void {
    this.x.bg_fake_tick(now);
  }
  fakeTake(): Uint8Array | null {
    return this.x.bg_fake_take() ? this.result() : null;
  }
}

/** Moves whole transfers; `recv` resolves with null when nothing arrived for a while. */
export interface Pipe {
  send(data: Uint8Array): Promise<void>;
  recv(): Promise<Uint8Array | null>;
  close(): Promise<void>;
  readonly label: string;
}

export function webUsbFilter(info: Info): USBDeviceFilter {
  return { classCode: info.class, subclassCode: info.subclass, protocolCode: info.protocol };
}

/** The phone's berga interface over WebUSB. */
export async function openWebUsb(dev: USBDevice, info: Info): Promise<Pipe> {
  if (!dev.opened) await dev.open();
  if (!dev.configuration) await dev.selectConfiguration(dev.configurations[0].configurationValue);
  const match = (a: USBAlternateInterface) => a.interfaceClass === info.class && a.interfaceSubclass === info.subclass && a.interfaceProtocol === info.protocol;
  const iface = dev.configuration!.interfaces.find((i) => i.alternates.some(match));
  if (!iface) throw new BergaError("this USB device has no berga interface (it needs an iOS boot with kernel patch 1037)");
  try {
    await dev.claimInterface(iface.interfaceNumber);
  } catch (e) {
    throw new BergaError(`the browser could not claim the berga interface: close berga and other tabs using the phone, then replug (${(e as Error).message})`);
  }
  const alt = iface.alternates.find(match)!;
  const epIn = alt.endpoints.find((e) => e.type === "bulk" && e.direction === "in")?.endpointNumber;
  const epOut = alt.endpoints.find((e) => e.type === "bulk" && e.direction === "out")?.endpointNumber;
  if (!epIn || !epOut) throw new BergaError("berga interface without bulk endpoints");
  return {
    label: `${dev.productName ?? "phone"} (USB ${dev.vendorId.toString(16).padStart(4, "0")}:${dev.productId.toString(16).padStart(4, "0")})`,
    async send(data) {
      const r = await dev.transferOut(epOut, data as BufferSource);
      if (r.status !== "ok") throw new BergaError(`USB write failed (${r.status})`);
    },
    async recv() {
      // a pending transferIn simply waits for the phone; the read loop keeps exactly one in flight
      const r = await dev.transferIn(epIn, info.maxFrame);
      if (r.status !== "ok" || !r.data) throw new BergaError(`USB read failed (${r.status})`);
      return new Uint8Array(r.data.buffer, r.data.byteOffset, r.data.byteLength);
    },
    async close() {
      try {
        await dev.releaseInterface(iface.interfaceNumber);
        await dev.close();
      } catch {
        /* already gone */
      }
    },
  };
}

/** The fake phone of berga-core, inside the page (demo mode). */
export function fakePipe(core: Core): Pipe {
  core.fakeReset();
  const t0 = performance.now();
  return {
    label: "Demo phone (fake, in this page)",
    async send(data) {
      core.fakeFeed(data);
    },
    async recv() {
      for (let i = 0; i < 5; i++) {
        core.fakeTick(performance.now() - t0);
        const b = core.fakeTake();
        if (b) return b;
        await new Promise((r) => setTimeout(r, 50));
      }
      return null;
    },
    async close() {},
  };
}

type Pending = { resolve: (r: Reply) => void; reject: (e: Error) => void; op: number };

/** A connection: request/response with the 15 s timeout, events to `onEvent`. */
export class Phone {
  private pending = new Map<number, Pending>();
  private closed = false;
  private t0 = performance.now();
  onEvent: (op: number, tag: number, body: Uint8Array) => void = () => {};
  onClose: (why: string) => void = () => {};

  readonly core: Core;
  readonly pipe: Pipe;

  constructor(core: Core, pipe: Pipe) {
    this.core = core;
    this.pipe = pipe;
    core.reset();
    void this.readLoop();
    const timer = setInterval(() => {
      if (this.closed) return clearInterval(timer);
      this.core.expire(this.now());
      this.drain();
    }, 500);
  }

  private now(): number {
    return performance.now() - this.t0;
  }

  private drain(): void {
    for (let n = this.core.next(); n; n = this.core.next()) {
      const { item, body } = n;
      const p = item.tag !== undefined ? this.pending.get(item.tag) : undefined;
      if (item.kind === "event") this.onEvent(item.op!, item.tag!, body);
      else if (item.kind === "protocol") this.onEvent(-1, 0, new TextEncoder().encode(`[berga] ${item.error}\n`));
      else if (p) {
        this.pending.delete(item.tag!);
        if (item.kind === "reply") p.resolve({ head: item.head, body });
        else if (item.kind === "failed") p.reject(new BergaError(item.error ?? "the phone refused"));
        else p.reject(new BergaError("no answer from the phone in 15 s"));
      }
    }
  }

  private async readLoop(): Promise<void> {
    while (!this.closed) {
      try {
        const b = await this.pipe.recv();
        if (b) {
          this.core.feed(b);
          this.drain();
        }
      } catch (e) {
        if (!this.closed) this.close((e as Error).message);
      }
    }
  }

  call(op: number, head?: unknown, body: Uint8Array = new Uint8Array()): Promise<Reply> {
    if (this.closed) return Promise.reject(new BergaError("not connected"));
    const { tag, bytes } = this.core.request(op, head, body, this.now());
    return new Promise<Reply>((resolve, reject) => {
      this.pending.set(tag, { resolve, reject, op });
      this.pipe.send(bytes).catch((e: Error) => {
        this.pending.delete(tag);
        reject(e);
      });
    });
  }

  async push(path: string, data: Uint8Array, onBytes: (done: number) => void = () => {}): Promise<void> {
    this.core.check("push", path);
    const chunk = this.core.info.chunk;
    for (let off = 0; off < data.length || off === 0; off += chunk) {
      await this.call(OP.PUSH, { path, offset: off, truncate: off === 0, mode: 0o644 }, data.subarray(off, off + chunk));
      onBytes(Math.min(off + chunk, data.length));
      if (data.length === 0) break;
    }
  }

  /** A picture or video into the camera roll. */
  async uploadMedia(name: string, data: Uint8Array, onBytes?: (done: number) => void): Promise<string> {
    const safe = name.replace(/[^A-Za-z0-9._-]/g, "_").replace(/^\.+/, "");
    const staged = `/var/mobile/Media/berga/${safe || "upload.jpg"}`;
    await this.push(staged, data, onBytes);
    return (await this.call(OP.MEDIA_IMPORT, { path: staged })).head.path;
  }

  async run(cmd: string, args: string[] = []): Promise<{ code: number; out: string }> {
    this.core.check("run", JSON.stringify({ cmd, args }));
    const r = await this.call(OP.RUN, { cmd, args });
    return { code: r.head.code, out: new TextDecoder().decode(r.body) };
  }

  async reboot(mode: string): Promise<void> {
    if (!this.core.info.rebootModes.includes(mode)) throw new BergaError(`unknown reboot mode ${mode}`);
    await this.call(OP.REBOOT, null, new TextEncoder().encode(mode));
  }

  close(why = "disconnected"): void {
    if (this.closed) return;
    this.closed = true;
    for (const p of this.pending.values()) p.reject(new BergaError(why));
    this.pending.clear();
    void this.pipe.close();
    this.onClose(why);
  }
}
