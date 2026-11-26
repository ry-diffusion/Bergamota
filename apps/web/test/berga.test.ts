// berga-core's wasm build (tools/berga/wasm) driven by src/lib/berga.ts against the fake phone it carries: the same
// path the /manage/ page takes, minus WebUSB. Skipped unless src/manage/berga.wasm is built (npm run build:wasm).
import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { Core, Phone, OP, fakePipe, BergaError } from "../src/lib/berga.ts";

const wasmPath = new URL("../src/manage/berga.wasm", import.meta.url);
const skip = !existsSync(wasmPath);

test("manage page core against the fake phone", { skip }, async () => {
  const core = await Core.load(readFileSync(wasmPath));
  assert.equal(core.info.vid, 0x1209);
  assert.deepEqual(core.info.partitions, ["boot", "userdata", "logo"]);
  const phone = new Phone(core, fakePipe(core));
  const log: string[] = [];
  phone.onEvent = (op, _tag, body) => op === OP.LOG_START && log.push(new TextDecoder().decode(body));
  try {
    const hello = (await phone.call(OP.HELLO)).head;
    assert.equal(hello.proto, 1);
    assert.equal((await phone.call(OP.STATUS)).head.ios.version, "18.7.8");
    assert.ok((await phone.call(OP.METRICS)).head.cpu.clusters.length === 2);
    const { code, out } = await phone.run("uptime");
    assert.equal(code, 0);
    assert.match(out, /up/);
    await assert.rejects(phone.run("sh", ["-c", "true"]), BergaError);
    await assert.rejects(phone.push("/etc/passwd", new Uint8Array([1])), BergaError);
    await assert.rejects(phone.reboot("edl"), BergaError);
    const pic = new Uint8Array(150_000).map((_, i) => i & 0xff);
    let done = 0;
    const dest = await phone.uploadMedia("my photo.jpg", pic, (d) => (done = d));
    assert.equal(done, pic.length);
    assert.match(dest, /^\/var\/mobile\/Media\/DCIM\/100APPLE\/IMG_0001\.JPG$/);
    const png = (await phone.call(OP.SCREENSHOT)).body;
    assert.deepEqual([...png.subarray(0, 4)], [0x89, 0x50, 0x4e, 0x47]);
    await phone.call(OP.LOG_START, null, new TextEncoder().encode("100"));
    await new Promise((r) => setTimeout(r, 1300));
    assert.ok(log.join("").includes("BERGAMOTA USB"), log.join(""));
    await phone.reboot("bergamota");
  } finally {
    phone.close();
  }
});
