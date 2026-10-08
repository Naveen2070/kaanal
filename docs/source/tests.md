# Tests, explained

Every test in the workspace, grouped by file. Run all with `cargo test --workspace` (must pass before any task is done — rule R7.1).

Fixtures shared by the server and CLI tests live in `crates/kaanal-server/tests/fixtures/`: `kaanal.yaml` (gateway `nginx` → service `app` → database `db`, all 1 ms costs, seed 42, two static route overrides), `openapi.yaml` (users/orders spec), `openapi-warn.yaml` (spec containing an ignored `pattern` keyword).

---

## `crates/kaanal-core/tests/config.rs` — YAML parsing

| Test                                                         | What it proves                                                                                                                                                        |
| ------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `parses_sample`                                              | A full sample file lands in the right typed fields: seed, port, node kinds, `spec`, edges, route override default status 200, and `proc: 5ms` parses as `Fixed(5ms)`. |
| `defaults_apply`                                             | Minimal config (`version: 1, nodes: {}`) gets seed 0, port 8080, empty edges.                                                                                         |
| `bad_value_reports_line`                                     | A type error deep in the file reports the right **line number** and renders as `3: …` (rule R6.1).                                                                    |
| `unknown_field_is_rejected`                                  | `deny_unknown_fields`: a `bogus:` key fails and names itself.                                                                                                         |
| `wrong_version_is_rejected`                                  | `version: 2` fails with a version message.                                                                                                                            |
| `bom_and_crlf_parse_like_lf`                                 | UTF-8 BOM + Windows CRLF line endings parse identically (rule R6.4).                                                                                                  |
| `bad_duration_explains_what_is_expected`                     | Bad `proc` values (in both duration shapes) fail with the friendly "expected a duration like …" message and the right line — not serde's untagged-enum gibberish.     |
| `huge_durations_are_rejected_with_a_position`                | A duration or p99 over 1 h is rejected with its line (summed hops would overflow); exactly 1 h is fine.                                                               |
| `exponential_duration_parses_and_is_strict`                  | `{ mean: 5ms }` parses; `{ mean: 2h }` is rejected with its line; `{ mean, p99 }` is rejected with the "expected a duration like" message.                            |
| `concurrency_parses_on_every_node_type_and_must_be_positive` | `concurrency` parses on gateway/service/database, is carried into `Node`, `0`/`-1`/`1.5` are rejected with line 3, absent = unlimited.                                |

## `crates/kaanal-core/tests/dist.rs` — latency distributions

| Test                                            | What it proves                                                                                                                                                                                  |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lognormal_hits_p50_and_p99_within_5_percent`   | 100 000 samples of a fitted lognormal land within 5 % of the configured p50 and p99 (mirrors NFR-11's 5 % accuracy bar).                                                                        |
| `fixed_is_fixed`                                | `Fixed(5ms)` always samples 5 ms.                                                                                                                                                               |
| `p99_not_above_p50_falls_back_to_fixed`         | Degenerate p50/p99 pairs (equal, or p99 < p50) become `Fixed(p50)` instead of producing a broken distribution.                                                                                  |
| `fat_tail_is_capped_at_ten_times_p99`           | Even a 1 ms/1 s distribution never samples above 10×p99 — a fat tail can't hang a request.                                                                                                      |
| `same_seed_same_samples`                        | Same seed → identical sample vectors (determinism).                                                                                                                                             |
| `exponential_has_the_configured_mean_and_a_cap` | 100k samples: mean within 2%, median ≈ mean·ln 2, never above 10 × mean, same seed → same samples.                                                                                              |
| `stable_hash_is_fixed_and_separates_parts`      | The FNV-1a hash matches a hard-coded known value (so it can never silently change) and `["ab","c"]` ≠ `["a","bc"]`. These vectors are duplicated in kaanal-data to pin the two copies together. |

## `crates/kaanal-core/tests/topology.rs` — graph validation

| Test                                                  | What it proves                                                                                     |
| ----------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `builds_base`                                         | Happy path: 3 nodes, gateway identified, name lookups, edge lookups, `out_edges` order.            |
| `rejects_cycle`                                       | `db → app` closing a loop fails with `Cycle(["app","db","app"])`.                                  |
| `rejects_unknown_node_in_edge`                        | An edge to `redis` (undefined) fails naming the edge number and the name.                          |
| `rejects_two_gateways` / `rejects_zero_gateways`      | Exactly one gateway is required; both directions fail with `GatewayCount`.                         |
| `rejects_database_without_ops`                        | A database needs `ops: read/write/scan`.                                                           |
| `rejects_service_spec_without_gateway_edge`           | A service with a spec unreachable from the gateway is an error (its routes would never be walked). |
| `rejects_service_calling_service_in_phase_one`        | `service → service` edges are `Unsupported` — phase one walks only gateway → service → database.   |
| `errors_display_human_text`                           | Error `Display` is human prose (`cycle: a -> b -> a`).                                             |
| `rejects_duplicate_edge`                              | The same from→to pair twice is rejected.                                                           |
| `rejects_edges_the_executor_would_ignore`             | `gateway → database` direct edges are rejected, not silently skipped.                              |
| `rejects_ops_on_non_database_and_spec_on_non_service` | `ops` on a service and `spec` on a gateway each fail with a targeted message.                      |

## `crates/kaanal-core/tests/exec.rs` — the chain executor

| Test                                            | What it proves                                                                                                                              |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `same_inputs_same_outcome`                      | Same `(seed, endpoint, n)` → identical outcome (determinism, rule R4.1).                                                                    |
| `different_n_gives_different_timings`           | The request counter changes the draws.                                                                                                      |
| `happy_path_visits_gateway_service_db_in_order` | Exact hop sequence `nginx.proc → net → app.proc → net → db.read`, result `Ok`, and `total()` equals the hop sum.                            |
| `error_rate_one_always_fails_at_that_node`      | 100 requests all fail at `db` with 504; the 4 hops before it still count toward the delay.                                                  |
| `down_node_returns_503`                         | `down` on `app` → 503 at `app`, walk stops after 2 hops.                                                                                    |
| `error_rate_zero_never_fails`                   | 1000 requests with a zero-rate chaos entry all succeed.                                                                                     |
| `latency_add_lands_on_that_node`                | +100 ms chaos appears in the db op hop's duration.                                                                                          |
| `inert_chaos_entry_does_not_change_timings`     | An all-default chaos entry leaves the outcome bit-identical — the error roll is always drawn, so chaos settings never shift the timing RNG. |
| `static_route_only_touches_gateway`             | `entry: None` walks only the gateway proc hop.                                                                                              |
| `op_inference`                                  | `GET /users/{id}` → read, `GET /users` → scan, POST/DELETE → write; case-insensitive; `Op::name()`.                                         |
| `gateway_down_fails_static_route_with_no_hops`  | Gateway chaos applies even to static routes, failing before any hop is recorded.                                                            |

## `crates/kaanal-data/tests/api.rs` — OpenAPI parsing

| Test                                      | What it proves                                                                                                                           |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `loads_endpoints_with_inlined_refs`       | 4 endpoints load; lowest 2xx wins (404 ignored); `$ref`s fully inlined (no `$ref` key remains); array-item refs inlined too.             |
| `picks_201_and_no_body_cases`             | 201 chosen when it's the only 2xx; 204 has no schema.                                                                                    |
| `rejects_openapi_31`                      | 3.1 fails, naming the version.                                                                                                           |
| `rejects_external_ref`                    | `other.yaml#/User` fails ("external $ref").                                                                                              |
| `rejects_recursive_schema`                | Self-referencing schema fails fast ("recursive").                                                                                        |
| `rejects_missing_ref_target`              | Dangling ref fails ("not found").                                                                                                        |
| `rejects_branching_recursive_schema_fast` | A cycle through fan-out 2 fails immediately, not after 2^depth work.                                                                     |
| `rejects_ref_expansion_bomb`              | An _acyclic_ chain doubling each level (2³⁰ nodes — "billion laughs") hits the 10 000-expansion cap and errors instead of eating memory. |
| `accepts_unquoted_numeric_version`        | `openapi: 3.0` (YAML number, not string) is accepted.                                                                                    |
| `rejects_path_without_leading_slash`      | `users:` (no `/`) fails with a clear message.                                                                                            |

## `crates/kaanal-data/tests/generate.rs` — data generation

| Test                                                     | What it proves                                                                                                                                                                  |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `same_seed_same_value_different_seed_differs`            | Seed reproducibility both ways.                                                                                                                                                 |
| `output_validates_against_schema`                        | 200 seeds of a rich schema all pass `jsonschema` validation (rule R7.2: invalid output is a bug).                                                                               |
| `allof_merges_properties`                                | `allOf` parts merge into one object with both property sets.                                                                                                                    |
| `formats_look_right`                                     | date-time is 20 chars `…T…Z`; uuid is 36 chars with the v4 nibble.                                                                                                              |
| `example_and_enum_win`                                   | `example` is returned verbatim; single-choice `enum` is honoured.                                                                                                               |
| `field_name_heuristics`                                  | `email`/`user_email` → `*@example.com`; `firstName` alphabetic; `pincode` 6 digits; `createdAt` in 2025–2026; `price` 99–4999; `phone` `+91 …`; `currency` INR; city non-empty. |
| `apply_params_sets_matching_fields`                      | Path params overwrite same-named fields with type conversion (`"42"` → `42`).                                                                                                   |
| `apply_params_keeps_integer_when_param_is_not_numeric`   | `/users/abc` never corrupts a numeric `id`.                                                                                                                                     |
| `deep_or_odd_schemas_do_not_panic`                       | Empty schema, `minItems=maxItems=0`, and `minimum > maximum` all return something sane.                                                                                         |
| `huge_max_items_stays_small`                             | `maxItems: 1_000_000` still yields ≤ 20 items (the cap).                                                                                                                        |
| `only_real_id_fields_get_ids`                            | `userId`/`orderId` get id treatment; `valid`/`paid` do not (tests the `is_id` word-boundary rules).                                                                             |
| `minimum_above_default_range_still_varies`               | A `minimum: 5000` above the default range still produces _varying_ values ≥ 5000.                                                                                               |
| `adding_or_reordering_properties_keeps_other_values`     | The per-field seed rule (R4.4): shared fields keep identical values across the two schema shapes.                                                                               |
| `exclusive_bounds_are_strict`                            | OpenAPI 3.0 boolean `exclusiveMinimum/Maximum`: int range collapses to exactly 2; floats stay strictly inside.                                                                  |
| `length_limits_never_break_formats`                      | `maxLength: 8` doesn't truncate an email; `minLength: 50` doesn't pad a uuid; name-heuristic emails are also protected (rule R4.5).                                             |
| `city_state_pincode_and_address_describe_the_same_place` | 300 seeds: one object always has a consistent (city, state, pincode) triple and the address ends in that city; cities still vary across seeds.                                  |
| `names_and_email_describe_the_same_person`               | 300 seeds: `name` is `firstName lastName` and `email` starts with `first.last`; people still vary across seeds.                                                                 |
| `unknown_formats_still_get_length_limits`                | `format: password` + `minLength`/`maxLength` stays within the lengths (only built formats skip them).                                                                           |
| `huge_or_deeply_nested_arrays_are_bounded`               | `minItems: 1e9` yields at most 10 000 items; five nested 20-item levels stay under 1 MB (per-body value budget).                                                                |

## `crates/kaanal-data/src/warn.rs` (unit) + `tests/warnings.rs` — ignored-keyword warnings

| Test                                                   | What it proves                                                                                                                                       |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `flags_ignored_keywords_but_not_property_names`        | A _property called_ `pattern` is not flagged; a real `pattern:` keyword and `uniqueItems` are, with locations.                                       |
| `walks_items_and_combinators`                          | `not` nested inside `items → allOf` is found.                                                                                                        |
| `flags_max_length_that_cannot_fit_the_format`          | `format: uuid` + `maxLength: 10` warns (the format wins); `maxLength: 36` does not.                                                                  |
| `clean_schema_has_no_warnings`                         | No ignored keywords → no warnings.                                                                                                                   |
| `too_deep_schema_warns_once_and_matches_the_generator` | (unit) 16 levels: no warning and a filled leaf; 17 levels on two branches: one warning per endpoint, and the generator returns `null` at that depth. |
| `api_model_collects_warnings_per_endpoint`             | End-to-end from a spec string: `ApiModel::warnings()` prefixes warnings with `GET /things`.                                                          |

## `crates/kaanal-server/tests/state.rs` — load pipeline (no HTTP)

| Test                                                     | What it proves                                                                            |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `fixture_config_is_valid`                                | The shared fixture passes `check`.                                                        |
| `missing_spec_file_is_reported_with_path`                | A missing spec file errors, naming the file path.                                         |
| `same_route_in_two_services_conflicts`                   | Two services defining `GET /users/{id}` → error naming both.                              |
| `malformed_route_key_is_rejected`                        | A `routes:` key without `"METHOD /path"` shape fails.                                     |
| `topology_errors_surface`                                | A missing gateway surfaces through `check` with "gateway" in the message.                 |
| `spec_on_non_service_is_rejected`                        | `spec` on a gateway node is rejected.                                                     |
| `unsupported_keywords_become_warnings`                   | The `pattern` keyword in `openapi-warn.yaml` becomes exactly one warning naming the file. |
| `clean_config_has_no_warnings`                           | The clean fixture produces zero warnings.                                                 |
| `concurrency_warns_that_live_mode_ignores_it`            | One warning per node with `concurrency`, naming the node and saying "simulator only".     |
| `token_is_64_hex_and_random`                             | `new_token`: 64 hex chars, two calls differ.                                              |
| `override_differing_only_in_param_name_is_a_clear_error` | `GET /users/{x}` vs spec's `GET /users/{id}` → error naming both spellings.               |
| `override_with_same_param_name_replaces_the_spec_route`  | Same shape, same names → override merges cleanly.                                         |
| `override_needs_known_method_and_leading_slash`          | `FOO /x` and `GET users` each fail with targeted messages.                                |
| `override_status_must_be_a_real_http_status`             | Status 99, 100, 199 and 1000 are rejected (200–599 only).                                 |
| `control_namespace_is_reserved`                          | `routes:` can never claim `/__kaanal` or anything under it.                               |

## `crates/kaanal-server/tests/http.rs` — end-to-end HTTP

Each test boots a real server on an OS-chosen port (`Srv` helper) against the fixtures.

_Mocks:_

| Test                                                          | What it proves                                                                                                                                                    |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `users_42_is_stable_traced_and_uses_path_id`                  | `GET /users/42` twice returns identical bodies; `id` is 42 (param applied); trace header starts `nginx.proc=` and contains `db.read=`; email ends `@example.com`. |
| `non_numeric_id_for_integer_field_still_returns_200`          | `/users/abc` keeps a generated numeric id.                                                                                                                        |
| `static_override_beats_generated`                             | The fixture's `"GET /users": [ { id: 1 } ]` override wins over the spec schema.                                                                                   |
| `static_only_route_works`                                     | `/health` exists only via override and returns 200 `{ ok: true }`.                                                                                                |
| `post_orders_returns_201`                                     | Status comes from the spec (201); body is generated (currency INR).                                                                                               |
| `unknown_and_trailing_slash_paths_are_404`                    | `/nope`, `/users/42/`, `/__kaanal/nope` all 404 `no route`.                                                                                                       |
| `wrong_method_is_405`                                         | DELETE on a GET-only path → 405.                                                                                                                                  |
| `mocks_send_cors_headers_and_answer_preflight`                | `Access-Control-Allow-Origin: *` on responses and 204 preflights.                                                                                                 |
| `head_answers_like_get_without_a_body`                        | HEAD reuses GET (200 + trace header) with an empty body.                                                                                                          |
| `percent_encoded_path_param_is_decoded_before_use`            | `/users/%34%32` behaves as `/users/42`.                                                                                                                           |
| `trace_shows_sub_millisecond_hops_and_names_the_failing_node` | Every hop shows one decimal (`1.0ms`, never `0ms`); with db chaos at 100 %, the trace ends `db.error=504`.                                                        |

_Control API & security:_

| Test                                                           | What it proves                                                                                                                              |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `chaos_on_db_fails_requests_then_clears`                       | PUT 100 % errors → mocks return 503 `{"error":"chaos","node":"db"}`; DELETE → mocks recover.                                                |
| `chaos_patch_merges_and_lists`                                 | Two PUTs merge (`latency_ms` + `down` both kept); GET lists the merged view; DELETE all empties it.                                         |
| `latency_chaos_slows_requests`                                 | +200 ms chaos makes a real request take ≥ 200 ms wall time.                                                                                 |
| `control_requires_token`                                       | No token and wrong token both → 401.                                                                                                        |
| `control_rejects_foreign_host`                                 | `Host: evil.example` → 403.                                                                                                                 |
| `host_guard_covers_the_whole_control_namespace`                | Even the token-free `health` and `ui` reject a foreign Host.                                                                                |
| `unauthenticated_calls_are_rejected_before_the_body_is_parsed` | A PUT with malformed JSON and no token gets 401 (not 400) — the guard runs first.                                                           |
| `query_token_only_works_on_events`                             | `?token=` on `/topology` → 401.                                                                                                             |
| `control_responses_are_not_cacheable`                          | `Cache-Control: no-store` + `X-Content-Type-Options: nosniff`.                                                                              |
| `unknown_node_is_404_with_message`                             | Chaos on `redis` → 404 `unknown node 'redis'`.                                                                                              |
| `bad_chaos_values_are_4xx_with_json_errors`                    | rate 1.5, status 418, latency 300 001 ms, negative latency, unknown keys — all 4xx with a JSON `error` field; range errors are exactly 400. |
| `topology_lists_nodes_edges_routes`                            | `/topology` returns the fixture's nodes, edges and the 4 sorted route keys.                                                                 |
| `events_stream_reports_requests`                               | SSE: a request to `/users/7` appears as `event: request` with the ordered node list `["nginx","app","db"]` and measured `ms` ≥ 5.           |
| `events_without_token_is_401`                                  | SSE needs the token too.                                                                                                                    |

_Embedded UI page:_

| Test                                                                        | What it proves                                                                                                            |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `ui_page_cannot_be_framed_and_sends_no_referrer`                            | CSP `frame-ancestors 'none'`; `Referrer-Policy: no-referrer`.                                                             |
| `ui_page_is_served_without_token`                                           | 200 HTML containing the app name, `EventSource`, and fragment-token reading.                                              |
| `ui_page_avoids_unsafe_dom_apis_and_external_assets`                        | Static guard: no `innerHTML`/`eval(`/inline handlers/`localStorage`/any `http(s)://` URL in the page.                     |
| `ui_controls_have_accessible_names_and_dark_form_controls`                  | `color-scheme: light dark`, aria labels, `role="alert"`.                                                                  |
| `ui_keeps_stream_status_and_api_errors_on_separate_lines_and_uses_counters` | Structural guards on the page's JS: separate status/error elements, fixed-size counter buckets, node names as `Map` keys. |
| `config_seed_changes_generated_bodies`                                      | Same `seed:` → same `/users/42` body; a different `seed:` → different data (path `id` still 42).                          |

## `crates/kaanal/tests/cli.rs` — binary end-to-end

Spawns the real compiled binary; `Up` starts `kaanal up --port 0` in a temp dir and kills it on drop.

| Test                                                           | What it proves                                                                                                                                                                                                                                    |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `validate_ok`                                                  | Fixture config → exit 0.                                                                                                                                                                                                                          |
| `validate_cycle_fails_with_message`                            | A cycle config → exit 1 with "cycle" on stderr.                                                                                                                                                                                                   |
| `validate_bad_yaml_shows_file_line_col`                        | `kaanal.yaml:2:` prefix in the error (rule R6.1).                                                                                                                                                                                                 |
| `validate_prints_unsupported_keyword_warnings`                 | Warnings on stderr, `ok:` on stdout, exit 0.                                                                                                                                                                                                      |
| `status_without_running_instance_exits_2`                      | No `run.json` → exit 2 "not running".                                                                                                                                                                                                             |
| `stale_run_file_exits_2`                                       | A `run.json` pointing at a dead port → exit 2.                                                                                                                                                                                                    |
| `live_status_chaos_and_errors`                                 | Full loop: status shows nodes; `chaos set` with all flags reflected in status; unknown node → exit 1 with the server's message; ANSI escape in a node name is neutralised to `?`; `chaos clear`; second `up` in the same dir → "already running". |
| `chaos_set_without_any_setting_is_an_error_not_an_empty_patch` | `chaos set db` alone → exit 1 "nothing to set".                                                                                                                                                                                                   |
| `killed_instance_does_not_block_the_next_up`                   | After a kill -9 style death, the OS releases the lock; a new `up` starts and `status` works (rule R6.7).                                                                                                                                          |
| `two_simultaneous_ups_let_exactly_one_win`                     | Race: exactly one instance exits 1, the winner keeps running.                                                                                                                                                                                     |
| `run_file_is_restricted_to_the_current_user_on_windows`        | (Windows only) `icacls` shows only the current user on `.kaanal/run.json` — no Everyone/Users grants (rule R5.8).                                                                                                                                 |
| `cli_reaches_an_instance_bound_to_ipv6_loopback`               | `up --host ::1` then `status` works (CLI connects to `run.json`'s host, sends `Host: localhost:<port>`).                                                                                                                                          |
| `bad_arguments_exit_1_not_2`                                   | Usage errors (`--error 70000`, unknown subcommand) exit 1, not clap's 2 ("not running"); `--help` exits 0.                                                                                                                                        |
| `spec_text_cannot_drive_the_terminal`                          | An ESC sequence in a spec property name reaches the warning as `?[31m…`, never as a raw escape.                                                                                                                                                   |
| `sample_project_validates`                                     | `examples/nginx-server/kaanal.yaml` validates with no warnings.                                                                                                                                                                                   |

## Unit tests inside source files

| Location                       | Tests                                                                                                                                                            |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kaanal/src/main.rs`           | `latency_parsing` (unit required), `rate_parsing` (incl. overflow/NaN), `clean_replaces_control_chars` (ANSI neutralising), `path_segments_are_percent_encoded`. |
| `kaanal-server/src/control.rs` | `hosts` (Host guard incl. attack spellings), `constant_time_eq`, `token_comes_only_from_the_token_parameter`.                                                    |
| `kaanal-data/src/generate.rs`  | `stable_hash_matches_core_algorithm` (pins the duplicated hash to core's vectors), `civil_dates` (epoch, BASE_EPOCH, leap day).                                  |
