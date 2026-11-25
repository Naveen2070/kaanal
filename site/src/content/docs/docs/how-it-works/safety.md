---
title: "Safety notes"
sidebar:
  order: 2
---

- Kaanal binds `127.0.0.1`. `--host 0.0.0.0` works but warns; the control API still requires the token and a loopback `Host` header.
- The token is per-run and lives in `.kaanal/run.json`, readable only by you. The background process's log (`.kaanal/kaanal.log`) never contains it.
- `KAANAL_CONFIG_DIR` sets the base folder for the auth signing key (default: your per-user local config folder). See [Auth](/docs/guides/mock/auth/).
- All generated data is synthetic. Emails always use `example.com`.
