// Kaanal Supply: the demo storefront and its chaos controls. Plain JS, no build step, and no
// HTML strings: every node is built with createElement, so product text can never become markup.
"use strict";

// ---- Chaos controls: what they send to the control API (tests/demo.rs reads these lines) ----

const TTL = "30s";
const TTL_MS = 30000;
const PIN_ENDPOINT = "GET /products";
// Stories are the project's scenarios, listed by GET /scenarios: a new scenarios/NAME.yaml file
// shows up here with no change to this script.
const PINS = [
  { key: "pin-empty", label: "Empty", pin: "empty" },
  { key: "pin-error", label: "Error", pin: "error" },
  { key: "pin-loading", label: "Loading", pin: "loading" },
];
const DEBOUNCE_MS = 150; // slider settle time before it is sent
const POLL_OPEN_MS = 1000; // poll while the panel is open
const POLL_ACTIVE_MS = 2500; // poll while it is closed but chaos is on; never when calm
const PING_MS = 2000; // measure the product list for the header pill while chaos is on
// Per-node sliders: PUT /chaos/{node} with { latency_ms, error_rate, ttl }; both at zero is
// DELETE /chaos/{node}. The server ends the lease, so a closed tab cannot leave them on.
const CHAOS_NODES = ["nginx", "shop", "auth", "db", "cache", "payments"];
const LATENCY_MAX = 2000;
const LATENCY_STEP = 50;
// "Reset all chaos" sends a DELETE to each: live chaos, every scenario, every live pin. Not
// POST /reset, which would also empty the cart.
const RESET_ALL = ["/chaos", "/scenarios", "/pins"];

const TOKEN_KEY = "kaanal.token";
const JWT_KEY = "kaanal.jwt";
const HINT_KEY = "kaanal.hint-seen";
const FALLBACK_IMG = "/img/product.svg";
const SVG_NS = "http://www.w3.org/2000/svg";

// ---- Small helpers ----

const $ = (id) => document.getElementById(id);
const money = new Intl.NumberFormat("en-IN", { style: "currency", currency: "INR", maximumFractionDigits: 0 });

function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === false || v == null) continue;
    if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (k === "class") el.className = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  el.append(...kids.flat().filter((k) => k != null && k !== false));
  return el;
}

function price(p) {
  return Number.isFinite(p.price) ? money.format(p.price) : "Price on request";
}

let announceTimer = 0;
function announce(text) {
  const el = $("announce");
  clearTimeout(announceTimer);
  el.textContent = "";
  // A fresh text node makes screen readers repeat an identical message.
  announceTimer = setTimeout(() => (el.textContent = text), 50);
}

/** A short visible note; the toast region is a polite live region, so it is read out too. */
function toast(text, kind) {
  const box = $("toasts");
  while (box.children.length >= 3) box.firstElementChild.remove();
  const t = h("div", { class: `toast ${kind || ""}` }, text);
  box.append(t);
  setTimeout(() => {
    t.classList.add("out");
    setTimeout(() => t.remove(), 300);
  }, 3500);
}

function store(key, value) {
  try {
    if (value == null) sessionStorage.removeItem(key);
    else sessionStorage.setItem(key, value);
  } catch (_) {}
}

function stored(key) {
  try {
    return sessionStorage.getItem(key);
  } catch (_) {
    return null;
  }
}

/** One storefront call. Resolves to { ok, status, body, ms }; a dead connection is status 0. */
async function api(method, path, body) {
  const started = performance.now();
  const opts = { method, headers: {} };
  if (body !== undefined) {
    opts.headers["content-type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  try {
    const res = await fetch(path, opts);
    const text = await res.text();
    let json = null;
    try {
      json = text ? JSON.parse(text) : null;
    } catch (_) {}
    return pill({ ok: res.ok, status: res.status, body: json, retry: res.headers.get("retry-after"), ms: Math.round(performance.now() - started) });
  } catch (_) {
    // A refused connection, or one cut off mid-answer (a reset): both read as status 0.
    return pill({ ok: false, status: 0, body: null, ms: Math.round(performance.now() - started) });
  }
}

/** Show the latest storefront call in the header pill, coloured by health. */
function pill(res) {
  const el = $("api-pill");
  const health = res.status === 0 || res.status === 429 || res.status >= 500 ? "bad" : res.ms >= 300 ? "slow" : "good";
  el.hidden = false;
  el.className = `pill ${health}`;
  el.textContent = res.status === 0 ? "API offline" : health === "bad" ? `API ${res.status}` : `API ${res.ms} ms`;
  el.title = `Last storefront request: ${res.status || "no answer"} in ${res.ms} ms`;
  return res;
}

function errorText(body) {
  return body && typeof body.error === "string" ? body.error : "";
}

/** One friendly sentence about a failed call. */
function describe(res) {
  const s = res.status;
  if (s === 0) return "The connection dropped before an answer arrived, as on a flaky network. Trying again usually works.";
  if (s === 402) return "The payment was declined (402). Nothing was charged and no order was placed.";
  if (s === 429) return `Too many shoppers right now: the shop turned this away (429)${res.retry ? `. Try again in ${res.retry} s` : ""}.`;
  if (s === 502) return "A service behind the shop failed (502).";
  if (s === 503) return "A service is down or overloaded (503).";
  if (s === 504) return "A service behind the shop took too long, so the request gave up (504).";
  const detail = errorText(res.body);
  return `The server answered ${res.status}${detail ? ` (${detail})` : ""}.`;
}

// ---- Product art: one inline drawing per kind, tinted per product ----

const GLYPHS = {
  backpack: {
    hue: 150,
    d: [
      "M34 44c0-11 8-18 18-18h16c10 0 18 7 18 18v48c0 4-3 7-7 7H41c-4 0-7-3-7-7z",
      "M50 26v-5c0-4 3-7 7-7h6c4 0 7 3 7 7v5",
      "M34 56c8 5 17 7 26 7s18-2 26-7",
      "M46 74h28v16H46zM60 63v8",
    ],
  },
  mug: {
    hue: 22,
    d: [
      "M24 40h56v36c0 13-10 22-22 22H46c-12 0-22-9-22-22z",
      "M80 50h7c8 0 12 5 12 12s-4 12-12 12h-7",
      "M42 30c-4-4-4-8 0-12M58 30c-4-4-4-8 0-12",
    ],
  },
  lamp: { hue: 44, d: ["M42 18h36l14 34H28z", "M60 52v40M40 100h40l-4-8H44z", "M46 62l-5 7M74 62l5 7"] },
  blanket: {
    hue: 340,
    d: [
      "M22 70h76c4 0 6 2 6 6v14c0 4-2 6-6 6H22c-4 0-6-2-6-6V76c0-4 2-6 6-6z",
      "M26 48h68c4 0 6 2 6 6v16H20V54c0-4 2-6 6-6z",
      "M30 28h60c4 0 6 2 6 6v14H24V34c0-4 2-6 6-6z",
      "M44 28v20M76 48v22M44 70v26",
    ],
  },
  bottle: {
    hue: 205,
    d: [
      "M48 30h24v8c8 4 12 11 12 20v38c0 5-3 8-8 8H44c-5 0-8-3-8-8V58c0-9 4-16 12-20z",
      "M48 14h24v16H48z",
      "M36 64h48M36 82h48",
    ],
  },
  tote: { hue: 85, d: ["M24 46h72l-7 54H31z", "M42 46V36c0-10 8-18 18-18s18 8 18 18v10", "M40 64h40"] },
};

function glyphFor(p) {
  if (GLYPHS[p.kind]) return GLYPHS[p.kind];
  const name = String(p.name || "").toLowerCase();
  for (const [kind, g] of Object.entries(GLYPHS)) if (name.includes(kind)) return g;
  return null;
}

function glyphSvg(g) {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 120 120");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  g.d.forEach((d, i) => {
    const path = document.createElementNS(SVG_NS, "path");
    path.setAttribute("d", d);
    if (i === 0) path.setAttribute("class", "body");
    svg.append(path);
  });
  return svg;
}

function tile(p, cls) {
  const g = glyphFor(p);
  // Four products share each drawing; a small hue shift per id tells them apart.
  const hue = g ? (g.hue + (Math.abs(Number(p.id) || 0) % 4) * 14) % 360 : (Math.abs(Number(p.id) || 0) * 67) % 360;
  const art = g ? glyphSvg(g) : h("img", { src: FALLBACK_IMG, alt: "", width: "120", height: "120" });
  return h("div", { class: `tile ${cls || ""}`, style: `--h: ${hue}` }, art);
}

// ---- Storefront state ----

let products = null; // the last non-empty /products list; an empty or failed answer is not kept
// A demo login: the mock does not check the JWT, the page only gates the cart on it. Keeping a
// real bearer token in sessionStorage is not a pattern to copy into a production app.
let jwt = stored(JWT_KEY);
let cartCount = 0;
let viewId = 0; // bumps on every render, so a late answer for an old view is dropped
let route = "/";

function setCartCount(n) {
  cartCount = n || 0;
  const el = $("cart-count");
  el.hidden = !cartCount;
  el.textContent = cartCount ? String(cartCount) : "";
}

function setAccount() {
  $("account-link").textContent = jwt ? "Sign out" : "Sign in";
  $("account-link").setAttribute("href", jwt ? "#/logout" : "#/login");
}

/** After a reload or a sign-in, show the real cart count. */
async function syncCart() {
  const res = await api("GET", "/cart");
  if (jwt && res.ok && Array.isArray(res.body)) setCartCount(res.body.reduce((n, i) => n + (Number(i.qty) || 1), 0));
}

// ---- Shared states: skeleton, error, empty ----

function skeletonGrid(n) {
  return h(
    "ul",
    { class: "grid", "aria-hidden": "true" },
    Array.from({ length: n }, () => h("li", { class: "card skeleton" }, h("div", { class: "tile" }), h("div", { class: "card-body" }, h("div", { class: "bar" }), h("div", { class: "bar short" }))))
  );
}

function errorState(title, res, retry) {
  return h(
    "section",
    { class: "state error", role: "alert" },
    h("div", { class: "state-icon", "aria-hidden": "true" }, "!"),
    h("h2", {}, title),
    h("p", {}, describe(res)),
    storyNote(),
    h("button", { type: "button", class: "btn primary", onclick: retry }, "Try again")
  );
}

function emptyState(title, text, action, kind) {
  const art = GLYPHS[kind || "tote"];
  return h("section", { class: "state empty" }, h("div", { class: "state-art", style: `--h: ${art.hue}` }, glyphSvg(art)), h("h2", {}, title), h("p", {}, text), action);
}

function timing(text) {
  return h("p", { class: "timing" }, text);
}

function show(...nodes) {
  $("main").replaceChildren(...nodes);
}

function crumbs(name) {
  return h("nav", { class: "crumbs", "aria-label": "Breadcrumb" }, h("ol", {}, h("li", {}, h("a", { href: "#/" }, "Shop")), h("li", { "aria-current": "page" }, name)));
}

/** A − n + control. `onChange(next)` may be async and return false to refuse the change, or a number to show instead. */
function stepper(label, value, min, max, onChange) {
  let busy = false;
  const out = h("output", { class: "qty", "aria-live": "polite" }, String(value));
  const dec = h("button", { type: "button", "aria-label": `Decrease ${label}` }, "−");
  const inc = h("button", { type: "button", "aria-label": `Increase ${label}` }, "+");
  const el = h("div", { class: "stepper", role: "group", "aria-label": label }, dec, out, inc);
  const set = (v) => {
    value = v;
    out.textContent = String(v);
    // aria-disabled, not disabled: a disabled button drops keyboard focus.
    dec.setAttribute("aria-disabled", String(v <= min));
    inc.setAttribute("aria-disabled", String(v >= max));
  };
  const step = async (d) => {
    const next = value + d;
    if (busy || next < min || next > max) return;
    if (!onChange) return set(next);
    busy = true;
    el.classList.add("busy");
    const ok = await onChange(next);
    busy = false;
    el.classList.remove("busy");
    if (typeof ok === "number") set(ok); // the handler says what the server really holds
    else if (ok !== false) set(next);
  };
  dec.addEventListener("click", () => step(-1));
  inc.addEventListener("click", () => step(1));
  set(value);
  return { el, get: () => value };
}

// ---- Views ----

function setReadout(res) {
  const n = res.ok && Array.isArray(res.body) ? res.body.length : null;
  $("chaos-readout").textContent =
    res.status === 0 ? "Shop: the server could not be reached" : n != null ? `Shop: ${n} products in ${res.ms} ms` : `Shop: answered ${res.status} in ${res.ms} ms`;
}

/** GET /products. Keeps the list only when it is a non-empty array; null if the view moved on. */
async function loadProducts(id) {
  const res = await api("GET", "/products");
  if (res.ok && Array.isArray(res.body) && res.body.length) products = res.body;
  setReadout(res);
  return id === viewId ? res : null;
}

function hero() {
  return h(
    "section",
    { class: "hero" },
    h("div", { class: "hero-copy" }, h("h1", {}, "Everyday goods, made to last"), h("p", {}, "Twenty-four sturdy things for the desk, the kitchen and the walk home.")),
    h("ul", { class: "hero-strip", "aria-hidden": "true" }, Object.values(GLYPHS).map((g) => h("li", { style: `--h: ${g.hue}` }, glyphSvg(g))))
  );
}

async function viewShop(id) {
  const head = hero();
  show(head, skeletonGrid(6));
  const res = await loadProducts(id);
  if (!res) return;
  if (!res.ok || !Array.isArray(res.body)) return show(head, errorState("The shelves did not load", res, render));
  if (res.body.length === 0) {
    return show(head, emptyState("Nothing on the shelves", "The shop has no products right now. Reload in a moment.", h("button", { type: "button", class: "btn", onclick: render }, "Reload"), "tote"));
  }
  const grid = h(
    "ul",
    { class: "grid" },
    res.body.map((p) =>
      h(
        "li",
        { class: "card" },
        h("a", { href: `#/product/${encodeURIComponent(p.id)}` }, tile(p), h("div", { class: "card-body" }, h("h2", {}, String(p.name || "Product")), h("p", { class: "price" }, price(p))))
      )
    )
  );
  show(head, h("div", { class: "shelf-head" }, h("h2", {}, "All goods"), timing(`${res.body.length} products in ${res.ms} ms`)), grid);
  announce(`${res.body.length} products loaded.`);
}

async function viewProduct(id, pid) {
  show(crumbs("Loading"), h("div", { class: "detail skeleton", "aria-hidden": "true" }, h("div", { class: "tile big" }), h("div", {}, h("div", { class: "bar" }), h("div", { class: "bar short" }))));
  let list = products;
  if (!list) {
    const res = await loadProducts(id);
    if (!res) return;
    if (!res.ok || !Array.isArray(res.body)) return show(crumbs("Unavailable"), errorState("This product did not load", res, render));
    list = res.body;
  }
  // The mock answers every id with product 1, so the page uses the call for its timing and
  // status only, and takes the product itself from the list.
  const res = await api("GET", `/products/${encodeURIComponent(pid)}`);
  if (id !== viewId) return;
  if (!res.ok) return show(crumbs("Unavailable"), errorState("This product did not load", res, render));
  const p = list.find((x) => String(x.id) === pid);
  if (!p) return show(crumbs("Not found"), emptyState("No such product", "It may have sold out, or the link is wrong.", h("a", { class: "btn primary", href: "#/" }, "Browse the shop"), "backpack"));
  const name = String(p.name || "Product");
  const qty = stepper("quantity", 1, 1, 9);
  const note = h("p", { class: "hint" });
  let adding = false;
  const add = h("button", { type: "button", class: "btn primary" }, "Add to cart");
  add.addEventListener("click", async () => {
    if (!jwt) {
      location.hash = `#/login?next=${encodeURIComponent(`#/product/${pid}`)}`;
      return;
    }
    if (adding) return;
    adding = true;
    add.setAttribute("aria-busy", "true");
    add.textContent = "Adding...";
    const n = qty.get();
    const r = await api("POST", "/cart", { productId: p.id, qty: n });
    adding = false;
    add.removeAttribute("aria-busy");
    add.textContent = "Add to cart";
    if (r.ok) {
      setCartCount(cartCount + n);
      note.textContent = `Added in ${r.ms} ms.`;
      toast(`Added ${n} × ${name} to your cart.`, "ok");
    } else {
      note.textContent = "";
      toast(`Not added. ${describe(r)}`, "bad");
    }
  });
  show(
    crumbs(name),
    h(
      "article",
      { class: "detail" },
      tile(p, "big"),
      h(
        "div",
        { class: "info" },
        h("h1", {}, name),
        h("p", { class: "price big" }, price(p)),
        h("p", { class: "lede" }, String(p.description || "")),
        h("div", { class: "buy" }, qty.el, add),
        jwt ? null : h("p", { class: "hint" }, "Sign in to add items to your cart."),
        note
      )
    ),
    timing(`Product loaded in ${res.ms} ms`)
  );
}

async function viewCart(id) {
  const head = h("div", { class: "head" }, h("h1", { tabindex: "-1" }, "Your cart"));
  if (!jwt) {
    return show(head, emptyState("Sign in to see your cart", "Your cart is kept for your account.", h("a", { class: "btn primary", href: "#/login?next=%23%2Fcart" }, "Sign in"), "tote"));
  }
  show(head, h("div", { class: "rows skeleton", "aria-hidden": "true" }, [1, 2].map(() => h("div", { class: "bar" }))));
  const [res] = await Promise.all([api("GET", "/cart"), products ? null : loadProducts(id)]);
  if (id !== viewId) return;
  if (!res.ok || !Array.isArray(res.body)) return show(head, errorState("The cart did not load", res, render));
  const empty = () => show(head, emptyState("Your cart is empty", "Pick something from the shop to start.", h("a", { class: "btn primary", href: "#/" }, "Browse the shop"), "tote"));
  if (res.body.length === 0) {
    setCartCount(0);
    return empty();
  }
  // The cart stores one row per add; the page shows one line per product.
  const byId = new Map((products || []).map((p) => [String(p.id), p]));
  const groups = new Map();
  for (const item of res.body) {
    const key = String(item.productId);
    if (!groups.has(key)) groups.set(key, { p: byId.get(key) || { id: item.productId, name: `Product ${item.productId}` }, items: [] });
    groups.get(key).items.push(item);
  }
  const qtyOf = (g) => g.items.reduce((n, i) => n + (Number(i.qty) || 1), 0);
  const total = h("strong", {});
  const recount = () => {
    let sum = 0;
    let n = 0;
    for (const g of groups.values()) {
      n += qtyOf(g);
      if (Number.isFinite(g.p.price)) sum += g.p.price * qtyOf(g);
    }
    total.textContent = money.format(sum);
    setCartCount(n);
  };
  const list = h("ul", { class: "rows" });
  for (const [key, g] of groups) {
    const name = String(g.p.name || "Product");
    const line = h("span", { class: "price" });
    const paintLine = () => (line.textContent = Number.isFinite(g.p.price) ? money.format(g.p.price * qtyOf(g)) : "Price on request");
    // Up adds one; down takes one off the newest row (delete it, re-add the rest).
    const qty = stepper(`quantity of ${name}`, qtyOf(g), 1, 99, async (next) => {
      let r;
      if (next > qtyOf(g)) {
        r = await api("POST", "/cart", { productId: g.p.id, qty: 1 });
        if (r.ok && r.body) g.items.push(r.body);
      } else {
        const last = g.items[g.items.length - 1];
        r = await api("DELETE", `/cart/${encodeURIComponent(last.id)}`);
        if (r.ok) {
          g.items.pop();
          const rest = (Number(last.qty) || 1) - 1;
          if (rest > 0) {
            r = await api("POST", "/cart", { productId: g.p.id, qty: rest });
            if (r.ok && r.body) g.items.push(r.body);
          }
        }
      }
      if (!r.ok) {
        // A decrease is a delete then a re-add, so the delete may have stuck while the re-add did not:
        // show what the rows now say, not what they said before.
        paintLine();
        recount();
        toast(`Quantity not changed. ${describe(r)}`, "bad");
        return qtyOf(g);
      }
      paintLine();
      recount();
      return true;
    });
    let removing = false;
    const remove = h("button", { type: "button", class: "btn quiet", "aria-label": `Remove ${name}` }, "Remove");
    const row = h("li", { class: "row" }, tile(g.p), h("div", { class: "row-name" }, h("strong", {}, name), h("span", {}, price(g.p), " each")), qty.el, line, remove);
    remove.addEventListener("click", async () => {
      if (removing) return;
      removing = true;
      const results = await Promise.all(g.items.map((i) => api("DELETE", `/cart/${encodeURIComponent(i.id)}`)));
      removing = false;
      const failed = results.find((r) => !r.ok);
      g.items = g.items.filter((_, i) => !results[i].ok);
      if (failed) {
        paintLine();
        recount();
        return toast(`Not removed. ${describe(failed)}`, "bad");
      }
      groups.delete(key);
      row.remove();
      recount();
      toast(`Removed ${name}.`, "ok");
      if (!groups.size) empty();
      head.firstElementChild.focus();
    });
    paintLine();
    list.append(row);
  }
  recount();
  show(head, list, h("p", { class: "total" }, h("span", {}, "Total"), total), h("p", { class: "cta" }, h("a", { class: "btn primary", href: "#/checkout" }, "Checkout")), timing(`Cart loaded in ${res.ms} ms`));
}

// ---- Checkout and orders ----

// A generic test number. Only its last four digits are sent; the full number never leaves the page.
const TEST_CARD = "4111 1111 1111 1111";

/** The cart rows as one line per product: [{ p, qty, items }]. */
function cartLines(items) {
  const byId = new Map((products || []).map((p) => [String(p.id), p]));
  const lines = new Map();
  for (const item of items) {
    const key = String(item.productId);
    if (!lines.has(key)) lines.set(key, { p: byId.get(key) || { id: item.productId, name: `Product ${item.productId}` }, qty: 0, items: [] });
    const l = lines.get(key);
    l.qty += Number(item.qty) || 1;
    l.items.push(item);
  }
  return [...lines.values()];
}

const lineTotal = (l) => (Number.isFinite(l.p.price) ? l.p.price * l.qty : 0);

function signInState(head, what, next) {
  return show(head, emptyState(`Sign in to ${what}`, "Your cart and orders are kept for your account.", h("a", { class: "btn primary", href: `#/login?next=${encodeURIComponent(next)}` }, "Sign in"), "tote"));
}

function field(label, id, attrs) {
  return [h("label", { for: id }, label), h("input", { id, required: true, ...attrs })];
}

async function viewCheckout(id) {
  const head = h("div", { class: "head" }, h("h1", { tabindex: "-1" }, "Checkout"));
  if (!jwt) return signInState(head, "check out", "#/checkout");
  show(head, h("div", { class: "rows skeleton", "aria-hidden": "true" }, [1, 2].map(() => h("div", { class: "bar" }))));
  const [res] = await Promise.all([api("GET", "/cart"), products ? null : loadProducts(id)]);
  if (id !== viewId) return;
  if (!res.ok || !Array.isArray(res.body)) return show(head, errorState("The cart did not load", res, render));
  const lines = cartLines(res.body);
  if (!lines.length) return show(head, emptyState("Nothing to pay for", "Your cart is empty.", h("a", { class: "btn primary", href: "#/" }, "Browse the shop"), "tote"));
  const total = lines.reduce((n, l) => n + lineTotal(l), 0);
  const summary = h(
    "ul",
    { class: "rows" },
    lines.map((l) => h("li", { class: "row sum" }, tile(l.p), h("div", { class: "row-name" }, h("strong", {}, String(l.p.name || "Product")), h("span", {}, `${l.qty} × ${price(l.p)}`)), h("span", { class: "price" }, money.format(lineTotal(l)))))
  );
  const status = h("p", { class: "form-status", role: "status" });
  const number = h("input", { id: "card-number", inputmode: "numeric", autocomplete: "cc-number", required: true, value: TEST_CARD });
  const pay = h("button", { type: "submit", class: "btn primary" }, `Pay ${money.format(total)}`);
  const form = h(
    "form",
    { class: "login wide", novalidate: true },
    h("h2", {}, "Pay by card"),
    h("p", { class: "hint" }, "A test card is filled in. Nothing here is real."),
    h("label", { for: "card-name" }, "Name on card"),
    h("input", { id: "card-name", autocomplete: "cc-name", required: true, value: "Demo Shopper" }),
    h("label", { for: "card-number" }, "Card number"),
    number,
    h("div", { class: "fields" }, h("div", {}, ...field("Expiry", "card-exp", { autocomplete: "cc-exp", value: "12 / 30" })), h("div", {}, ...field("CVC", "card-cvc", { inputmode: "numeric", autocomplete: "cc-csc", value: "123" }))),
    pay,
    status
  );
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    if (pay.getAttribute("aria-busy")) return;
    const digits = number.value.replace(/\D/g, "");
    if (digits.length < 12) {
      status.className = "form-status bad";
      status.textContent = "Enter a card number of at least 12 digits.";
      number.focus();
      return;
    }
    pay.setAttribute("aria-busy", "true");
    status.className = "form-status";
    status.textContent = "Contacting the payment service...";
    const order = { lines: lines.map((l) => ({ productId: l.p.id, name: l.p.name, qty: l.qty, price: l.p.price })), total, method: "card", last4: digits.slice(-4) };
    const r = await api("POST", "/checkout", order);
    if (r.ok && r.body && r.body.id != null) {
      // The order is stored. Pay stays busy until we leave, so a second click cannot place a second order.
      status.textContent = "Payment received. Emptying your cart...";
      const gone = await Promise.all(res.body.map((i) => api("DELETE", `/cart/${encodeURIComponent(i.id)}`)));
      const left = res.body.filter((_, i) => !gone[i].ok);
      setCartCount(left.reduce((n, i) => n + (Number(i.qty) || 1), 0));
      if (left.length) toast("Order placed, but some cart items could not be removed. Check your cart before paying again.", "bad");
      else toast("Payment received. Order placed.", "ok");
      // Leave only if the shopper is still here; otherwise the toast is the news.
      if (id === viewId) location.hash = `#/order/${encodeURIComponent(r.body.id)}`;
    } else {
      pay.removeAttribute("aria-busy");
      status.className = "form-status bad";
      status.textContent = r.status === 402 ? describe(r) : `Payment failed. ${describe(r)} No order was placed; you can try again.`;
    }
  });
  show(head, h("div", { class: "checkout" }, h("section", {}, h("h2", {}, "Your order"), summary, h("p", { class: "total" }, h("span", {}, "Total"), h("strong", {}, money.format(total)))), form), timing(`Cart loaded in ${res.ms} ms`));
  number.focus();
}

async function viewOrder(id, oid) {
  show(crumbs("Order"), h("div", { class: "rows skeleton", "aria-hidden": "true" }, [1, 2].map(() => h("div", { class: "bar" }))));
  const [res] = await Promise.all([api("GET", `/orders/${encodeURIComponent(oid)}`), products ? null : loadProducts(id)]);
  if (id !== viewId) return;
  if (res.status === 404) return show(crumbs("Not found"), emptyState("No such order", "It may have been cleared, or the link is wrong.", h("a", { class: "btn primary", href: "#/orders" }, "Your orders"), "backpack"));
  if (!res.ok || !res.body) return show(crumbs("Unavailable"), errorState("The order did not load", res, render));
  const o = res.body;
  const byId = new Map((products || []).map((p) => [String(p.id), p]));
  const lines = Array.isArray(o.lines) ? o.lines : [];
  show(
    crumbs(`Order #${o.id}`),
    h("section", { class: "placed" }, h("h1", { tabindex: "-1" }, "Thank you. Your order is placed."), h("p", { class: "lede" }, `Order #${o.id}, paid by card ending ${o.last4 || "----"}.`)),
    h(
      "ul",
      { class: "rows" },
      lines.map((l) => h("li", { class: "row sum" }, tile(byId.get(String(l.productId)) || { id: l.productId }), h("div", { class: "row-name" }, h("strong", {}, String(l.name || "Product")), h("span", {}, `${l.qty} × ${money.format(l.price)}`)), h("span", { class: "price" }, money.format(l.qty * l.price))))
    ),
    h("p", { class: "total" }, h("span", {}, "Paid"), h("strong", {}, money.format(Number(o.total) || 0))),
    h("p", { class: "cta" }, h("a", { class: "btn", href: "#/orders" }, "Your orders"), " ", h("a", { class: "btn primary", href: "#/" }, "Keep shopping")),
    timing(`Order loaded in ${res.ms} ms`)
  );
}

async function viewOrders(id) {
  const head = h("div", { class: "head" }, h("h1", { tabindex: "-1" }, "Your orders"));
  if (!jwt) return signInState(head, "see your orders", "#/orders");
  show(head, h("div", { class: "rows skeleton", "aria-hidden": "true" }, [1, 2].map(() => h("div", { class: "bar" }))));
  const res = await api("GET", "/orders");
  if (id !== viewId) return;
  if (!res.ok || !Array.isArray(res.body)) return show(head, errorState("Your orders did not load", res, render));
  if (!res.body.length) return show(head, emptyState("No orders yet", "Pay for a cart and the order shows up here.", h("a", { class: "btn primary", href: "#/" }, "Browse the shop"), "tote"));
  const rows = [...res.body].reverse().map((o) => {
    const n = (Array.isArray(o.lines) ? o.lines : []).reduce((s, l) => s + (Number(l.qty) || 1), 0);
    return h("li", { class: "row sum order" }, h("div", { class: "row-name" }, h("a", { href: `#/order/${encodeURIComponent(o.id)}` }, h("strong", {}, `Order #${o.id}`)), h("span", {}, `${n} ${n === 1 ? "item" : "items"}, card ending ${o.last4 || "----"}`)), h("span", { class: "price" }, money.format(Number(o.total) || 0)));
  });
  show(head, h("ul", { class: "rows" }, rows), timing(`${res.body.length} orders in ${res.ms} ms`));
}

function viewLogin(next) {
  // Only an in-page route may follow the sign-in.
  const target = next && next.startsWith("#/") && !next.startsWith("#/logout") ? next : "#/cart";
  const status = h("p", { class: "form-status", role: "status" });
  const user = h("input", { id: "user", name: "username", type: "email", autocomplete: "username", required: true, value: "demo@example.com" });
  const pass = h("input", { id: "pass", name: "password", type: "password", autocomplete: "current-password", required: true });
  const submit = h("button", { type: "submit", class: "btn primary" }, "Sign in");
  const form = h(
    "form",
    { class: "login", novalidate: true },
    h("h1", {}, "Sign in"),
    h("p", { class: "hint" }, "Demo account: demo@example.com, password demo. Try a wrong password to see the 401."),
    h("label", { for: "user" }, "Email"),
    user,
    h("label", { for: "pass" }, "Password"),
    pass,
    submit,
    status
  );
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    if (submit.getAttribute("aria-busy")) return;
    submit.setAttribute("aria-busy", "true");
    status.textContent = "Signing in...";
    status.className = "form-status";
    const r = await api("POST", "/login", { username: user.value, password: pass.value });
    submit.removeAttribute("aria-busy");
    if (r.ok && r.body && typeof r.body.access_token === "string") {
      jwt = r.body.access_token;
      store(JWT_KEY, jwt);
      setAccount();
      syncCart();
      toast("Signed in.", "ok");
      location.hash = target;
    } else if (r.status === 401) {
      status.className = "form-status bad";
      status.textContent = "Wrong email or password. The server answered 401 (invalid_credentials).";
      pass.select();
    } else {
      status.className = "form-status bad";
      status.textContent = `Sign-in failed. ${describe(r)}`;
    }
  });
  show(form);
  pass.focus();
}

function viewNotFound() {
  show(crumbs("Not found"), emptyState("Page not found", "That link does not lead anywhere in the shop.", h("a", { class: "btn primary", href: "#/" }, "Browse the shop"), "lamp"));
}

// ---- Router ----

function render() {
  const id = ++viewId;
  // Only "#/..." is a route; anything else (an in-page anchor, a stray fragment) shows the shop.
  const hash = location.hash.startsWith("#/") ? location.hash : "#/";
  const [path, query] = hash.slice(1).split("?");
  route = path;
  const params = new URLSearchParams(query || "");
  for (const a of document.querySelectorAll("[data-nav]")) {
    const on = (a.dataset.nav === "shop" && (path === "/" || path.startsWith("/product/"))) || path === `/${a.dataset.nav}`;
    if (on) a.setAttribute("aria-current", "page");
    else a.removeAttribute("aria-current");
  }
  const m = path.match(/^\/product\/([^/]+)$/);
  if (m) {
    let pid;
    try {
      pid = decodeURIComponent(m[1]);
    } catch (_) {
      return viewNotFound();
    }
    return viewProduct(id, pid);
  }
  if (path === "/") return viewShop(id);
  if (path === "/cart") return viewCart(id);
  if (path === "/checkout") return viewCheckout(id);
  if (path === "/orders") return viewOrders(id);
  const ord = path.match(/^\/order\/([^/]+)$/);
  if (ord) {
    try {
      return viewOrder(id, decodeURIComponent(ord[1]));
    } catch (_) {
      return viewNotFound();
    }
  }
  if (path === "/login") return viewLogin(params.get("next"));
  if (path === "/logout") {
    jwt = null;
    store(JWT_KEY, null);
    setAccount();
    setCartCount(0);
    toast("Signed out.");
    location.replace("#/");
    return;
  }
  return viewNotFound();
}

let lastPath = (location.hash || "#/").split("?")[0];
window.addEventListener("hashchange", () => {
  if (adoptToken()) {
    setupChaos();
  } else if (location.hash && !location.hash.startsWith("#/")) {
    return; // an in-page anchor, not a route
  }
  const path = (location.hash || "#/").split("?")[0];
  if (path !== lastPath) {
    window.scrollTo(0, 0);
    // Before render, so a view that focuses its own field (sign-in) wins.
    $("main").focus({ preventScroll: true });
  }
  lastPath = path;
  render();
});

$("skip").addEventListener("click", (e) => {
  e.preventDefault();
  $("main").focus();
});

// ---- Chaos controls ----

// The control token rides in the URL fragment, which is never sent to a server. It is kept in
// sessionStorage (this tab only, R5.9) so a reload keeps the controls, taken out of the address
// bar, and only ever sent to this origin's /__kaanal API.
let token = null;

function adoptToken() {
  const t = new URLSearchParams(location.hash.slice(1)).get("token");
  if (!t) return false;
  token = t;
  store(TOKEN_KEY, t);
  history.replaceState(null, "", `${location.pathname}${location.search}#/`);
  return true;
}

if (!adoptToken()) token = stored(TOKEN_KEY);

const stories = []; // one per scenario from GET /scenarios: { key, scenario, label, description, button, left }
const chaos = {}; // story or pin key -> { deadline: ms epoch, or 0 when on with no known end; ours }
const nodes = {}; // node -> { latency, rate (percent), deadline, pending, queue, row parts }
let busy = 0; // control writes in flight; polls are skipped meanwhile
let pollSeq = 0; // bumps on every poll and write, so an older poll answer is dropped
let pollTimer = 0;
let pollEvery = 0;
let pingTimer = 0;
let pinging = false;
let tickTimer = 0;
let built = false;
let calm = true; // the server reports no effective chaos and no pins
let lastSig = null; // the effective chaos and pins the shop was last read under

async function control(method, path, body) {
  const opts = { method, headers: { authorization: `Bearer ${token}` } };
  if (body !== undefined) {
    opts.headers["content-type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  let res;
  try {
    res = await fetch(`/__kaanal${path}`, opts);
  } catch (_) {
    throw new Error("The control API could not be reached. Is the demo still running?");
  }
  const text = await res.text();
  let json = null;
  try {
    json = text ? JSON.parse(text) : null;
  } catch (_) {}
  if (res.status === 401) throw new Error("The control token was refused (401). Open the link the demo printed again, or forget the token.");
  if (!res.ok) {
    const detail = errorText(json);
    throw new Error(`The control API answered ${res.status}${detail ? `: ${detail}` : ""}.`);
  }
  return json;
}

const pinQuery = `/pins?endpoint=${encodeURIComponent(PIN_ENDPOINT)}`;

/** Run a control write; polls wait for it. The poll after the last write re-reads the shop. */
async function write(fn) {
  busy++;
  pollSeq++;
  try {
    await fn();
    setStatus("");
    return true;
  } catch (e) {
    setStatus(e.message);
    return false;
  } finally {
    if (!--busy) poll();
  }
}

function leaseLeft(r) {
  const left = Object.values((r && r.expires_in_ms) || {}).filter(Number.isFinite);
  return left.length ? Math.max(...left) : TTL_MS;
}

/** Re-show the shop if it is on screen; otherwise just measure it for the readout. */
function refreshShop() {
  if (route === "/") render();
  else loadProducts(null);
}

function storyLabel(name) {
  const text = name
    .split(/[-_]/)
    .map((w) => (w === "db" ? "DB" : w))
    .join(" ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function storyNote() {
  const on = stories.filter((s) => chaos[s.key]).map((s) => s.label);
  return on.length ? h("p", { class: "hint" }, `Chaos story on: ${on.join(", ")}. It ends by itself, or turn it off in the chaos controls.`) : null;
}

/** Mirror GET /scenarios; the buttons are rebuilt only when the set of names changes. */
function syncStories(list, now) {
  const rows = Array.isArray(list) ? list.filter((r) => r && typeof r.name === "string") : [];
  if (rows.map((r) => r.name).join() !== stories.map((s) => s.scenario).join()) {
    for (const s of stories) delete chaos[s.key];
    stories.length = 0;
    for (const r of rows) {
      const s = { key: `s:${r.name}`, scenario: r.name, label: storyLabel(r.name), description: String(r.description || ""), left: h("span", { class: "left" }) };
      const badge = r.name
        .split(/[-_]/)
        .filter(Boolean)
        .slice(0, 2)
        .map((w) => w[0].toUpperCase())
        .join("");
      s.button = h(
        "button",
        { type: "button", class: "story", "aria-pressed": "false", onclick: () => press(s) },
        h("span", { class: "badge", "aria-hidden": "true" }, badge),
        h("span", { class: "story-text" }, h("strong", {}, s.label), s.description ? h("small", {}, s.description) : null),
        s.left
      );
      stories.push(s);
    }
    $("chaos-scenarios").replaceChildren(...stories.map((s) => s.button));
  }
  rows.forEach((r, i) => {
    const s = stories[i];
    if (r.active) chaos[s.key] = { deadline: Number.isFinite(r.expires_in_ms) ? now + r.expires_in_ms : 0 };
    else delete chaos[s.key];
  });
}

/** A story or a pin. The button flips at once; a refused write flips it back. */
async function press(a) {
  const before = { ...chaos };
  const on = !!chaos[a.key];
  if (on) delete chaos[a.key];
  else {
    if (a.pin) for (const p of PINS) delete chaos[p.key];
    // Pins have no lease in the control API, so the page lifts its own pin when time is up
    // (and on pagehide).
    chaos[a.key] = { deadline: Date.now() + TTL_MS, ours: !!a.pin };
  }
  paint();
  const ok = await write(async () => {
    if (a.scenario) {
      const path = `/scenarios/${encodeURIComponent(a.scenario)}`;
      if (on) await control("POST", `${path}/deactivate`);
      else {
        const r = await control("POST", `${path}/activate`, { ttl: TTL });
        const left = r && r.expires_in_ms ? r.expires_in_ms[a.scenario] : 0;
        if (chaos[a.key]) chaos[a.key].deadline = Date.now() + (left || TTL_MS);
      }
    } else if (on) await control("DELETE", pinQuery);
    else await control("PUT", "/pins", { endpoint: PIN_ENDPOINT, state: a.pin });
  });
  if (ok) announce(`${a.label} ${on ? "off" : "on for 30 seconds"}.`);
  else {
    for (const k of Object.keys(chaos)) delete chaos[k];
    Object.assign(chaos, before);
  }
  paint();
}

/** Send one node's sliders. Calls for a node run one after another, so they land in order. */
function queueNode(name) {
  const n = nodes[name];
  n.queue = n.queue.then(async () => {
    n.pending = false;
    await write(async () => {
      if (!n.latency && !n.rate) {
        await control("DELETE", `/chaos/${name}`);
        n.deadline = 0;
      } else {
        const r = await control("PUT", `/chaos/${name}`, { latency_ms: n.latency, error_rate: n.rate / 100, ttl: TTL });
        n.deadline = Date.now() + leaseLeft(r);
      }
    });
    paint();
  });
}

/** Read what is active (from the page, the CLI or another tab). A change re-reads the shop. */
async function poll() {
  if (busy || !token) return;
  const seq = ++pollSeq;
  try {
    const [list, pins, live] = await Promise.all([control("GET", "/scenarios"), control("GET", "/pins"), control("GET", "/chaos")]);
    if (seq !== pollSeq || busy) return;
    const now = Date.now();
    syncStories(list, now);
    const pin = (pins.live || []).find((p) => p.endpoint === PIN_ENDPOINT);
    for (const a of PINS) {
      // Keep our own countdown; a pin set elsewhere shows as "on".
      if (pin && pin.state === a.pin) chaos[a.key] = chaos[a.key] || { deadline: 0 };
      else delete chaos[a.key];
    }
    for (const name of CHAOS_NODES) {
      const n = nodes[name];
      if (n.pending) continue; // the slider is being moved
      const e = (live.nodes || {})[name];
      n.latency = e ? Number(e.latency_ms) || 0 : 0;
      n.rate = e ? Math.round((Number(e.error_rate) || 0) * 100) : 0;
      n.deadline = e && e.expires_in_ms ? now + leaseLeft(e) : 0;
    }
    const eff = live.effective || {};
    const pinned = pins.effective || {};
    calm = !Object.keys(eff.nodes || {}).length && !Object.keys(eff.endpoints || {}).length && !(eff.node_at || []).length && !Object.keys(pinned).length;
    const sig = JSON.stringify([eff, pinned]);
    if (lastSig !== null && sig !== lastSig) refreshShop();
    lastSig = sig;
    setStatus("");
  } catch (e) {
    if (seq === pollSeq) setStatus(e.message);
  }
  paint();
}

/** Measure the product list for the header pill; one at a time, however slow it gets. */
async function pingShop() {
  if (pinging) return;
  pinging = true;
  await api("GET", "/products");
  pinging = false;
}

function setStatus(text) {
  $("chaos-status").textContent = text;
}

function tick() {
  const now = Date.now();
  let ended = false;
  for (const a of [...stories, ...PINS]) {
    const c = chaos[a.key];
    if (!c || !c.deadline || c.deadline > now) continue;
    delete chaos[a.key];
    ended = true;
    if (c.ours) write(() => control("DELETE", pinQuery)); // our own pin is lifted here
  }
  for (const name of CHAOS_NODES) {
    const n = nodes[name];
    // The server has already cleared the lease; mirror it.
    if (!n.pending && n.deadline && n.deadline <= now) {
      n.latency = n.rate = n.deadline = 0;
      ended = true;
    }
  }
  paint();
  if (ended) {
    announce("Chaos ended; the shop is back to normal.");
    poll();
  }
}

function secondsLeft(deadline) {
  return Math.max(0, Math.ceil((deadline - Date.now()) / 1000));
}

function paint() {
  if (!built) return;
  let count = 0;
  for (const a of [...stories, ...PINS]) {
    const c = chaos[a.key];
    a.button.setAttribute("aria-pressed", c ? "true" : "false");
    a.left.textContent = c ? (c.deadline ? `${secondsLeft(c.deadline)} s` : "on") : "";
    if (c) count++;
  }
  for (const name of CHAOS_NODES) {
    const n = nodes[name];
    const on = n.latency > 0 || n.rate > 0;
    if (!n.pending) {
      n.lat.value = String(n.latency);
      n.err.value = String(n.rate);
    }
    n.latOut.textContent = `${n.lat.value} ms`;
    n.errOut.textContent = `${n.err.value}%`;
    n.left.textContent = on && n.deadline ? `${secondsLeft(n.deadline)} s` : "";
    n.row.classList.toggle("on", on);
    if (on) count++;
  }
  $("chaos-badge").hidden = !count;
  $("chaos-badge").textContent = String(count);
  $("chaos-fab").setAttribute("aria-label", count ? `Chaos controls, ${count} active` : "Chaos controls");
  $("chaos").classList.toggle("live", count > 0);
  // The strip under the header says which stories are playing.
  const playing = stories.filter((s) => chaos[s.key]);
  const strip = $("incident");
  const text = playing.map((s) => `${s.label}: ${s.description}`).join("\n");
  if (strip.dataset.text !== text) {
    strip.dataset.text = text;
    strip.hidden = !playing.length;
    strip.replaceChildren(...playing.map((s) => h("p", {}, h("strong", {}, s.label), s.description ? ` ${s.description}` : "")));
  }
  // Tick while anything runs. Poll fast while the panel is open, slower while chaos is on,
  // never when calm or hidden; ping the shop for the pill only while chaos is on.
  const open = !$("chaos-panel").hidden;
  const active = count > 0 || !calm;
  const shown = !document.hidden;
  if (count && !tickTimer) tickTimer = setInterval(tick, 1000);
  if (!count && tickTimer) tickTimer = clearInterval(tickTimer) || 0;
  const every = !token || !shown ? 0 : open ? POLL_OPEN_MS : active ? POLL_ACTIVE_MS : 0;
  if (every !== pollEvery) {
    clearInterval(pollTimer);
    pollTimer = every ? setInterval(poll, every) : 0;
    pollEvery = every;
  }
  const ping = !!token && active && shown;
  if (ping && !pingTimer) pingTimer = setInterval(pingShop, PING_MS);
  if (!ping && pingTimer) pingTimer = clearInterval(pingTimer) || 0;
}

function openPanel() {
  $("chaos-panel").hidden = false;
  $("chaos-fab").setAttribute("aria-expanded", "true");
  $("chaos-hint").hidden = true;
  store(HINT_KEY, "1");
  $("chaos-close").focus();
  poll();
}

function closePanel(refocus) {
  if ($("chaos-panel").hidden) return;
  $("chaos-panel").hidden = true;
  $("chaos-fab").setAttribute("aria-expanded", "false");
  pollSeq++; // drop a poll in flight
  paint();
  if (refocus) $("chaos-fab").focus();
}

function nodeRow(name) {
  const slider = (kind, max, step, unit) => {
    const id = `chaos-${name}-${kind}`;
    const input = h("input", { type: "range", id, min: "0", max: String(max), step: String(step), value: "0" });
    const out = h("output", { for: id });
    return { input, out, el: h("div", { class: "slider" }, h("label", { for: id }, unit), input, out) };
  };
  const lat = slider("latency", LATENCY_MAX, LATENCY_STEP, "Delay");
  const err = slider("errors", 100, 5, "Errors");
  const left = h("span", { class: "left" });
  const row = h("div", { class: "node", role: "group", "aria-label": `${name} node` }, h("div", { class: "node-head" }, h("strong", {}, name), left), lat.el, err.el);
  const n = { latency: 0, rate: 0, deadline: 0, pending: false, queue: Promise.resolve(), timer: 0, row, left, lat: lat.input, err: err.input, latOut: lat.out, errOut: err.out };
  const changed = () => {
    n.pending = true;
    n.latency = Number(lat.input.value);
    n.rate = Number(err.input.value);
    paint();
    clearTimeout(n.timer);
    n.timer = setTimeout(() => queueNode(name), DEBOUNCE_MS);
  };
  lat.input.addEventListener("input", changed);
  err.input.addEventListener("input", changed);
  nodes[name] = n;
  return row;
}

async function resetAll() {
  await write(async () => {
    for (const path of RESET_ALL) await control("DELETE", path);
    for (const k of Object.keys(chaos)) delete chaos[k];
    for (const name of CHAOS_NODES) Object.assign(nodes[name], { latency: 0, rate: 0, deadline: 0, pending: false });
    announce("All chaos cleared.");
  });
  paint();
}

async function forgetToken() {
  if (PINS.some((a) => chaos[a.key] && chaos[a.key].ours)) await write(() => control("DELETE", pinQuery));
  token = null;
  store(TOKEN_KEY, null);
  closePanel(false);
  for (const k of Object.keys(chaos)) delete chaos[k];
  for (const name of CHAOS_NODES) Object.assign(nodes[name], { latency: 0, rate: 0, deadline: 0 });
  calm = true;
  lastSig = null;
  paint();
  $("chaos").hidden = true;
  $("main").focus();
  toast("Token forgotten. Open the demo link again to bring the chaos controls back.");
}

function setupChaos() {
  if (!token) return; // no token, no controls; the shop still works
  $("chaos").hidden = false;
  if (built) return poll();
  built = true;
  for (const a of PINS) {
    a.left = h("span", { class: "left" });
    a.button = h("button", { type: "button", class: "chip", "aria-pressed": "false", onclick: () => press(a) }, h("span", {}, a.label), a.left);
    $("chaos-pins").append(a.button);
  }
  $("chaos-nodes").append(...CHAOS_NODES.map(nodeRow));
  $("chaos-fab").addEventListener("click", () => ($("chaos-panel").hidden ? openPanel() : closePanel(true)));
  $("chaos-close").addEventListener("click", () => closePanel(true));
  $("chaos-reset").addEventListener("click", resetAll);
  $("chaos-forget").addEventListener("click", forgetToken);
  $("chaos-panel").addEventListener("keydown", (e) => {
    if (e.key === "Escape") closePanel(true);
  });
  if (!stored(HINT_KEY)) {
    $("chaos-hint").hidden = false;
    setTimeout(() => ($("chaos-hint").hidden = true), 12000);
  }
  // Pins have no lease: lift the one this tab set when the tab goes away.
  window.addEventListener("pagehide", () => {
    if (!token || !PINS.some((a) => chaos[a.key] && chaos[a.key].ours)) return;
    fetch(`/__kaanal${pinQuery}`, { method: "DELETE", keepalive: true, headers: { authorization: `Bearer ${token}` } }).catch(() => {});
  });
  window.addEventListener("pageshow", (e) => {
    if (e.persisted) poll();
  });
  // A hidden tab stops polling and pinging; coming back catches up at once.
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) poll();
    paint();
  });
  paint();
  poll(); // once at load: builds the stories and shows chaos set elsewhere
}

// ---- Start ----

setAccount();
if (jwt) syncCart();
setupChaos();
render();
