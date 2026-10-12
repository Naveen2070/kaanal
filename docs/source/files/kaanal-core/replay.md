# `crates/kaanal-core/src/replay/` (`mod.rs`, `bucket.rs`, `gate.rs`)

The replay machine both drivers share (B1, rules R2.5 and R2.10). One page covers the whole directory: three small files that read better together.

## What it does

A request's `Plan` is walked against per-node limits: a slot limit ([`Gate`](#gate)) and a rate limit ([`Bucket`](#bucket)). Everything here is pure: time is a `u64` of nanoseconds that the caller supplies, and nothing sleeps, reads a clock or does IO. The simulator drives it with virtual time, live mode with real time. This page covers the two building blocks; the engine that uses them is added later.

## Bucket

`Bucket::new(&RateLimit)` makes a full bucket; `Bucket::take(now) -> Result<(), u64>` takes one token.

- It is the generic cell rate algorithm (GCRA) in integer nanoseconds. State is one `tat` (theoretical arrival time of the next cell), starting at 0.
- `take(now)`: with `window = (burst - 1) * interval`, if `tat > now + window` the bucket is empty and `Err(tat - now - window)` says how many nanoseconds until a token exists. Otherwise `tat = max(tat, now) + interval` and the result is `Ok(())`.
- A refusal changes nothing: refused callers do not push the next token away.
- An idle bucket never holds more than `burst` tokens.
- `tat` is a `u128`. With `now < 2^64`, `interval < 2^64` and `burst < 2^32` it cannot overflow, so `take` needs no checked arithmetic and cannot fail (decision Q5; the spec said `u64`). An `Err` wait larger than `u64::MAX` saturates.

## Gate

`Gate::new(&Admission)` makes a gate with every slot free. Waiters are `(request slot, token index)` pairs (`Waiter`).

- `arrive(w)` returns `Admit::Run` (a slot was free and is now held), `Admit::Queued` (all busy, `w` is in the FIFO) or `Admit::Overflow` (all busy and the queue is at its cap). A cap of `None` never overflows; a cap of 0 overflows as soon as every slot is busy.
- `release()` ends a running hop. The oldest waiter, if any, gets the slot at once and the slot is not freed, so a later arrival at the same instant cannot barge past waiters. With no waiter the slot is freed.
- `cancel(w)` removes one queued waiter (its request failed); the others keep their order. A waiter that is not queued is ignored.
- `busy()`, `queued()`, `slots()` and `overflow_status()` read the state.
