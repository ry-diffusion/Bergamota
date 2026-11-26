<script lang="ts">
  import { onDestroy } from "svelte";
  import { Core, Phone, OP, openWebUsb, fakePipe, webUsbFilter, type Pipe } from "../lib/berga.ts";
  import wasmUrl from "./berga.wasm?url";

  const webusb = typeof navigator !== "undefined" && "usb" in navigator && window.isSecureContext;
  let core: Core | null = null;
  let phone = $state<Phone | null>(null);
  let label = $state("");
  let hello = $state<any>(null);
  let status = $state<any>(null);
  let metrics = $state<any>(null);
  let errors = $state<Record<string, string>>({});
  let busy = $state(false);
  let logOn = $state(false);
  let logText = $state("");
  let logBox: HTMLPreElement | undefined = $state();
  let shot = $state<string | null>(null);
  let uploads = $state<{ name: string; done: number; total: number; dest?: string; error?: string }[]>([]);
  let armed = $state<string | null>(null);
  let timer: ReturnType<typeof setInterval> | undefined;

  const reboots: [string, string, string][] = [
    ["normal", "Restart", "iOS again"],
    ["fastboot", "ABL fastboot", "Qualcomm's bootloader fastboot"],
    ["bergamota", "Bergamota fastboot", "Enxerto's fastboot: flash from /flash/"],
    ["recovery", "Recovery", "the recovery partition"],
    ["poweroff", "Power off", "PSCI system off"],
  ];

  function fail(section: string, e: unknown) {
    errors[section] = (e as Error).message ?? String(e);
  }
  function ok(section: string) {
    delete errors[section];
  }

  async function loadCore(): Promise<Core> {
    core ??= await Core.load(fetch(wasmUrl));
    return core;
  }

  async function start(pipe: Pipe) {
    const c = await loadCore();
    const p = new Phone(c, pipe);
    p.onEvent = (op, _tag, body) => {
      if (op === OP.LOG_START || op === -1) {
        const lines = (logText + new TextDecoder().decode(body)).split("\n");
        logText = lines.slice(-2000).join("\n");
        queueMicrotask(() => logBox && (logBox.scrollTop = logBox.scrollHeight));
      }
    };
    p.onClose = (why) => {
      if (phone === p) disconnect(why);
    };
    phone = p;
    label = pipe.label;
    hello = (await p.call(OP.HELLO)).head;
    void refreshStatus();
    void refreshMetrics();
    timer = setInterval(() => document.visibilityState === "visible" && refreshMetrics(), 2000);
  }

  async function connectUsb() {
    busy = true;
    errors = {};
    try {
      const c = await loadCore();
      const dev = await navigator.usb.requestDevice({ filters: [webUsbFilter(c.info)] });
      await start(await openWebUsb(dev, c.info));
    } catch (e) {
      if ((e as Error).name !== "NotFoundError") fail("connect", e);
    } finally {
      busy = false;
    }
  }

  async function connectDemo() {
    busy = true;
    errors = {};
    try {
      await start(fakePipe(await loadCore()));
    } catch (e) {
      fail("connect", e);
    } finally {
      busy = false;
    }
  }

  function disconnect(why?: string) {
    clearInterval(timer);
    const p = phone;
    phone = null;
    hello = status = metrics = null;
    logOn = false;
    p?.close();
    if (why && why !== "disconnected") errors = { connect: `Connection lost: ${why}` };
  }
  onDestroy(() => disconnect());

  async function refreshStatus() {
    try {
      status = (await phone!.call(OP.STATUS)).head;
      ok("status");
    } catch (e) {
      fail("status", e);
    }
  }
  async function refreshMetrics() {
    if (!phone) return;
    try {
      metrics = (await phone.call(OP.METRICS)).head;
      ok("metrics");
    } catch (e) {
      fail("metrics", e);
    }
  }
  async function toggleLog() {
    try {
      if (logOn) await phone!.call(OP.LOG_STOP);
      else await phone!.call(OP.LOG_START, null, new TextEncoder().encode("16384"));
      logOn = !logOn;
      ok("log");
    } catch (e) {
      fail("log", e);
    }
  }
  async function screenshot() {
    try {
      const r = await phone!.call(OP.SCREENSHOT);
      if (shot) URL.revokeObjectURL(shot);
      shot = URL.createObjectURL(new Blob([r.body as BlobPart], { type: "image/png" }));
      ok("screen");
    } catch (e) {
      fail("screen", e);
    }
  }
  async function upload(files: FileList | null) {
    for (const f of Array.from(files ?? [])) {
      uploads = [{ name: f.name, done: 0, total: f.size }, ...uploads].slice(0, 12);
      const u = uploads[0]; // the reactive proxy: updates below re-render
      try {
        u.dest = await phone!.uploadMedia(f.name, new Uint8Array(await f.arrayBuffer()), (d) => (u.done = d));
      } catch (e) {
        u.error = (e as Error).message;
      }
    }
  }
  async function reboot(mode: string) {
    if (armed !== mode) {
      armed = mode;
      return;
    }
    armed = null;
    try {
      await phone!.reboot(mode);
      ok("power");
      setTimeout(() => disconnect(), 800);
    } catch (e) {
      fail("power", e);
    }
  }

  const mhz = (khz?: number) => (khz == null ? "–" : khz >= 1e6 ? `${(khz / 1e6).toFixed(2)} GHz` : `${Math.round(khz / 1e3)} MHz`);
  const pct = (v?: number | null) => (v == null ? "–" : `${Math.round(v)}%`);
  const gib = (b?: number) => (b == null ? "–" : `${(b / 2 ** 30).toFixed(1)} GiB`);
  const dur = (s?: number) => (s == null ? "–" : s < 3600 ? `${Math.floor(s / 60)} min` : `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} min`);
  const rootDisk = $derived(Array.isArray(status?.disk) ? status.disk.find((d: any) => d.mount === "/private/var") ?? status.disk[0] : status?.disk);
</script>

<div class="page">
  <header class="top">
    <a class="mark" href="/"><img src="/favicon.svg" alt="" width="32" height="32" /> Bergamota <span>Manage</span></a>
    <nav class="nav"><a href="/flash/">Flasher</a></nav>
  </header>

  {#if !phone}
    <section class="sheet intro" aria-labelledby="connect-title">
      <h2 id="connect-title">Plug in your phone running Bergamota.</h2>
      <p>This page talks to the phone's own USB interface straight from the browser: status, live kernel log, metrics, screenshots, photos and restarts. Nothing goes through a server.</p>
      {#if !webusb}
        <p class="problem">This browser has no WebUSB (Chrome and Edge on desktop do). You can still try the demo.</p>
      {/if}
      {#if errors.connect}
        <p class="problem" role="alert">{errors.connect}</p>
      {/if}
      <div class="row">
        <button class="btn leaf" disabled={busy || !webusb} onclick={connectUsb}>{busy ? "Talking to the phone…" : "Find my phone"}</button>
        <button class="btn ghost" disabled={busy} onclick={connectDemo}>Try the demo</button>
      </div>
      <div class="note">
        <div>
          <b>What the phone needs</b>
          An iOS boot with the Heartwood kernel from patch 1037 on (it adds the "Bergamota berga" USB interface next to USB Ethernet). Status, metrics, screenshots and photos also need <code>bergad</code> on the phone; the log and restarts work without it.
        </div>
        <div>
          <b>Linux</b>
          Add a udev rule so Chrome can open the phone, then replug:
          <pre>SUBSYSTEM=="usb", ATTR&#123;idVendor&#125;=="1209", ATTR&#123;idProduct&#125;=="0001", MODE="0660", TAG+="uaccess"</pre>
          <b style="margin-top: 14px">Windows, macOS, ChromeOS</b>
          No setup: the phone tells Windows to use WinUSB by itself.
        </div>
      </div>
    </section>
  {:else}
    <div class="chip">
      <i class="dot"></i>
      <b>{label}</b>
      <span>{hello?.kernel ?? ""}</span>
      <span>up {dur((hello?.uptime_ms ?? 0) / 1000)} · daemon {hello?.daemon ? "running" : "not running"}</span>
      <button class="btn ghost small" onclick={() => disconnect()}>Disconnect</button>
    </div>

    <main class="grid">
      <section class="sheet" aria-labelledby="m-title">
        <h3 id="m-title">Live</h3>
        {#if errors.metrics}<p class="problem">{errors.metrics}</p>{/if}
        {#if metrics}
          {#each metrics.cpu?.clusters ?? [] as c}
            <div class="meter">
              <span>{c.name} <small>{mhz(c.curKhz)}</small></span>
              <i style:--v={`${Math.min(100, c.loadPct ?? 0)}%`}></i>
              <b>{pct(c.loadPct)}</b>
            </div>
          {/each}
          <div class="meter">
            <span>GPU <small>{metrics.gpu?.mhz ? `${metrics.gpu.mhz} MHz` : metrics.gpu?.state ?? ""}</small></span>
            <i style:--v={`${Math.min(100, metrics.gpu?.busyPct ?? 0)}%`}></i>
            <b>{pct(metrics.gpu?.busyPct)}</b>
          </div>
          <table class="summary">
            <tbody>
              <tr><td>Battery</td><td>{metrics.battery?.present ? `${pct(metrics.battery.percent)} · ${metrics.battery.charging ? "charging" : "on battery"}${metrics.battery.currentMa != null ? ` · ${Math.round(metrics.battery.currentMa)} mA` : ""}` : "not present"}</td></tr>
              <tr><td>Temperature</td><td>{metrics.thermal?.batteryC != null ? `${metrics.thermal.batteryC.toFixed(1)} °C` : "–"}</td></tr>
              <tr><td>Renderer</td><td>{metrics.renderer?.label ?? "–"}</td></tr>
            </tbody>
          </table>
        {:else if !errors.metrics}
          <p class="muted">Reading…</p>
        {/if}
      </section>

      <section class="sheet" aria-labelledby="s-title">
        <h3 id="s-title">Phone</h3>
        {#if errors.status}<p class="problem">{errors.status}</p>{/if}
        <table class="summary">
          <tbody>
            <tr><td>iOS</td><td>{status?.ios?.version ?? "–"} <small>{status?.ios?.build ?? ""}</small></td></tr>
            <tr><td>Uptime</td><td>{dur(status?.uptime_s)}</td></tr>
            <tr><td>Free space</td><td>{gib(rootDisk?.free)} of {gib(rootDisk?.total)}</td></tr>
            <tr><td>Kernel</td><td class="mono">{status?.kernel ?? hello?.kernel ?? "–"}</td></tr>
          </tbody>
        </table>
        <div class="row"><button class="btn ghost small" onclick={refreshStatus}>Refresh</button></div>
      </section>

      <section class="sheet wide" aria-labelledby="l-title">
        <div class="head">
          <h3 id="l-title">Kernel log</h3>
          <div class="row tight">
            <button class="btn small" class:leaf={!logOn} onclick={toggleLog}>{logOn ? "Stop" : "Follow"}</button>
            <button class="btn ghost small" onclick={() => (logText = "")}>Clear</button>
          </div>
        </div>
        {#if errors.log}<p class="problem">{errors.log}</p>{/if}
        <pre class="log" bind:this={logBox} aria-live="off">{logText || "The console log streams here: Follow starts with the last 16 KiB."}</pre>
      </section>

      <section class="sheet" aria-labelledby="sc-title">
        <h3 id="sc-title">Screen</h3>
        {#if errors.screen}<p class="problem">{errors.screen}</p>{/if}
        {#if shot}
          <img class="shot" src={shot} alt="Screenshot of the phone" />
        {/if}
        <div class="row">
          <button class="btn leaf small" onclick={screenshot}>Take screenshot</button>
          {#if shot}<a class="btn ghost small" href={shot} download="bergamota-screenshot.png">Save</a>{/if}
        </div>
      </section>

      <section class="sheet" aria-labelledby="p-title">
        <h3 id="p-title">Photos</h3>
        <p class="muted">Pictures and videos go into the camera roll (Photos may need a relaunch to show them).</p>
        <label class="drop">
          <b>Choose pictures or videos</b>
          <input type="file" accept="image/*,video/*" multiple onchange={(e) => upload((e.currentTarget as HTMLInputElement).files)} />
        </label>
        {#each uploads as u}
          <p class="up" class:bad={u.error}>
            <b>{u.name}</b>
            {#if u.error}{u.error}{:else if u.dest}→ {u.dest}{:else}{Math.round((100 * u.done) / Math.max(1, u.total))}%{/if}
          </p>
        {/each}
      </section>

      <section class="sheet" aria-labelledby="w-title">
        <h3 id="w-title">Power</h3>
        {#if errors.power}<p class="problem">{errors.power}</p>{/if}
        <div class="power">
          {#each reboots as [mode, name, what]}
            <button class:btn={true} class:danger={armed === mode} class:ghost={armed !== mode} class="small" onclick={() => reboot(mode)} title={what}>
              {armed === mode ? `Confirm: ${name}` : name}
            </button>
          {/each}
        </div>
        <p class="muted">Boot images and flashing go through fastboot: restart into Bergamota fastboot, then open the <a href="/flash/">Flasher</a>.</p>
      </section>
    </main>
  {/if}
</div>

<style>
  .page { min-height: 100vh; padding: 0 clamp(16px, 3.4vw, 48px) 48px; max-width: 1280px; margin: 0 auto; }
  .top { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 26px 0 30px; }
  .mark { display: inline-flex; align-items: center; gap: 12px; font-weight: 900; font-size: 24px; font-stretch: 112%; letter-spacing: -0.02em; text-decoration: none; }
  .mark img { border-radius: 8px; }
  .mark span { font-weight: 600; font-stretch: 100%; }
  .nav a { font-weight: 800; }
  .intro { max-width: 860px; }
  .chip .btn { margin-left: auto; color: var(--white); box-shadow: inset 0 0 0 2px var(--white); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 340px), 1fr)); gap: 18px; align-items: start; }
  .wide { grid-column: 1 / -1; }
  h3 { margin: 0 0 16px; font-size: 28px; font-weight: 850; letter-spacing: -0.03em; line-height: 1; }
  .head { display: flex; justify-content: space-between; align-items: center; gap: 12px; flex-wrap: wrap; }
  .head h3 { margin: 0; }
  .row.tight { margin-top: 0; }
  .btn.small { min-height: 40px; padding: 0 18px; font-size: 15px; }
  .meter { display: grid; grid-template-columns: 9em 1fr 3.2em; align-items: center; gap: 12px; margin: 0 0 12px; font-weight: 700; }
  .meter small { font: 400 12px var(--mono); color: var(--ink-soft); }
  .meter i { height: 12px; border-radius: 6px; background: linear-gradient(90deg, var(--leaf) var(--v), var(--paper-line) var(--v)); transition: background 400ms; }
  .meter b { text-align: right; font-variant-numeric: tabular-nums; }
  .mono { font: 400 12px/1.5 var(--mono) !important; }
  .log { margin: 16px 0 0; height: 340px; overflow: auto; background: var(--ink); color: #d9e7b0; font: 400 12px/1.5 var(--mono); padding: 14px 16px; border-radius: 4px; white-space: pre-wrap; word-break: break-all; }
  .shot { display: block; max-height: 420px; margin: 0 auto 4px; border-radius: 10px; box-shadow: var(--lift); }
  .drop { cursor: pointer; }
  .drop input { max-width: 46%; font-size: 13px; }
  .up { margin: 10px 0 0; font: 400 13px/1.4 var(--mono); word-break: break-all; }
  .up b { font-family: var(--display); font-size: 14px; margin-right: 8px; }
  .up.bad { color: var(--warn); }
  .power { display: flex; flex-wrap: wrap; gap: 10px; margin-bottom: 14px; }
</style>
