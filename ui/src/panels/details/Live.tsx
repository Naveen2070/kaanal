// The Live tab: what is switched on in the running system, apart from the file. Scenarios are named
// sets of chaos and pins turned on in a stack; pins force one endpoint to answer in a state
// (loading, empty, error, slow, success), and a cycle steps that state. Nothing here is saved to
// kaanal.yaml. A scenario changes what the next run sees, so turning one on or off starts a run.

import { useCallback, useEffect, useId, useState } from "react";
import { ApiError, api } from "../../data/api/client";
import { PIN_STATES, type PinState, type PinsView, type ScenarioRow } from "../../data/api/types";
import { runNow } from "../../data/store/loopRuntime";
import { notify } from "../../shell/toast/toasts";
import { Section, UnitInput } from "./Field";
import { Sample } from "./Sample";
import { TIME_UNITS } from "./units";
import {
  describePin,
  leaseLeft,
  LEASES,
  pinBody,
  SOURCE,
  statusRange,
  takesDelay,
} from "./liveModel";

interface Loaded {
  scenarios: ScenarioRow[];
  pins: PinsView;
  endpoints: string[];
}

/** How often the tab looks again: a lease can end without a click. */
const POLL_MS = 3000;

function useLive() {
  /** undefined: loading; null: no system applied yet. */
  const [data, setData] = useState<Loaded | null | undefined>(undefined);
  /** Bumped to look again at once (after an action); the timer looks again on its own. */
  const [again, setAgain] = useState(0);
  const load = useCallback(() => setAgain((n) => n + 1), []);
  useEffect(() => {
    let live = true;
    const look = () =>
      Promise.all([api.listScenarios(), api.getPins(), api.getTopology()])
        .then(([scenarios, pins, topology]): Loaded => ({
          scenarios,
          pins,
          endpoints: topology.endpoints.map((e) => e.key),
        }))
        // 409 before the first Apply, or the server went away: nothing live to show
        .catch(() => null)
        .then((d) => live && setData(d));
    void look();
    const t = setInterval(() => void look(), POLL_MS);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [again]);
  return { data, load };
}

const say = (e: unknown) => (e instanceof ApiError ? e.message : String(e));

export function Live() {
  const { data, load } = useLive();
  const [lease, setLease] = useState(0);
  const act = async (f: () => Promise<unknown>, rerun: boolean) => {
    try {
      await f();
      notify.clear("live");
      if (rerun) runNow();
    } catch (e) {
      notify.error(say(e), "live");
    }
    load();
  };

  if (data === undefined) return <p className="p-3 text-sm text-muted">Loading...</p>;
  if (data === null) {
    return (
      <p className="p-3 text-sm text-muted">
        Scenarios and pins act on the running system, and there is none yet. Apply the draft first.
      </p>
    );
  }
  const anyOn = data.scenarios.some((s) => s.active);
  const btn =
    "min-h-7 rounded border border-control bg-sheet px-2 text-xs hover:bg-raised focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40";
  return (
    <div className="p-3 text-sm">
      <Section title="Scenarios">
        <p className="mb-2 text-xs text-muted">
          A scenario is a named set of failures and pins. Turn several on and they stack, the last
          one on top. This is live: it is not written to the file.
        </p>
        {data.scenarios.length === 0 ? (
          <p className="text-xs text-muted">
            None are defined. Put them in kaanal.yaml or in a scenarios/ folder beside it.
          </p>
        ) : (
          <>
            <div className="mb-2 flex items-center gap-1 text-xs">
              <label htmlFor="live-lease">Turn on</label>
              <select
                id="live-lease"
                value={lease}
                onChange={(e) => setLease(Number(e.currentTarget.value))}
                className="h-7 rounded border border-control bg-sheet px-1 text-text"
              >
                {LEASES.map((l, i) => (
                  <option key={l.label} value={i}>
                    {l.label}
                  </option>
                ))}
              </select>
              {anyOn ? (
                <button
                  type="button"
                  className={`${btn} ml-auto`}
                  onClick={() => void act(() => api.clearScenarios(), true)}
                >
                  Turn all off
                </button>
              ) : null}
            </div>
            <ul className="m-0 list-none p-0">
              {data.scenarios.map((s) => (
                <li key={s.name} className="mb-2 flex items-start gap-2">
                  <div className="min-w-0 flex-1">
                    <p className="font-medium">{s.name}</p>
                    <p className="text-xs text-muted">
                      {s.description ? `${s.description}. ` : ""}
                      {s.entries} {s.entries === 1 ? "entry" : "entries"}, {SOURCE[s.source]}
                      {s.active && s.expires_in_ms !== undefined
                        ? `, ${leaseLeft(s.expires_in_ms)}`
                        : ""}
                    </p>
                  </div>
                  <button
                    type="button"
                    aria-pressed={s.active}
                    aria-label={`${s.active ? "Turn off" : "Turn on"} ${s.name}`}
                    className={`${btn} ${s.active ? "border-accent text-accent" : ""}`}
                    onClick={() =>
                      void act(
                        () =>
                          s.active
                            ? api.deactivateScenario(s.name)
                            : api.activateScenario(s.name, LEASES[lease]?.ttl),
                        true,
                      )
                    }
                  >
                    {s.active ? "On" : "Off"}
                  </button>
                </li>
              ))}
            </ul>
          </>
        )}
      </Section>

      <Section title="Pins">
        <p className="mb-2 text-xs text-muted">
          A pin makes one endpoint answer in a state you pick, so a screen can be seen loading,
          empty or failing. Live pins sit above the pins of any scenario.
        </p>
        <PinList pins={data.pins} btn={btn} act={act} />
        <PinForm endpoints={data.endpoints} btn={btn} act={act} />
      </Section>

      <Section title="Sample a response">
        <Sample endpoints={data.endpoints} />
      </Section>
    </div>
  );
}

type Act = (f: () => Promise<unknown>, rerun: boolean) => Promise<void>;

function PinList({ pins, btn, act }: { pins: PinsView; btn: string; act: Act }) {
  const rows = Object.entries(pins.effective);
  if (rows.length === 0) return <p className="mb-2 text-xs text-muted">No endpoint is pinned.</p>;
  return (
    <ul className="m-0 mb-2 list-none p-0">
      {rows.map(([endpoint, p]) => {
        const live = p.source === "live";
        return (
          <li key={endpoint} className="mb-1.5 flex items-start gap-2">
            <div className="min-w-0 flex-1">
              <p className="wrap-break-word font-medium">{endpoint}</p>
              <p className="text-xs text-muted">
                {describePin(p)}
                {typeof p.source === "object" ? `, from the scenario ${p.source.scenario}` : ""}
              </p>
            </div>
            {live ? (
              <>
                <button
                  type="button"
                  className={btn}
                  aria-label={`Next state for ${endpoint}`}
                  onClick={() => void act(() => api.cyclePin({ endpoint }), false)}
                >
                  Next
                </button>
                <button
                  type="button"
                  className={btn}
                  aria-label={`Remove the pin on ${endpoint}`}
                  onClick={() => void act(() => api.deletePins(endpoint), false)}
                >
                  Remove
                </button>
              </>
            ) : null}
          </li>
        );
      })}
      {pins.live.length > 1 ? (
        <li>
          <button
            type="button"
            className={btn}
            onClick={() => void act(() => api.deletePins(), false)}
          >
            Remove all live pins
          </button>
        </li>
      ) : null}
    </ul>
  );
}

function PinForm({ endpoints, btn, act }: { endpoints: string[]; btn: string; act: Act }) {
  const id = useId();
  const [endpoint, setEndpoint] = useState("");
  const [state, setState] = useState<PinState>("error");
  const [status, setStatus] = useState("");
  const [delay, setDelay] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const range = statusRange(state);
  const field = "h-7 rounded border border-control bg-sheet px-1 text-text";
  return (
    <form
      noValidate
      className="flex flex-col gap-1.5 text-xs"
      onSubmit={(e) => {
        e.preventDefault();
        const r = pinBody(endpoint, state, status, delay);
        if ("problem" in r) return setProblem(r.problem);
        setProblem(null);
        void act(() => api.putPin(r.body), false);
      }}
    >
      <div className="flex flex-wrap items-center gap-1">
        <label htmlFor={`${id}-ep`}>Pin</label>
        <select
          id={`${id}-ep`}
          value={endpoint}
          onChange={(e) => setEndpoint(e.currentTarget.value)}
          className={`${field} min-w-0 max-w-full flex-1`}
        >
          <option value="">an endpoint...</option>
          {endpoints.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
      </div>
      <div className="flex flex-wrap items-center gap-1">
        <label htmlFor={`${id}-st`}>as</label>
        <select
          id={`${id}-st`}
          value={state}
          onChange={(e) => setState(e.currentTarget.value as PinState)}
          className={field}
        >
          {PIN_STATES.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        {range ? (
          <>
            <label htmlFor={`${id}-sc`}>status</label>
            <input
              id={`${id}-sc`}
              type="number"
              min={range.min}
              max={range.max}
              placeholder={state === "error" ? "default" : "200"}
              value={status}
              onChange={(e) => setStatus(e.currentTarget.value)}
              className={`${field} w-20`}
            />
          </>
        ) : null}
        {takesDelay(state) ? (
          <>
            <label htmlFor={`${id}-dl`}>delay</label>
            <UnitInput
              id={`${id}-dl`}
              unitLabel="delay unit"
              units={TIME_UNITS}
              min={0}
              placeholder="none"
              value={delay}
              onChange={setDelay}
              className={field}
            />
          </>
        ) : null}
      </div>
      <div className="flex items-center gap-2">
        <button type="submit" className={btn}>
          Pin it
        </button>
        <button
          type="button"
          className={btn}
          disabled={endpoint === ""}
          onClick={() => void act(() => api.cyclePin({ endpoint }), false)}
        >
          Step to the next state
        </button>
      </div>
      {problem ? (
        <p role="alert" className="text-negative">
          {problem}
        </p>
      ) : null}
    </form>
  );
}
