//! `kaanal up`: start the server in the foreground or detached, the startup banner and the exposure warnings.

use crate::{
    Config, Duration, Fail, IpAddr, Path, PathBuf, RUN_FILE, STATE_DIR, ServeOpts, ShutdownReply,
    auth_users, blank_loaded, clean, err, json, keys, lifecycle, load, located_err, lock_instance,
    post_shutdown, private_dir, write_run_file,
};

/// **Private.** The `up` lifecycle: private state dir → instance lock → signing key (auth
/// configs only) → token → tokio runtime → serve → run file → wait for a stop signal or
/// `POST /__kaanal/shutdown` → drain → clean up the run file (only if it still holds our token).
/// `detached` (the hidden `--detached`) is `up -d`'s child: its stdout is a log file, so the UI
/// URL carries no token (M4), and it ignores SIGHUP (M11).
pub(crate) fn up(
    config: &Path,
    port: Option<u16>,
    host: IpAddr,
    detached: bool,
    blank: bool,
) -> Result<(), Fail> {
    start(config, port, host, detached, blank, false)
}

/// **Private.** `kaanal studio`: `up --blank`, and the browser opens on the UI unless `no_open`.
pub(crate) fn studio(
    config: &Path,
    port: Option<u16>,
    host: IpAddr,
    no_open: bool,
) -> Result<(), Fail> {
    start(config, port, host, false, true, !no_open)
}

/// **Private.** Best effort: asks the desktop to open `url`. A failure is silent; the URL is
/// printed either way.
fn open_browser(url: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = cmd
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// **Private.** The body of `up` and `studio`.
fn start(
    config: &Path,
    port: Option<u16>,
    host: IpAddr,
    detached: bool,
    blank: bool,
    open: bool,
) -> Result<(), Fail> {
    let loaded = if blank {
        if config.exists() {
            eprintln!(
                "note: {} exists and is not loaded; Save replaces it only if it has not changed since now",
                config.display()
            );
        }
        blank_loaded(config)?
    } else {
        load(config)?
    };
    private_dir(Path::new(STATE_DIR))?;
    // Held until this process exits. The OS releases it even if we crash, so there is no stale lock,
    // and two `up` runs started at the same instant cannot both get past this point.
    let lock = lock_instance()?;
    serve_locked(
        config,
        loaded,
        port,
        host,
        Mode { detached, blank },
        lock,
        |s| {
            let port = s.port;
            println!("Mocks: http://localhost:{port}");
            if detached {
                println!("UI:    http://localhost:{port}/__kaanal/ui");
            } else {
                let ui = format!("http://localhost:{port}/__kaanal/ui#token={}", s.token);
                println!("UI:    {ui}");
                if open {
                    open_browser(&ui);
                }
            }
            if blank {
                println!(
                    "Blank: no system yet; build one in the UI or with `kaanal draft`, then apply it (Save writes {})",
                    config.display()
                );
            }
            if let Some(a) = s.auth {
                println!("Auth:  {a}");
            }
            if let Some(seed) = s.drawn_seed {
                println!("{}", seed_line(seed));
            }
        },
    )
}

/// **Private.** How [`serve_locked`] starts: as `up -d`'s background child (`detached`: the token
/// stays out of the output, hangup is ignored) and/or with no system (`blank`).
#[derive(Clone, Copy, Default)]
pub(crate) struct Mode {
    pub(crate) detached: bool,
    pub(crate) blank: bool,
}

/// **Private.** What a started instance tells the caller of [`serve_locked`], to print.
pub(crate) struct Started<'a> {
    pub(crate) port: u16,
    pub(crate) token: &'a str,
    /// The `Auth:` text, with an auth node.
    pub(crate) auth: Option<&'a str>,
    /// The seed picked at startup (`determinism: realistic`), else `None`.
    pub(crate) drawn_seed: Option<u64>,
    /// The seed in force: the drawn one, else the config's.
    pub(crate) seed: u64,
    pub(crate) warnings: &'a [String],
}

/// **Private.** The rest of [`up`] once `lock` (the instance lock) is held: shared by `up` and
/// `kaanal demo`, which print different lines through `announce` (called once, after `run.json`
/// is written).
pub(crate) fn serve_locked(
    config: &Path,
    (cfg, dir, src): (Config, PathBuf, String),
    port: Option<u16>,
    host: IpAddr,
    mode: Mode,
    _lock: std::fs::File,
    announce: impl FnOnce(&Started),
) -> Result<(), Fail> {
    let Mode { detached, blank } = mode;
    // The lock proves no live instance owns run.json, so whatever is there is a dead one's (M9).
    // Removed now, not overwritten after binding, so nothing reads it as live during startup.
    let _ = std::fs::remove_file(RUN_FILE);
    // X4: only an auth node needs a key. It is loaded (or created) before binding, so a bad
    // key file stops startup with its name; `validate` and `endpoints` never get here.
    let users = if blank { None } else { auth_users(&cfg) };
    let auth_key = match users {
        // a blank start cannot know whether an auth node will be added; the key lives in memory
        // for this session (the project's own key file is made by `kaanal up` once it is saved)
        _ if blank => Some(kaanal_server::AuthKey::generate().map_err(err)?),
        Some(_) => {
            // A config that cannot load must not leave a key behind: check it first (the
            // same load `serve` does, so a topology or spec error shows before any keygen).
            kaanal_server::check(&cfg, &dir).map_err(|e| located_err(config, &src, &e))?;
            Some(keys::signing_key(config)?)
        }
        None => None,
    };
    exposure_warning(host);
    let token = kaanal_server::new_token();
    let rt = tokio::runtime::Runtime::new().map_err(|e| err(e.to_string()))?;
    rt.block_on(async {
        // Handlers first, before anything is published: a signal that arrives while we start up
        // is queued and drained, not fatal (and `up -d`'s child survives a hangup from the start).
        let stop_requested = lifecycle::stop_signal();
        if detached {
            lifecycle::ignore_hangup();
        }
        let opts = ServeOpts {
            host,
            port: port.unwrap_or(cfg.port),
            token: token.clone(),
            auth_key,
            draft: Some(kaanal_server::DraftSource {
                file: config.to_path_buf(),
                text: src.clone(),
            }),
        };
        let started = if blank {
            kaanal_server::serve_blank(&dir, config, opts).await
        } else {
            kaanal_server::serve(&cfg, &dir, opts).await
        };
        let server = started.map_err(|e| located_err(config, &src, &e))?;
        let port = server.addr.port();
        for w in &server.warnings {
            eprintln!("warning: {}", clean(w));
        }
        let auth = users.map(|n| {
            format!(
                "POST http://localhost:{port}/login, JWKS http://localhost:{port}/.well-known/jwks.json, {n} {}",
                if n == 1 { "user" } else { "users" }
            )
        });
        write_run_file(
            server.addr,
            &token,
            &json!({ "auth": auth, "seed": server.drawn_seed, "warnings": server.warnings }),
        )?;
        announce(&Started {
            port,
            token: &token,
            auth: auth.as_deref(),
            drawn_seed: server.drawn_seed,
            seed: server.drawn_seed.unwrap_or(cfg.seed),
            warnings: &server.warnings,
        });
        // After the port is bound, the banner is out and off the serving path, so a big spec never
        // slows startup and its lines never interleave with the banner.
        // A plain thread: a blocking task would make the runtime wait for it on shutdown.
        let (check_cfg, check_dir) = (cfg.clone(), dir.clone());
        if !blank {
            std::thread::spawn(move || {
                for w in kaanal_server::selfcheck(&check_cfg, &check_dir).unwrap_or_default() {
                    eprintln!("warning: {}", clean(&w));
                }
            });
        }
        // A stop signal (or POST /__kaanal/shutdown, which ends `wait` by itself) drains for up
        // to 5s, then cuts what is left; either way the run file goes below.
        let stop = server.stopper();
        let wait = server.wait();
        tokio::pin!(wait);
        let res = tokio::select! {
            r = &mut wait => r,
            () = stop_requested => {
                stop(Duration::from_secs(5));
                wait.await
            }
        };
        // Only remove the file if it is still ours; a newer instance may have replaced it.
        if lifecycle::read_run(Path::new(STATE_DIR)).is_some_and(|r| r.token == token) {
            let _ = std::fs::remove_file(RUN_FILE);
        }
        res.map_err(|e| err(e.to_string()))
    })
}

/// **Private.** The network-exposure warning for a non-loopback `--host`.
pub(crate) fn exposure_text(host: IpAddr) -> String {
    format!(
        "warning: binding {host} makes the mock and control ports reachable from your network; control calls still need the token and Host localhost/127.0.0.1/[::1]"
    )
}

/// **Private.** Prints [`exposure_text`] to stderr for a non-loopback `--host`.
pub(crate) fn exposure_warning(host: IpAddr) {
    if !host.is_loopback() {
        eprintln!("{}", exposure_text(host));
    }
}

/// **Private.** The `Seed:` line `up` prints under `determinism: realistic`.
pub(crate) fn seed_line(seed: u64) -> String {
    format!(
        "Seed:  {seed} (determinism: realistic; replay its timings and failures with determinism: controlled and seed: {seed}; generated data then follows that seed too)"
    )
}

/// **Private.** `--wait` of `up -d`. A bad value is a clap error, which `main` turns into exit 64.
pub(crate) fn start_budget(s: &str) -> Result<Duration, String> {
    humantime::parse_duration(s.trim())
        .ok()
        .filter(|d| (Duration::from_millis(1)..=Duration::from_secs(3600)).contains(d))
        .ok_or_else(|| format!("bad wait '{s}' (1ms to 1h, e.g. 15s)"))
}

/// **Private.** How long `down` waits for the lock after asking: its own budget, or the time left
/// of an earlier, longer drain if that is more (a second `down` must not cut it short), plus 2 s.
pub(crate) fn drain_wait(timeout: Duration, reply: Option<&ShutdownReply>) -> Duration {
    timeout.max(reply.and_then(|r| r.left).unwrap_or_default()) + Duration::from_secs(2)
}

/// **Private.** The lines `up` prints once the server is up, rebuilt from `run.json`'s `startup`.
/// The UI URL carries the token: the caller's terminal is the one place it may go.
pub(crate) fn print_started(run: &lifecycle::Run) {
    let port = run.addr.port();
    println!("Mocks: http://localhost:{port}");
    println!(
        "UI:    http://localhost:{port}/__kaanal/ui#token={}",
        run.token
    );
    if let Some(a) = &run.startup.auth {
        println!("Auth:  {}", clean(a));
    }
    if let Some(seed) = run.startup.seed {
        println!("{}", seed_line(seed));
    }
    for w in &run.startup.warnings {
        eprintln!("warning: {}", clean(w));
    }
}

/// **Private.** An instance that is already up: print its lines and `already running (pid N)`.
pub(crate) fn print_running(run: &lifecycle::Run) {
    print_started(run);
    println!(
        "already running (pid {})",
        run.pid.map_or("unknown".to_string(), |p| p.to_string())
    );
}

/// **Private.** Looks at the directory once: free (stale state cleaned), a healthy instance, or
/// busy (the lock is held but nothing answers yet: starting, or draining).
pub(crate) fn look(state: &Path) -> Result<lifecycle::Look, Fail> {
    if lifecycle::clear_stale(state)?.is_some() {
        return Ok(lifecycle::Look::Free);
    }
    Ok(lifecycle::read_run(state)
        .filter(|r| lifecycle::healthy(r.addr))
        .map_or(lifecycle::Look::Busy, lifecycle::Look::Healthy))
}

/// **Private.** `kaanal up -d` (spec M3): validate here, then start `up --detached` in the
/// background with its output in `.kaanal/kaanal.log`, wait until it answers, print its lines.
pub(crate) fn up_detached(
    config: &Path,
    port: Option<u16>,
    host: IpAddr,
    wait: Duration,
) -> Result<(), Fail> {
    let (cfg, dir, src) = load(config)?;
    // The same checks `serve` makes, before anything is spawned or created: a bad config is
    // an error here, with its file:line:col, not a log tail of a child that died.
    kaanal_server::check(&cfg, &dir).map_err(|e| located_err(config, &src, &e))?;
    let state = Path::new(STATE_DIR);
    if !state.exists() {
        private_dir(state)?;
    }
    // One `up -d` at a time per directory, from here until the child answers: a second one blocks
    // (for up to the winner's --wait plus its checks, not its own --wait; the two can add up),
    // then finds the instance running and prints it (exit 0) instead of rotating the live log away.
    // The child's instance lock stays the final arbiter. Not inherited by the child.
    let _start = lock_start(state)?;
    // A healthy instance is the answer; one that is starting or draining is waited for (within
    // --wait); a free directory (stale state cleaned) means start fresh.
    if let lifecycle::Existing::Running(run) =
        lifecycle::existing_instance(wait, Duration::from_millis(100), || look(state))?
    {
        print_running(&run);
        return Ok(());
    }
    exposure_warning(host);
    let log_path = lifecycle::rotate_log(state)?;
    let log = std::fs::OpenOptions::new()
        .append(true)
        .open(&log_path)
        .map_err(|e| err(format!("{}: {e}", log_path.display())))?;
    let exe =
        std::env::current_exe().map_err(|e| err(format!("cannot find kaanal itself: {e}")))?;
    let cwd = std::env::current_dir().map_err(|e| err(e.to_string()))?;
    let args: Vec<std::ffi::OsString> = vec![
        "up".into(),
        "-c".into(),
        config.into(),
        "--port".into(),
        port.unwrap_or(cfg.port).to_string().into(),
        "--host".into(),
        host.to_string().into(),
        "--detached".into(),
    ];
    let mut child = lifecycle::spawn_detached(&exe, &args, log, &cwd)?;
    let log_shown = std::path::absolute(&log_path).unwrap_or(log_path.clone());
    let began = std::time::Instant::now();
    let ready = lifecycle::wait_ready(state, &mut child, wait, Duration::from_millis(100));
    let why = match ready {
        Ok(run) => {
            print_started(&run);
            println!("Log: {}", clean(&log_shown.display().to_string()));
            println!("Stop: kaanal down");
            return Ok(());
        }
        Err(lifecycle::ReadyError::ChildExited) => {
            // Lost a race with a simultaneous `up -d`: the instance that won is the answer.
            let left = wait.saturating_sub(began.elapsed());
            if let Some(run) =
                lifecycle::after_child_exit(left, Duration::from_millis(100), || look(state))?
            {
                print_running(&run);
                return Ok(());
            }
            "the background process exited".to_string()
        }
        Err(lifecycle::ReadyError::TimedOut) => {
            let pid = child.id();
            let healthy = || {
                lifecycle::run_of(state, pid)
                    .filter(|r| lifecycle::healthy(r.addr))
                    .filter(|_| {
                        matches!(
                            lifecycle::lock_probe(state),
                            Ok(lifecycle::Liveness::Running)
                        )
                    })
            };
            let settled = lifecycle::settle_unready(
                healthy,
                || lifecycle::run_of(state, pid),
                |run| {
                    post_shutdown(run, "5s");
                },
                || {
                    lifecycle::wait_lock_free(state, Duration::from_secs(7));
                },
                || {
                    let _ = child.kill();
                },
            );
            if let lifecycle::Settled::Healthy(run) = settled {
                eprintln!(
                    "note: kaanal answered just after the {} wait; leaving it running",
                    humantime::format_duration(wait)
                );
                print_started(&run);
                println!("Log: {}", clean(&log_shown.display().to_string()));
                println!("Stop: kaanal down");
                return Ok(());
            }
            let _ = child.wait();
            format!("no answer within {}", humantime::format_duration(wait))
        }
    };
    Err(err(format!(
        "kaanal did not start: {why}; last lines of {}:\n{}",
        log_shown.display(),
        Some(lifecycle::tail_log(&log_path, 20))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "(log is empty)".to_string())
    )))
}

/// **Private.** Takes `.kaanal/start.lock`, waiting for any other `up -d` in this directory.
pub(crate) fn lock_start(state: &Path) -> Result<std::fs::File, Fail> {
    let path = state.join("start.lock");
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| err(format!("{}: {e}", path.display())))?;
    f.lock()
        .map_err(|e| err(format!("{}: {e}", path.display())))?;
    Ok(f)
}
