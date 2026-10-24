// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
// Synthetic test doubles for public/index.html (no hardware, no browser).
// Runs the EXACT embedded <script> in a vm sandbox with realistic DOM/USB stubs.
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const htmlPath = path.resolve(__dirname, "../public/index.html");
const html = fs.readFileSync(htmlPath, "utf8");
const scriptMatch = html.match(/<script>([\s\S]*)<\/script>/);
if (!scriptMatch) {
  throw new Error("Could not find <script> in " + htmlPath);
}
const script = scriptMatch[1];

// Helper to create a deferred promise that can be resolved/rejected on demand
function createDeferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// ---- minimal DOM stubs ----
function makeEl(tag, doc) {
  const e = {
    tag,
    children: [],
    listeners: {},
    _text: "",
    _value: "",
    disabled: false,
    className: "",
    selectedIndex: 0,
    selected: false,
    options: tag === "select" ? [] : undefined,
    getAttribute(k) {
      return this["_attr_" + k];
    },
    setAttribute(k, v) {
      this["_attr_" + k] = v;
    },
    removeAttribute(k) {
      delete this["_attr_" + k];
    },
    appendChild(c) {
      this.children.push(c);
      if (this.tag === "select" && c.tag === "option") {
        this.options.push(c);
        if (this.options.length === 1 && !this._value) {
          this._value = c.value;
          this.selectedIndex = 0;
        }
      }
      return c;
    },
    addEventListener(t, f) {
      (this.listeners[t] = this.listeners[t] || []).push(f);
    },
    focus() {},
    select() {},
    click() {
      (this.listeners.click || []).forEach((f) => f({}));
    },
    change() {
      (this.listeners.change || []).forEach((f) => f({}));
    },
  };

  Object.defineProperty(e, "value", {
    get() {
      return this._value;
    },
    set(v) {
      this._value = String(v);
      if (this.options) {
        const idx = this.options.findIndex((o) => o.value === this._value);
        if (idx !== -1) {
          this.selectedIndex = idx;
        }
      }
    },
  });

  Object.defineProperty(e, "textContent", {
    get() {
      if (this.children.length) return this.children.map(visibleText).join("");
      return this._text;
    },
    set(v) {
      this._text = String(v);
      this.children = [];
      if (this.tag === "select") {
        this.options = [];
        this._value = "";
        this.selectedIndex = -1;
      }
    },
  });
  return e;
}

function visibleText(n) {
  if (typeof n === "string") return n;
  if (n && n.nodeText !== undefined) return n.nodeText;
  return n.textContent || "";
}

function textNode(s) {
  return { nodeText: String(s) };
}

function makeDoc() {
  const els = {};
  const ids = [
    "caps",
    "desc",
    "log",
    "report",
    "copyStatus",
    "btnConnect",
    "btnDisconnect",
    "btnClaim",
    "btnCopy",
    "selConfig",
    "selInterface",
  ];
  ids.forEach((id) => {
    const tag = id.startsWith("btn")
      ? "button"
      : id.startsWith("sel")
        ? "select"
        : id === "report"
          ? "textarea"
          : id === "copyStatus"
            ? "span"
            : "pre";
    els[id] = makeEl(tag);
  });
  return {
    els,
    readyState: "complete",
    getElementById(id) {
      return els[id] || null;
    },
    createElement(tag) {
      return makeEl(tag);
    },
    createTextNode(s) {
      return textNode(s);
    },
    addEventListener() {},
    execCommand() {
      return true;
    },
  };
}

// ---- fake USB devices ----
function domErr(name, message) {
  const e = new Error(message);
  e.name = name;
  return e;
}

function ccidDevice() {
  return {
    vendorId: 0x076b,
    productId: 0x5422,
    productName: "Fake CCID",
    manufacturerName: "FakeCo",
    serialNumber: "SECRET-SERIAL-123",
    opened: false,
    configuration: null,
    configurations: [
      {
        configurationValue: 1,
        interfaces: [
          {
            interfaceNumber: 0,
            alternates: [
              {
                interfaceClass: 0x0b,
                interfaceSubclass: 0,
                interfaceProtocol: 0,
                endpoints: [
                  { endpointNumber: 1, direction: "in", type: "bulk" },
                  { endpointNumber: 2, direction: "out", type: "bulk" },
                ],
              },
            ],
          },
        ],
      },
    ],
    open() {
      this.opened = true;
      return Promise.resolve();
    },
    selectConfiguration(v) {
      this.configuration = { configurationValue: v };
      return Promise.resolve();
    },
    claimInterface() {
      return Promise.reject(
        domErr("SecurityError", "The requested interface implements a protected class."),
      );
    },
    releaseInterface() {
      return Promise.resolve();
    },
    close() {
      this.opened = false;
      return Promise.resolve();
    },
  };
}

function vendorDevice() {
  const d = ccidDevice();
  d.productName = "Fake Vendor";
  d.configurations[0].interfaces[0].alternates[0].interfaceClass = 0xff;
  d.claimed = [];
  d.claimInterface = function (n) {
    this.claimed.push(n);
    return Promise.resolve();
  };
  d.releaseInterface = function (n) {
    this.released = (this.released || []).concat(n);
    return Promise.resolve();
  };
  return d;
}

function multiConfigDevice() {
  return {
    vendorId: 0x1234,
    productId: 0x5678,
    productName: "Multi Config Dev",
    manufacturerName: "MultiCo",
    serialNumber: "SECRET-999",
    opened: false,
    configuration: null,
    selectedConfigs: [],
    claimed: [],
    configurations: [
      {
        configurationValue: 1,
        interfaces: [
          {
            interfaceNumber: 0,
            alternates: [
              {
                interfaceClass: 0xff,
                interfaceSubclass: 0,
                interfaceProtocol: 0,
                endpoints: [{ endpointNumber: 1, direction: "in", type: "bulk" }],
              },
            ],
          },
        ],
      },
      {
        configurationValue: 2,
        interfaces: [
          {
            interfaceNumber: 7,
            alternates: [
              {
                interfaceClass: 0xff,
                interfaceSubclass: 0,
                interfaceProtocol: 0,
                endpoints: [{ endpointNumber: 2, direction: "out", type: "bulk" }],
              },
            ],
          },
        ],
      },
      {
        configurationValue: 3,
        interfaces: [], // configuration with no interfaces
      },
      {
        configurationValue: 4,
        interfaces: [
          {
            interfaceNumber: 3,
            alternates: [
              {
                interfaceClass: 0x0b,
                interfaceSubclass: 0,
                interfaceProtocol: 0, // protected CCID
                endpoints: [{ endpointNumber: 3, direction: "in", type: "bulk" }],
              },
            ],
          },
        ],
      },
    ],
    open() {
      this.opened = true;
      return Promise.resolve();
    },
    selectConfiguration(v) {
      this.selectedConfigs.push(v);
      this.configuration = { configurationValue: v };
      return Promise.resolve();
    },
    claimInterface(n) {
      this.claimed.push(n);
      if (n === 3) {
        return Promise.reject(domErr("SecurityError", "Protected Smart Card class"));
      }
      return Promise.resolve();
    },
    releaseInterface(n) {
      this.released = (this.released || []).concat(n);
      return Promise.resolve();
    },
    close() {
      this.opened = false;
      return Promise.resolve();
    },
  };
}

function runPage({ secure, usb }) {
  const doc = makeDoc();
  const sandbox = {
    window: { isSecureContext: secure },
    document: doc,
    navigator: usb ? { usb } : {},
    console,
  };
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(script, sandbox, { filename: "page-script.js" });
  return { doc, usb };
}

const flush = (ms = 30) => new Promise((r) => setTimeout(r, ms));
let pass = 0,
  fail = 0;
function check(name, cond, extra = "") {
  if (cond) {
    pass++;
    console.log("PASS " + name);
  } else {
    fail++;
    console.log("FAIL " + name + " " + extra);
  }
}
const logText = (doc) => doc.els.log.children.map(visibleText).join("\n");

(async () => {
  console.log("--- Running original baseline assertions (1-29) ---");

  // 1. unsupported API + insecure origin
  {
    const { doc } = runPage({ secure: false, usb: null });
    await flush();
    const caps = doc.els.caps.textContent;
    check("insecure+unsupported verdict blocked", caps.includes("verdict: blocked"), caps);
    check("connect disabled when blocked", doc.els.btnConnect.disabled === true);
    check(
      "explanation mentions localhost+Firefox",
      /localhost/i.test(caps) && /Firefox/i.test(caps),
      caps,
    );
  }
  // 2. secure but no USB (Firefox truthful path)
  {
    const { doc } = runPage({ secure: true, usb: null });
    await flush();
    const caps = doc.els.caps.textContent;
    check(
      "firefox-like verdict blocked",
      caps.includes("verdict: blocked") && /navigator\.usb is missing/.test(caps),
      caps,
    );
  }
  // 3. ready state enables connect; no auto-prompt on load
  {
    let prompted = false;
    const usb = {
      requestDevice() {
        prompted = true;
        return Promise.reject(domErr("NotFoundError", "x"));
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    check("ready verdict", doc.els.caps.textContent.includes("verdict: ready"));
    check("connect enabled when ready", doc.els.btnConnect.disabled === false);
    check("no prompt on load", prompted === false);
  }
  // 4. permission cancellation is a normal outcome
  {
    const usb = {
      requestDevice() {
        return Promise.reject(domErr("NotFoundError", "No device selected."));
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    await flush();
    const t = logText(doc);
    check(
      "cancellation logged as dismissed",
      /permission dismissed/.test(t) && /NotFoundError/.test(t),
      t,
    );
    check("not busy after cancel", doc.els.btnConnect.disabled === false);
    check("no errors recorded on cancel", !/failed/.test(t), t);
  }
  // 5. CCID device: descriptors, protected flag, claim -> SecurityError
  {
    const dev = ccidDevice();
    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    await flush(60);
    const desc = doc.els.desc.textContent;
    check("VID/PID shown", /VID 0x076B/.test(desc) && /PID 0x5422/.test(desc), desc);
    check(
      "class 0x0B + endpoints shown",
      /class 0x0B/.test(desc) &&
        /endpoint 1 in bulk/.test(desc) &&
        /endpoint 2 out bulk/.test(desc),
      desc,
    );
    check("protected flagged", /PROTECTED: Smart Card/.test(desc), desc);
    check("serial never shown", !/SECRET-SERIAL/.test(desc + doc.els.report.value + logText(doc)));
    check("grant != access disclaimer", /not proof of usable card access/.test(desc));
    doc.els.btnClaim.click();
    await flush(60);
    const t = logText(doc);
    check(
      "claim stage fails SecurityError",
      /claimInterface failed/.test(t) && /SecurityError/.test(t),
      t,
    );
    check("blocked-access verdict text", /protected-class policy/.test(t), t);
    const rep = JSON.parse(doc.els.report.value);
    check(
      "report has stages+errors, no serial",
      Array.isArray(rep.stages) && rep.errors.length > 0 && !/SECRET/.test(doc.els.report.value),
    );
    check(
      "report records protected alternates",
      rep.device.protectedAlternates.length === 1 &&
        rep.device.protectedAlternates[0].classHex === "0x0B",
    );
  }
  // 6. vendor-specific claim success; disconnect releases + closes
  {
    const dev = vendorDevice();
    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    await flush(60);
    doc.els.btnClaim.click();
    await flush(60);
    let t = logText(doc);
    check(
      "vendor claim success logged",
      /claimInterface succeeded/.test(t) && /not a usable card protocol/.test(t),
      t,
    );
    doc.els.btnDisconnect.click();
    await flush(60);
    check(
      "release called on disconnect",
      (dev.released || []).includes(0),
      JSON.stringify(dev.released),
    );
    check("device closed on disconnect", dev.opened === false);
    t = logText(doc);
    check("cleanup logged", /cleanup/.test(t), t);
  }
  // 7. duplicate connect clicks: second ignored, no second prompt
  {
    let prompts = 0;
    const usb = {
      requestDevice() {
        prompts++;
        return new Promise(() => {});
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    doc.els.btnConnect.click();
    await flush();
    const t = logText(doc);
    check("single prompt on double click", prompts === 1, "prompts=" + prompts);
    check("overlap logged", /already running/.test(t), t);
  }
  // 8. unplug cleans up
  {
    const dev = vendorDevice();
    let discHandler = null;
    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener(t, f) {
        if (t === "disconnect") discHandler = f;
      },
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    await flush(60);
    discHandler({ device: dev });
    await flush(60);
    const t = logText(doc);
    check("unplug logged + cleaned", /unplug/.test(t) && /cleanup/.test(t), t);
    check("device closed on unplug", dev.opened === false);
  }
  // 9. already-claimed device -> InvalidStateError with exact stage
  {
    const dev = vendorDevice();
    dev.claimInterface = () =>
      Promise.reject(domErr("InvalidStateError", "Interface already claimed."));
    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    await flush(60);
    doc.els.btnClaim.click();
    await flush(60);
    const t = logText(doc);
    check(
      "busy claim failure names stage+exception",
      /claimInterface failed/.test(t) && /InvalidStateError/.test(t),
      t,
    );
  }
  // 10. copy fallback path
  {
    const usb = {
      requestDevice() {
        return Promise.reject(domErr("NotFoundError", "x"));
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnCopy.click();
    await flush();
    check(
      "copy status set",
      /Copied|manually/.test(doc.els.copyStatus.textContent),
      doc.els.copyStatus.textContent,
    );
  }

  console.log("\n--- Running Bug 1 regression tests (selConfig interface rebuilding) ---");

  // 11. Bug 1: selConfig change rebuilds selInterface, updates report, and routes claim target
  {
    const dev = multiConfigDevice();
    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    await flush(60);

    // Initial state: config 1 active, interface 0
    check("initially config 1 selected", doc.els.selConfig.value === "1");
    check(
      "initially interface 0 populated",
      doc.els.selInterface.options.length === 1 && doc.els.selInterface.options[0].value === "0",
    );
    check(
      "initial report selection matches",
      JSON.parse(doc.els.report.value).selection.configurationValue === 1 &&
        JSON.parse(doc.els.report.value).selection.interfaceNumber === 0,
    );

    // Switch to Configuration 2 (which has interface 7)
    doc.els.selConfig.value = "2";
    doc.els.selConfig.change();
    await flush(20);

    check(
      "selInterface rebuilt for config 2 (interface 7 present)",
      doc.els.selInterface.options.length === 1 && doc.els.selInterface.options[0].value === "7",
    );
    check(
      "selInterface does NOT retain interface 0",
      !doc.els.selInterface.options.some((o) => o.value === "0"),
    );
    check("selInterface value is 7", doc.els.selInterface.value === "7");
    const repAfterSwitch = JSON.parse(doc.els.report.value);
    check(
      "report selection agrees after config switch",
      repAfterSwitch.selection.configurationValue === 2 &&
        repAfterSwitch.selection.interfaceNumber === 7,
    );

    // Now click claim: must select config 2 and claim interface 7
    doc.els.btnClaim.click();
    await flush(60);
    check("selectConfiguration called with 2", dev.selectedConfigs.includes(2));
    check(
      "claimInterface called with 7 (not 0)",
      dev.claimed.includes(7) && !dev.claimed.includes(0),
    );
    check(
      "claim target agreed across UI, report, and USB call",
      logText(doc).includes("Interface 7 claimed"),
    );
  }

  // 12. Bug 1: Configuration with no interfaces disables claim and selInterface, and clears selection
  {
    const dev = multiConfigDevice();
    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    await flush(60);

    // Switch to Configuration 3 (which has no interfaces)
    doc.els.selConfig.value = "3";
    doc.els.selConfig.change();
    await flush(20);

    check(
      "selInterface has no options for config with no interfaces",
      doc.els.selInterface.options.length === 0,
    );
    check("selInterface.value is empty", doc.els.selInterface.value === "");
    check("selInterface is disabled when no interfaces", doc.els.selInterface.disabled === true);
    check("btnClaim is disabled when no interfaces", doc.els.btnClaim.disabled === true);
    const rep = JSON.parse(doc.els.report.value);
    check(
      "report selection records null interfaceNumber",
      rep.selection.configurationValue === 3 && rep.selection.interfaceNumber === null,
    );

    // Attempting claim programmatically blocks cleanly
    doc.els.btnClaim.click();
    await flush(20);
    check(
      "claim blocked logged when no interface selected",
      /claim blocked.*No interface selected/.test(logText(doc)),
    );
  }

  // 13. Bug 1: Switching to configuration with protected alternate setting flags protection and enforces SecurityError
  {
    const dev = multiConfigDevice();
    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();
    doc.els.btnConnect.click();
    await flush(60);

    // Switch to Configuration 4 (which has interface 3 with class 0x0B Smart Card)
    doc.els.selConfig.value = "4";
    doc.els.selConfig.change();
    await flush(20);

    check(
      "selInterface displays protected label for class 0x0B",
      /PROTECTED/.test(doc.els.selInterface.options[0].textContent),
    );
    check("selInterface value is 3", doc.els.selInterface.value === "3");

    // Claim interface 3 -> rejects with SecurityError
    doc.els.btnClaim.click();
    await flush(60);
    check(
      "claim fails with SecurityError on protected interface",
      /claimInterface failed.*SecurityError/.test(logText(doc)),
    );
    check(
      "protected class verdict logged for config 4 interface",
      /protected-class policy/.test(logText(doc)),
    );
  }

  console.log(
    "\n--- Running Bug 2 regression tests (lifecycle races & non-leaking cancellation) ---",
  );

  // 14. Bug 2: Disconnect while device.open() is pending does NOT leak the open device
  {
    const dev = vendorDevice();
    const openDef = createDeferred();
    dev.open = function () {
      return openDef.promise.then(() => {
        this.opened = true;
      });
    };

    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();

    // Start connection; open() is now pending
    doc.els.btnConnect.click();
    await flush(20);
    check("open in-flight: device not yet opened", dev.opened === false);
    check("open logged", logText(doc).includes("Opening device..."));

    // User clicks Disconnect while open() is in-flight
    doc.els.btnDisconnect.click();
    await flush(20);
    check("disconnect requested while open pending", logText(doc).includes("disconnect requested"));
    check(
      "UI reset: descriptors show no device",
      doc.els.desc.textContent.includes("No device selected."),
    );

    // Now the asynchronous open() finally resolves late
    openDef.resolve();
    await flush(60);

    // CRITICAL: The device must have been closed by cleanup, NOT left open/leaked!
    check("device NOT leaked: dev.opened is false", dev.opened === false);
    check("cleanup was logged for the stale/pending open", logText(doc).includes("cleanup"));
    check(
      "stale descriptors NOT repopulated on late open",
      doc.els.desc.textContent.includes("No device selected."),
    );
    const rep = JSON.parse(doc.els.report.value);
    check("report shows no device connected", rep.device === null);
  }

  // 15. Bug 2: Guard against reconnecting the same device while cleanup is pending
  {
    const dev = vendorDevice();
    const openDef = createDeferred();
    let openCount = 0;
    dev.open = function () {
      openCount++;
      if (openCount === 1) {
        return openDef.promise.then(() => {
          this.opened = true;
        });
      }
      this.opened = true;
      return Promise.resolve();
    };

    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();

    // 1st connect -> open is deferred
    doc.els.btnConnect.click();
    await flush(20);
    check("run 1: open in-flight", openCount === 1 && dev.opened === false);

    // Disconnect clicked while open is pending -> cleanup is pending for dev
    doc.els.btnDisconnect.click();
    await flush(20);

    // 2nd connect immediately requested for the SAME device
    doc.els.btnConnect.click();
    await flush(20);
    check(
      "connect guarded: waited for cleanup of same device",
      logText(doc).includes("Waiting for previous cleanup of this device"),
    );
    check("run 2 open not yet called before run 1 cleanup", openCount === 1);

    // Now resolve Run 1's pending open()
    openDef.resolve();
    await flush(80);

    // Run 1 cleanup completes (closes device), then Run 2 proceeds and opens device fresh!
    check("run 2 open called after cleanup completed", openCount === 2);
    check("device is now cleanly opened and active for run 2", dev.opened === true);
    check("descriptors populated for run 2", doc.els.desc.textContent.includes("VID 0x076B"));
    check("btnDisconnect enabled for run 2", doc.els.btnDisconnect.disabled === false);
  }

  // 16. Bug 2: Late work of slow Device A does NOT close newer connection to Device B or repopulate stale UI
  {
    const devA = vendorDevice();
    devA.productName = "Device A (Slow)";
    const openDefA = createDeferred();
    devA.open = function () {
      return openDefA.promise.then(() => {
        this.opened = true;
      });
    };

    const devB = vendorDevice();
    devB.productName = "Device B (Fast)";
    devB.vendorId = 0xabcd;

    let pickDevice = devA;
    const usb = {
      requestDevice() {
        return Promise.resolve(pickDevice);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();

    // Connect to Device A
    doc.els.btnConnect.click();
    await flush(20);

    // Disconnect Device A
    doc.els.btnDisconnect.click();
    await flush(20);

    // Connect to Device B
    pickDevice = devB;
    doc.els.btnConnect.click();
    await flush(60);
    check("device B connected and opened", devB.opened === true);
    check("UI shows Device B descriptors", doc.els.desc.textContent.includes("Device B (Fast)"));

    // Now Device A's open finally resolves!
    openDefA.resolve();
    await flush(60);

    check("late Device A is closed", devA.opened === false);
    check(
      "newer connection Device B is STILL OPEN (not closed by late work)",
      devB.opened === true,
    );
    check(
      "UI still shows Device B (not overwritten by stale Device A)",
      doc.els.desc.textContent.includes("Device B (Fast)"),
    );
    const rep = JSON.parse(doc.els.report.value);
    check("report still reflects Device B (VID 0xABCD)", rep.device.vendorId === "0xABCD");
  }

  // 17. Bug 2: Disconnect while claimInterface() is pending cleans up and releases
  {
    const dev = vendorDevice();
    const claimDef = createDeferred();
    dev.claimInterface = function (n) {
      return claimDef.promise.then(() => {
        this.claimed.push(n);
      });
    };

    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener() {},
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();

    // Connect
    doc.els.btnConnect.click();
    await flush(60);

    // Start Claim -> pending
    doc.els.btnClaim.click();
    await flush(20);
    check("claimInterface in flight", logText(doc).includes("Claiming interface 0..."));

    // Click Disconnect while claim is in flight
    doc.els.btnDisconnect.click();
    await flush(20);

    // Resolve claim late
    claimDef.resolve();
    await flush(60);

    check("device closed on disconnect after pending claim resolved", dev.opened === false);
    check(
      "stale claim success not logged in current state",
      !logText(doc).includes("Interface 0 claimed. This proves"),
    );
  }

  // 18. Bug 2: Unplug while device.open() is pending cleans up without errors
  {
    const dev = vendorDevice();
    const openDef = createDeferred();
    dev.open = function () {
      return openDef.promise.then(() => {
        this.opened = true;
      });
    };

    let discHandler = null;
    const usb = {
      requestDevice() {
        return Promise.resolve(dev);
      },
      addEventListener(t, f) {
        if (t === "disconnect") discHandler = f;
      },
    };
    const { doc } = runPage({ secure: true, usb });
    await flush();

    // Connect
    doc.els.btnConnect.click();
    await flush(20);

    // Unplug occurs while open is pending
    discHandler({ device: dev });
    await flush(20);
    check("unplug logged during pending open", logText(doc).includes("unplug"));

    // Open resolves late
    openDef.resolve();
    await flush(60);

    check("device closed after unplug and late open", dev.opened === false);
  }

  console.log(`\n========================================`);
  const verdict = fail ? "SOME TESTS FAILED" : "ALL SYNTHETIC & REGRESSION TESTS PASSED";
  console.log(`${pass} passed, ${fail} failed - ${verdict}`);
  console.log(`========================================`);
  process.exit(fail ? 1 : 0);
})();
