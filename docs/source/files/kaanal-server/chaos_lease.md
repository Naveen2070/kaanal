# `crates/kaanal-server/src/chaos_lease.rs`

Timed live chaos (spec L1-L6): the lease table behind the optional `ttl` of `PUT /__kaanal/chaos/{node}` and `PUT /__kaanal/chaos`, and the `expires_in_ms` those routes and `GET /__kaanal/chaos` report. Pure functions over plain data; the handlers in [control.md](control.md) call them under the control lock. The table lives in `Control::chaos_leases` and is copied into each published `Effective` ([state.md](state.md)), like the scenario leases.

## Items

| Item                                  | Kind | What it does                                                                                                                                                                  |
| ------------------------------------- | ---- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Unit`                                | enum | `Latency`, `Error` (`error_rate`, `error_kind`, `error_status` as one), `Down`, `Mode`. What a lease covers and expires as a whole. `as_str()` gives the `expires_in_ms` key. |
| `Leases`                              | type | `HashMap<(ChaosKey, Unit), tokio::time::Instant>`: one deadline per key and unit. `tokio` time, so tests can pause the clock.                                                 |
| `units_set(&ChaosLayer)`              | fn   | The units a sparse layer sets: a patch (what a PUT leases or makes permanent) or a stored entry (what `prune` keeps).                                                         |
| `apply(leases, key, units, ttl, now)` | fn   | `Some(ttl)`: each unit's deadline becomes `now + ttl` (a new one replaces the old). `None`: each unit's lease is removed, so the field is permanent.                          |
| `prune(leases, live)`                 | fn   | Drops every lease whose key has no live entry or whose unit that entry no longer sets. Called by `commit` on every write.                                                     |
| `clear_units(layer, unit)`            | fn   | Removes one unit's fields; `Mode` unsets `replace` (merge). The expiry timer uses it.                                                                                         |
| `remaining(leases, key, now)`         | fn   | `{unit: ms}` for one key, rounded up with `control::remaining_ms`; empty when the key has none.                                                                               |
| `leased_fields(leases, live)`         | fn   | How many stored fields the leases cover, for the `save` warning: 1 for `latency` or `down`, each of rate, kind and status an `error` lease's entry holds, 0 for `mode`.       |

## Rules worth knowing

- **One PUT, one key.** A PUT with `ttl` leases every unit it sets; without `ttl` it removes the leases of the units it sets; units it does not mention keep theirs. A plain `{down: true}` after a timed one makes the outage permanent.
- **The `ttl` is checked first** (`control::parse_ttl`, 100 ms to 24 h), before the lock: a bad one is a 400 naming `ttl` and nothing changes.
- **A lease dies with its field.** `prune` runs inside `commit` next to the scenario-lease `retain`, so `DELETE` of an entry, `reset`, `save` with `clear_live` and `down: false` (which unsets `down`) all drop the matching leases without each handler doing it. A failed `commit` restores the table with the rest of `Control`.
- **Save.** `scenarios/save` from live or effective writes leased fields as permanent and answers `"N fields have a ttl; the file has none"`, counting fields with `leased_fields` (`mode` is not counted: a file holds no mode).
- Leases are in memory only, gone on restart. A PUT with a `ttl` wakes the lease task (`lease_wake`).
- **Expiry clears, it does not restore.** The one lease timer (`control::lease_task`, `next_deadline` over scenario and chaos leases) calls `control::expire_chaos`: per key, one `commit` that runs `clear_units` for each due unit and removes an entry left empty, then one `expire` event with `chaos_key`. A failed commit keeps the chaos and drops the leases, so no past-due lease is left to spin on.

## Tests

[../../tests.md](../../tests.md#crateskaanal-serversrcchaos_leasers-unit--the-chaos-lease-table) (unit), the expiry tests in [control.rs](../../tests.md#unit-tests-inside-source-files) (paused clock) and the lease tests in [scenarios_http.rs](../../tests.md#crateskaanal-servertestsscenarios_httprs--scenario-control-api).

**Link chaos (1d).** `Unit::Bandwidth` leases `bandwidth_cap`; `jitter_ms` is leased with `latency_ms` (`Unit::Latency`). `units_set`, `leased_fields` and `clear_units` know both.
