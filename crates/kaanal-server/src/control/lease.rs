//! Expiring leases: the due-time sweep and the background task that wakes for the next deadline.

use super::{
    AppState, Arc, BusEvent, ChaosKey, ChaosLayer, Control, Instant, Map, MutexGuard, Value,
    chaos_lease, commit, emit, key_fields, key_label, lock,
};

/// Deactivates every scenario whose lease deadline has passed, one `expire` event each, earliest
/// first; then clears the due live chaos units (see [`expire_chaos`]). Returns how many expiries
/// it published: one per scenario, one per chaos key. Every deadline is compared under the lock,
/// so a lease replaced between the task's wake-up and here is not expired early.
///
/// No past-due lease is left behind, whatever happens (the task would spin on it).
pub fn expire_due(st: &AppState) -> usize {
    let mut c = lock(st);
    let now = Instant::now();
    let mut due: Vec<(Instant, String)> = c
        .leases
        .iter()
        .filter(|(_, d)| **d <= now)
        .map(|(n, d)| (*d, n.clone()))
        .collect();
    due.sort();
    let mut n = 0;
    for (_, name) in due {
        // `commit` drops the lease with the activation.
        let done = commit(st, &mut c, |c| {
            c.stack.deactivate(&name);
            Ok(())
        });
        if let Ok(snap) = done {
            emit(st, "expire", Some(&name), &snap);
            n += 1;
        } else {
            // The restore put the past-due lease back; left there, `lease_task` would spin.
            // Drop only the lease: the scenario stays active, so Control and the published
            // state still agree, and a manual deactivate works through `commit`.
            eprintln!("kaanal: could not expire '{name}'; left it active and dropped its lease");
            c.leases.remove(&name);
        }
    }
    n + expire_chaos(st, &mut c)
}

/// **Private.** The chaos half of [`expire_due`] (spec L3): the due units are grouped by key, and
/// each key gets one `commit` that clears them from its live entry (an entry left empty is
/// removed), so one publish and one `expire` event (`chaos_key` set, no `name`) per key. A failed
/// commit leaves the chaos in place and drops the leases. A lease with no unit behind it (only
/// possible if a write skipped `commit`) is dropped without an event.
pub(super) fn expire_chaos(st: &AppState, c: &mut MutexGuard<'_, Control>) -> usize {
    let now = Instant::now();
    let mut due: Vec<(Instant, ChaosKey, chaos_lease::Unit)> = c
        .chaos_leases
        .iter()
        .filter(|(_, d)| **d <= now)
        .map(|((k, u), d)| (*d, k.clone(), *u))
        .collect();
    due.sort_by_key(|(d, _, u)| (*d, *u));
    let mut keys: Vec<(ChaosKey, Vec<chaos_lease::Unit>)> = Vec::new();
    for (_, k, u) in due {
        match keys.iter_mut().find(|(have, _)| *have == k) {
            Some((_, us)) => us.push(u),
            None => keys.push((k, vec![u])),
        }
    }
    let mut n = 0;
    for (key, units) in keys {
        let set = c
            .stack
            .live()
            .layer(&key)
            .map(chaos_lease::units_set)
            .unwrap_or_default();
        if !units.iter().any(|u| set.contains(u)) {
            for u in &units {
                c.chaos_leases.remove(&(key.clone(), *u));
            }
            continue;
        }
        // `commit` drops the leases with their fields.
        let done = commit(st, c, |c| {
            let mut l = c.stack.live().layer(&key).cloned().unwrap_or_default();
            for u in &units {
                chaos_lease::clear_units(&mut l, *u);
                c.chaos_leases.remove(&(key.clone(), *u));
            }
            if l == ChaosLayer::default() {
                c.stack.live_mut().remove(&key);
            } else {
                c.stack.live_mut().set(key.clone(), l);
            }
            Ok(())
        });
        if let Ok(snap) = done {
            let mut k = Map::new();
            key_fields(st, &key, &mut k);
            let _ = st.events.send(BusEvent::Scenario {
                action: "expire",
                name: None,
                chaos_key: Some(Value::Object(k)),
                active: snap.stack.active().to_vec(),
            });
            n += 1;
        } else {
            // As for scenarios: the restore put the past-due leases back; drop only them.
            let label = key_label(st, &key);
            eprintln!(
                "kaanal: could not expire chaos on {label}; left it in place and dropped its lease"
            );
            for u in &units {
                c.chaos_leases.remove(&(key.clone(), *u));
            }
        }
    }
    n
}

/// **Private.** The earliest deadline over scenario and chaos leases (spec L4: one timer).
pub(super) fn next_deadline(c: &Control) -> Option<Instant> {
    c.leases
        .values()
        .chain(c.chaos_leases.values())
        .min()
        .copied()
}

/// Expires leases as their deadlines pass; spawned once by `serve`. Sleeps to the earliest
/// deadline of both tables, or until a write adds or replaces a lease.
///
/// No wake-up is lost: `notify_one` stores a permit when nobody waits, so a lease added between
/// reading the deadline and polling `notified()` makes that `notified()` return at once.
pub async fn lease_task(st: Arc<AppState>) {
    loop {
        let next = next_deadline(&lock(&st));
        match next {
            None => st.lease_wake.notified().await,
            Some(d) => tokio::select! {
                () = tokio::time::sleep_until(d) => { expire_due(&st); }
                () = st.lease_wake.notified() => {}
            },
        }
    }
}
