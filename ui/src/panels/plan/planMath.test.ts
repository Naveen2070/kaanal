import { expect, test } from "vite-plus/test";
import { effect, move } from "../../test/planRecord";
import {
  DEFAULT_FORM,
  changeWords,
  clean,
  dollars,
  effectWords,
  errorWords,
  isStale,
  price,
  priceEffect,
  toBody,
  type FormState,
} from "./planMath";

const form = (over: Partial<FormState> = {}): FormState => ({ ...DEFAULT_FORM, ...over });
const problems = (f: FormState) => {
  const r = toBody(f);
  return "problems" in r ? Object.keys(r.problems) : [];
};

test("dollars is exact and never reads 0.00 for a real figure", () => {
  expect(dollars(3500)).toBe("0.0035");
  expect(dollars(926_500)).toBe("0.9265");
  expect(dollars(1_620_000)).toBe("1.62");
  expect(dollars(0)).toBe("0.00");
  expect(dollars(2_000_000)).toBe("2.00");
  expect(price(840_000)).toBe("$0.84/h");
  expect(price(null)).toBe("not priced");
  expect(priceEffect(-400_000)).toBe("-$0.40/h");
  expect(priceEffect(96_000)).toBe("+$0.096/h");
  expect(priceEffect(-3500)).toBe("-$0.0035/h");
  expect(priceEffect(null)).toBe("not priced");
  expect(priceEffect(0)).toBe("no price change");
});

test("a move and a run's figures read as the report writes them", () => {
  expect(changeWords(move("app", { knob: "copies", from: 5, to: 3 }))).toBe("copies 5 to 3");
  expect(changeWords(move("db", { knob: "class", from: "a", to: "b" }))).toBe(
    "instance class a to b",
  );
  expect(changeWords(move("x", { knob: "cores", from: 4, to: 2 }))).toBe("cores 4 to 2");
  expect(changeWords(move("x", { knob: "memory", from: 2 ** 30, to: 512 * 2 ** 20 }))).toBe(
    "memory limit 1 GiB to 512 MiB",
  );
  expect(changeWords(move("x", { knob: "memory", from: 1000, to: 500 }))).toBe(
    "memory limit 1000 bytes to 500 bytes",
  );
  expect(effectWords(effect(300, 0, 251))).toBe("p99 300 ms, errors 0%, capacity est. 251 req/s");
  expect(effectWords(effect(118.4, 0.001, null))).toBe(
    "p99 118.4 ms, errors 0.1%, capacity est. no estimate",
  );
  expect(errorWords(0.00001)).toBe("<0.01%");
  expect(errorWords(0.05)).toBe("5%");
});

test("the defaults send a plain plan", () => {
  expect(toBody(DEFAULT_FORM)).toEqual({
    body: { target: { multiple: 2 }, p99_ms: 1000, error_pct: 1 },
  });
});

test("the form's bounds are the server's", () => {
  expect(problems(form({ target: "0.05" }))).toEqual(["target"]);
  expect(problems(form({ target: "65" }))).toEqual(["target"]);
  expect(problems(form({ target: "0.1" }))).toEqual([]);
  expect(problems(form({ target: "64" }))).toEqual([]);
  expect(problems(form({ targetKind: "req_s", target: "0" }))).toEqual(["target"]);
  expect(problems(form({ p99: "0" }))).toEqual(["p99"]);
  expect(problems(form({ p99: "1e10" }))).toEqual(["p99"]);
  expect(problems(form({ p99: "" }))).toEqual(["p99"]);
  expect(problems(form({ err: "101" }))).toEqual(["err"]);
  expect(problems(form({ err: "0" }))).toEqual([]);
  expect(problems(form({ budget: "0" }))).toEqual(["budget"]);
  expect(problems(form({ budget: "-1" }))).toEqual(["budget"]);
});

test("the cheapest fields count only when the box is ticked", () => {
  // bad values are ignored while it is unticked, and absent from the body
  const off = toBody(form({ time: "0", runCap: "0" }));
  expect(off).toEqual({ body: { target: { multiple: 2 }, p99_ms: 1000, error_pct: 1 } });
  expect(problems(form({ cheapest: true, time: "0" }))).toEqual(["time"]);
  expect(problems(form({ cheapest: true, time: "601" }))).toEqual(["time"]);
  expect(problems(form({ cheapest: true, runCap: "0" }))).toEqual(["runCap"]);
  expect(problems(form({ cheapest: true, runCap: "2001" }))).toEqual(["runCap"]);
  expect(problems(form({ cheapest: true, runCap: "1.5" }))).toEqual(["runCap"]);
  const r = toBody(
    form({ cheapest: true, time: "150.2", runCap: "15", providers: ["gcp", "azure"] }),
  );
  expect(r).toEqual({
    body: {
      target: { multiple: 2 },
      p99_ms: 1000,
      error_pct: 1,
      cheapest: true,
      time_limit_s: 151,
      run_cap: 15,
      providers: ["gcp", "azure"],
    },
  });
});

test("providers none sends none, an empty choice sends nothing, a budget and a keep list are sent", () => {
  const none = toBody(form({ cheapest: true, providers: "none" }));
  expect("body" in none && none.body.providers).toEqual(["none"]);
  const dflt = toBody(form({ cheapest: true }));
  expect("body" in dflt && "providers" in dflt.body).toBe(false);
  const k = toBody(form({ budget: "2.5", keep: ["cache"] }));
  expect("body" in k && k.body).toMatchObject({ budget_per_hour: 2.5, keep: ["cache"] });
});

test("isStale tells a restart from an Apply", () => {
  const view = { generation: 3, boot: "b2" };
  expect(isStale({ generation: 3 }, view, "b2")).toBeNull();
  expect(isStale({ generation: 2 }, view, "b2")).toBe("earlier");
  expect(isStale({ generation: 3 }, view, "b1")).toBe("restarted");
  expect(isStale({ generation: 2 }, view, null)).toBe("earlier");
});

test("clean takes control and bidi characters out of a name", () => {
  expect(clean("db\u{202E}evil\n")).toBe("dbevil");
});
