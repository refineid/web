// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
// Real browser smoke test using installed Google Chrome and loopback HTTP server.
// Uses native Node.js (v22+) WebSocket & fetch with Chrome DevTools Protocol (CDP).
// No external npm dependencies or CDN required.

const { spawn } = require("child_process");
const net = require("net");
const fs = require("fs");
const os = require("os");
const path = require("path");

// Helper to find a free port
function getFreePort(preferred = 0) {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(preferred, "127.0.0.1", () => {
      const port = srv.address().port;
      srv.close(() => resolve(port));
    });
    srv.on("error", () => {
      if (preferred !== 0) {
        // Preferred port busy; get any free port
        getFreePort(0).then(resolve, reject);
      } else {
        reject(new Error("Could not find free port"));
      }
    });
  });
}

function flush(ms = 100) {
  return new Promise((r) => setTimeout(r, ms));
}

async function runBrowserSmoke() {
  console.log("====================================================");
  console.log("Starting Real Google Chrome WebUSB Browser Smoke Test");
  console.log("====================================================");

  const repoRoot = path.resolve(__dirname, "..");
  const chromePath = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  if (!fs.existsSync(chromePath)) {
    throw new Error("Google Chrome binary not found at " + chromePath);
  }

  // 1. Pick ports
  const httpPort = await getFreePort(8080);
  const cdpPort = await getFreePort(9333);
  console.log(`[setup] Selected HTTP server port: ${httpPort}, CDP port: ${cdpPort}`);

  // 2. Start loopback python3 HTTP server
  console.log(`[setup] Spawning python3 http.server on 127.0.0.1:${httpPort} serving public/`);
  const httpServer = spawn(
    "python3",
    [
      "-m",
      "http.server",
      String(httpPort),
      "--bind",
      "127.0.0.1",
      "--directory",
      path.join(repoRoot, "public"),
    ],
    { stdio: "ignore" },
  );

  // Verify HTTP server is ready
  let serverReady = false;
  for (let i = 0; i < 30; i++) {
    await flush(100);
    try {
      const res = await fetch(`http://127.0.0.1:${httpPort}/index.html`);
      if (res.ok) {
        serverReady = true;
        break;
      }
    } catch (e) {}
  }
  if (!serverReady) {
    httpServer.kill("SIGTERM");
    throw new Error(`HTTP server failed to respond on 127.0.0.1:${httpPort}`);
  }
  console.log(`[setup] Loopback HTTP server ready at http://127.0.0.1:${httpPort}/index.html`);

  // 3. Launch Chrome headless with clean temp user-data-dir
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "refineid-chrome-smoke-"));
  console.log(`[setup] Launching Chrome headless (Profile: ${tmpDir})`);
  const chrome = spawn(
    chromePath,
    [
      "--headless=new",
      "--no-first-run",
      "--no-default-browser-check",
      "--user-data-dir=" + tmpDir,
      "--remote-debugging-port=" + cdpPort,
      "about:blank",
    ],
    { stdio: "ignore" },
  );

  let ws = null;
  let checksPassed = 0;
  let checksFailed = 0;

  function assert(name, condition, extra = "") {
    if (condition) {
      checksPassed++;
      console.log(`  PASS: ${name}`);
    } else {
      checksFailed++;
      console.log(`  FAIL: ${name} ${extra}`);
    }
  }

  try {
    // 4. Connect to Chrome CDP
    let target = null;
    let browserVersion = "";
    let browserUserAgent = "";
    for (let i = 0; i < 40; i++) {
      await flush(150);
      try {
        const verRes = await fetch(`http://127.0.0.1:${cdpPort}/json/version`);
        const ver = await verRes.json();
        browserVersion = ver.Browser;
        browserUserAgent = ver["User-Agent"];
        console.log(`[browser] Detected: ${ver.Browser} (${ver["User-Agent"]})`);

        const listRes = await fetch(`http://127.0.0.1:${cdpPort}/json/list`);
        const list = await listRes.json();
        target = list.find((t) => t.type === "page") || list[0];
        if (target && target.webSocketDebuggerUrl) {
          break;
        }
      } catch (e) {}
    }

    if (!target) {
      throw new Error("Could not find active Chrome page target");
    }

    ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((res, rej) => {
      ws.onopen = res;
      ws.onerror = rej;
    });

    let msgId = 1;
    const pending = new Map();
    const consoleLogs = [];
    const pageErrors = [];

    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && pending.has(msg.id)) {
        const cb = pending.get(msg.id);
        pending.delete(msg.id);
        cb(msg.result, msg.error);
        return;
      }
      if (msg.method === "Console.messageAdded") {
        consoleLogs.push(msg.params.message);
      } else if (msg.method === "Runtime.exceptionThrown") {
        pageErrors.push(msg.params.exceptionDetails);
      }
    };

    function send(method, params = {}) {
      return new Promise((resolve, reject) => {
        const id = msgId++;
        pending.set(id, (result, err) => {
          if (err) reject(new Error(err.message || JSON.stringify(err)));
          else resolve(result);
        });
        ws.send(JSON.stringify({ id, method, params }));
      });
    }

    await send("Page.enable");
    await send("Runtime.enable");
    await send("Console.enable");

    async function evaluate(expr) {
      const res = await send("Runtime.evaluate", {
        expression: expr,
        returnByValue: true,
        awaitPromise: true,
      });
      if (res.exceptionDetails) {
        throw new Error(res.exceptionDetails.exception?.description || "Evaluation exception");
      }
      return res.result?.value;
    }

    // ============================================================
    // Phase 1: Real Native Browser Verification (Unmocked WebUSB)
    // ============================================================
    console.log("\n--- Phase 1: Native Chrome Page Load & Capabilities ---");
    const pageUrl = `http://127.0.0.1:${httpPort}/index.html`;
    await send("Page.navigate", { url: pageUrl });
    await flush(500);

    const title = await evaluate("document.title");
    assert("Page title matches", title.includes("RefineID WebUSB feasibility probe"));

    const isSecureContext = await evaluate("window.isSecureContext");
    assert("Loopback origin is treated as secure context in Chrome", isSecureContext === true);

    const hasNavigatorUsb = await evaluate(`'usb' in navigator && !!navigator.usb`);
    assert("navigator.usb is natively present in Chrome", hasNavigatorUsb === true);

    const capsText = await evaluate('document.getElementById("caps").textContent');
    assert("Capabilities panel reports verdict: ready", capsText.includes("verdict: ready"));
    assert(
      "Capabilities panel explains secure context with WebUSB",
      capsText.includes("Secure context with navigator.usb present"),
    );

    const capsClass = await evaluate('document.getElementById("caps").className');
    assert('Capabilities box has class "ok"', capsClass === "ok");

    // Controls state check
    const controls = await evaluate(`({
      btnConnect: !document.getElementById('btnConnect').disabled,
      btnDisconnectDisabled: document.getElementById('btnDisconnect').disabled,
      btnClaimDisabled: document.getElementById('btnClaim').disabled,
      selConfigDisabled: document.getElementById('selConfig').disabled,
      selInterfaceDisabled: document.getElementById('selInterface').disabled,
      descText: document.getElementById('desc').textContent,
      logText: document.getElementById('log').textContent,
      reportEmpty: !document.getElementById('report').value.trim()
    })`);

    assert("btnConnect is enabled on load", controls.btnConnect === true);
    assert("btnDisconnect is disabled on load", controls.btnDisconnectDisabled === true);
    assert("btnClaim is disabled on load", controls.btnClaimDisabled === true);
    assert("selConfig is disabled on load", controls.selConfigDisabled === true);
    assert("selInterface is disabled on load", controls.selInterfaceDisabled === true);
    assert(
      'Descriptors panel shows "No device selected."',
      controls.descText.includes("No device selected."),
    );
    assert("Diagnostic report initial JSON generated", controls.reportEmpty === false);
    assert(
      "Zero uncaught script exceptions on page load",
      pageErrors.length === 0,
      JSON.stringify(pageErrors),
    );

    // ============================================================
    // Phase 2: Native WebUSB User Gesture & Chooser Lifecycle
    // ============================================================
    console.log("\n--- Phase 2: Native WebUSB User Gesture & Chooser Lifecycle ---");

    // Test that triggering connect with userGesture invokes native requestDevice
    await send("Runtime.evaluate", {
      expression: `document.getElementById('btnConnect').click()`,
      userGesture: true,
    });
    await flush(100);

    const activeChooserLog = await evaluate(`document.getElementById('log').textContent`);
    assert(
      "Native WebUSB requestDevice successfully invoked with user activation",
      activeChooserLog.includes("connect requested: Waiting for the browser device chooser..."),
    );

    const busyState = await evaluate(`({
      connectDisabled: document.getElementById('btnConnect').disabled,
      disconnectEnabled: !document.getElementById('btnDisconnect').disabled
    })`);
    assert(
      "Connect disabled and Disconnect enabled while awaiting chooser",
      busyState.connectDisabled === true && busyState.disconnectEnabled === true,
    );

    // In headless Chrome, no interactive user can select hardware; user clicks Disconnect to cancel
    await evaluate(`document.getElementById('btnDisconnect').click()`);
    await flush(100);

    const disconnectLog = await evaluate(`document.getElementById('log').textContent`);
    assert(
      "Disconnect cancels pending chooser wait and triggers cleanup",
      disconnectLog.includes("disconnect requested"),
    );

    const afterDisconnectState = await evaluate(`({
      connectEnabled: !document.getElementById('btnConnect').disabled,
      disconnectDisabled: document.getElementById('btnDisconnect').disabled
    })`);
    assert(
      "Connect re-enabled and Disconnect disabled after cancellation",
      afterDisconnectState.connectEnabled === true &&
        afterDisconnectState.disconnectDisabled === true,
    );

    // Test simulated permission dismissal (NotFoundError) in browser engine
    const dismissTest = await evaluate(`(async () => {
      const orig = navigator.usb.requestDevice;
      const err = new Error('No device selected.');
      err.name = 'NotFoundError';
      navigator.usb.requestDevice = () => Promise.reject(err);

      document.getElementById('btnConnect').click();
      await new Promise(r => setTimeout(r, 60));

      const log = document.getElementById('log').textContent;
      const connectEnabled = !document.getElementById('btnConnect').disabled;
      const rep = JSON.parse(document.getElementById('report').value);

      navigator.usb.requestDevice = orig;
      return {
        logDismissed: /permission dismissed.*NotFoundError/.test(log),
        connectEnabled,
        errorsLength: rep.errors.length
      };
    })()`);

    assert(
      "Permission dismissal (NotFoundError) logged as normal outcome",
      dismissTest.logDismissed === true,
    );
    assert("Connect re-enabled after dismissal", dismissTest.connectEnabled === true);
    assert("No errors recorded in report on cancellation", dismissTest.errorsLength === 0);

    // ============================================================
    // Phase 3: Browser-Executed Simulated USB Flows in Real Chrome
    // ============================================================
    console.log("\n--- Phase 3: In-Browser Simulated USB Flows (Real Chrome JS Engine) ---");

    // Test CCID Protected interface behavior in real Chrome
    const ccidSimTest = await evaluate(`(async () => {
      const dev = {
        vendorId: 0x076B, productId: 0x5422, productName: 'Chrome CCID Reader',
        manufacturerName: 'OmniKey', serialNumber: 'SECRET-999',
        opened: false, configuration: { configurationValue: 1 },
        configurations: [{
          configurationValue: 1,
          interfaces: [{
            interfaceNumber: 0,
            alternates: [{
              interfaceClass: 0x0B, interfaceSubclass: 0, interfaceProtocol: 0,
              endpoints: [{ endpointNumber: 1, direction: 'in', type: 'bulk' }]
            }]
          }]
        }],
        open() { this.opened = true; return Promise.resolve(); },
        selectConfiguration() { return Promise.resolve(); },
        claimInterface() {
          const e = new Error('The requested interface implements a protected class.');
          e.name = 'SecurityError';
          return Promise.reject(e);
        },
        releaseInterface() { return Promise.resolve(); },
        close() { this.opened = false; return Promise.resolve(); }
      };

      const orig = navigator.usb.requestDevice;
      navigator.usb.requestDevice = () => Promise.resolve(dev);

      document.getElementById('btnConnect').click();
      await new Promise(r => setTimeout(r, 60));

      const descText = document.getElementById('desc').textContent;
      const repAfterConnect = JSON.parse(document.getElementById('report').value);

      document.getElementById('btnClaim').click();
      await new Promise(r => setTimeout(r, 60));

      const logText = document.getElementById('log').textContent;
      const repAfterClaim = JSON.parse(document.getElementById('report').value);

      document.getElementById('btnDisconnect').click();
      await new Promise(r => setTimeout(r, 60));

      navigator.usb.requestDevice = orig;
      return {
        hasProtectedLabel: descText.includes('PROTECTED: Smart Card'),
        serialNotShown: !descText.includes('SECRET-999') && !JSON.stringify(repAfterConnect).includes('SECRET-999'),
        claimFailedSecurityError: /claimInterface failed.*SecurityError/.test(logText),
        protectedPolicyLogged: logText.includes('protected-class policy'),
        errorRecorded: repAfterClaim.errors.length > 0 && repAfterClaim.errors[0].name === 'SecurityError',
        closedOnDisconnect: dev.opened === false
      };
    })()`);

    assert(
      "[CCID Simulation In-Browser] Descriptors flag PROTECTED: Smart Card",
      ccidSimTest.hasProtectedLabel === true,
    );
    assert(
      "[CCID Simulation In-Browser] Serial number strictly absent from descriptors and report",
      ccidSimTest.serialNotShown === true,
    );
    assert(
      "[CCID Simulation In-Browser] claimInterface fails with SecurityError",
      ccidSimTest.claimFailedSecurityError === true,
    );
    assert(
      "[CCID Simulation In-Browser] WebUSB protected-class policy verdict logged",
      ccidSimTest.protectedPolicyLogged === true,
    );
    assert(
      "[CCID Simulation In-Browser] Device closed cleanly on disconnect",
      ccidSimTest.closedOnDisconnect === true,
    );

    // Test Bug 1 in real Chrome DOM:
    // Multi-config device stub: selConfig change -> rebuilds selInterface in real Chrome DOM
    const bug1Test = await evaluate(`(async () => {
      const dev = {
        vendorId: 0x1234, productId: 0x5678, productName: 'Chrome MultiConfig Sim',
        opened: false, configuration: { configurationValue: 1 },
        configurations: [
          {
            configurationValue: 1,
            interfaces: [{
              interfaceNumber: 0,
              alternates: [{ interfaceClass: 0xFF, endpoints: [] }]
            }]
          },
          {
            configurationValue: 2,
            interfaces: [{
              interfaceNumber: 7,
              alternates: [{ interfaceClass: 0xFF, endpoints: [] }]
            }]
          },
          {
            configurationValue: 3,
            interfaces: []
          }
        ],
        selectedConfigs: [],
        claimed: [],
        open() { this.opened = true; return Promise.resolve(); },
        selectConfiguration(v) {
          this.selectedConfigs.push(v);
          this.configuration = { configurationValue: v };
          return Promise.resolve();
        },
        claimInterface(n) { this.claimed.push(n); return Promise.resolve(); },
        releaseInterface() { return Promise.resolve(); },
        close() { this.opened = false; return Promise.resolve(); }
      };

      const orig = navigator.usb.requestDevice;
      navigator.usb.requestDevice = () => Promise.resolve(dev);

      // Connect
      document.getElementById('btnConnect').click();
      await new Promise(r => setTimeout(r, 60));

      const selConfig = document.getElementById('selConfig');
      const selInterface = document.getElementById('selInterface');
      const btnClaim = document.getElementById('btnClaim');
      const report = document.getElementById('report');

      const initConfig = selConfig.value;
      const initIface = selInterface.value;

      // Select Config 2 (which has interface 7)
      selConfig.value = '2';
      selConfig.dispatchEvent(new Event('change'));
      await new Promise(r => setTimeout(r, 30));

      const config2Iface = selInterface.value;
      const config2IfaceCount = selInterface.options.length;
      const repConfig2 = JSON.parse(report.value);

      // Claim
      btnClaim.click();
      await new Promise(r => setTimeout(r, 60));
      const claimedIface = dev.claimed[0];
      const selectedConfig = dev.selectedConfigs[0];

      // Select Config 3 (no interfaces)
      selConfig.value = '3';
      selConfig.dispatchEvent(new Event('change'));
      await new Promise(r => setTimeout(r, 30));
      const config3IfaceCount = selInterface.options.length;
      const config3ClaimDisabled = btnClaim.disabled;
      const repConfig3 = JSON.parse(report.value);

      document.getElementById('btnDisconnect').click();
      await new Promise(r => setTimeout(r, 60));

      navigator.usb.requestDevice = orig;

      return {
        initConfig, initIface,
        config2Iface, config2IfaceCount,
        repConfig2Selection: repConfig2.selection,
        claimedIface, selectedConfig,
        config3IfaceCount, config3ClaimDisabled,
        repConfig3Selection: repConfig3.selection
      };
    })()`);

    assert(
      "[Bug 1 In-Browser] Initially Config 1 and Interface 0 selected",
      bug1Test.initConfig === "1" && bug1Test.initIface === "0",
    );
    assert(
      "[Bug 1 In-Browser] Switching to Config 2 rebuilds selInterface to Interface 7",
      bug1Test.config2Iface === "7" && bug1Test.config2IfaceCount === 1,
    );
    assert(
      "[Bug 1 In-Browser] Report agrees on { configurationValue: 2, interfaceNumber: 7 }",
      bug1Test.repConfig2Selection.configurationValue === 2 &&
        bug1Test.repConfig2Selection.interfaceNumber === 7,
    );
    assert(
      "[Bug 1 In-Browser] Claim targets interface 7 and selects configuration 2",
      bug1Test.claimedIface === 7 && bug1Test.selectedConfig === 2,
    );
    assert(
      "[Bug 1 In-Browser] Config 3 with no interfaces empties selInterface and disables claim",
      bug1Test.config3IfaceCount === 0 && bug1Test.config3ClaimDisabled === true,
    );
    assert(
      "[Bug 1 In-Browser] Config 3 report selection records null interfaceNumber",
      bug1Test.repConfig3Selection.interfaceNumber === null,
    );

    // Test Bug 2 in real Chrome DOM:
    // Delayed open -> Disconnect -> resolve open -> verify device closed and not leaked
    const bug2Test = await evaluate(`(async () => {
      let resolveOpen;
      const openPromise = new Promise(r => { resolveOpen = r; });

      const dev = {
        vendorId: 0x9999, productId: 0x8888,
        opened: false, configuration: null, configurations: [],
        open() {
          return openPromise.then(() => { this.opened = true; });
        },
        selectConfiguration() { return Promise.resolve(); },
        claimInterface() { return Promise.resolve(); },
        releaseInterface() { return Promise.resolve(); },
        close() { this.opened = false; return Promise.resolve(); }
      };

      const orig = navigator.usb.requestDevice;
      navigator.usb.requestDevice = () => Promise.resolve(dev);

      // Start Connect (open will be delayed)
      document.getElementById('btnConnect').click();
      await new Promise(r => setTimeout(r, 30));
      const openedBeforeDisconnect = dev.opened;

      // Click Disconnect while open is pending
      document.getElementById('btnDisconnect').click();
      await new Promise(r => setTimeout(r, 30));

      // Resolve delayed open()
      resolveOpen();
      await new Promise(r => setTimeout(r, 60));

      const openedAfterResolution = dev.opened;
      const descText = document.getElementById('desc').textContent;
      const rep = JSON.parse(document.getElementById('report').value);

      navigator.usb.requestDevice = orig;

      return {
        openedBeforeDisconnect,
        openedAfterResolution,
        descShowsNoDevice: descText.includes('No device selected.'),
        reportDeviceNull: rep.device === null
      };
    })()`);

    assert(
      "[Bug 2 In-Browser] Device not open while promise is pending",
      bug2Test.openedBeforeDisconnect === false,
    );
    assert(
      "[Bug 2 In-Browser] Device NOT LEAKED: dev.opened is false after late open resolution",
      bug2Test.openedAfterResolution === false,
    );
    assert(
      '[Bug 2 In-Browser] Descriptors show "No device selected." (no stale UI repopulation)',
      bug2Test.descShowsNoDevice === true,
    );
    assert(
      "[Bug 2 In-Browser] Report shows no device connected",
      bug2Test.reportDeviceNull === true,
    );
  } finally {
    // Clean up
    console.log("\n[cleanup] Tearing down Chrome and HTTP server...");
    if (ws) {
      try {
        ws.close();
      } catch (e) {}
    }
    chrome.kill("SIGTERM");
    await new Promise((r) => chrome.on("exit", r));
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    } catch (e) {}

    httpServer.kill("SIGTERM");
    await new Promise((r) => httpServer.on("exit", r));
    console.log("[cleanup] All processes stopped and temporary files removed.");
  }

  console.log("\n====================================================");
  console.log(`Real Browser Smoke Test Results: ${checksPassed} passed, ${checksFailed} failed`);
  console.log("====================================================");
  if (checksFailed > 0) {
    process.exit(1);
  }
}

runBrowserSmoke().catch((err) => {
  console.error("Browser smoke test crashed:", err);
  process.exit(1);
});
