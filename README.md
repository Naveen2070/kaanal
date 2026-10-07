# Kaanal

*Kaanal* (கானல், Tamil for "mirage") is a single binary that pretends to be your backend:
OpenAPI mocks with realistic data, simulated gateway → service → database timing, and live chaos.

Status: Phase One (pre-release). Site: https://kaanal.naveenr.in

## Try it

Needs Rust 1.89 or newer.

    cargo run -p kaanal -- up -c examples/nginx-server/kaanal.yaml
    curl -i http://localhost:8080/users/42
    cargo run -p kaanal -- chaos set db --error-rate 100% --error 503
    curl -i http://localhost:8080/users/42      # 503 from the "db"
    cargo run -p kaanal -- chaos clear

Open the `UI:` link that `up` prints to see live requests and change chaos per node.
Run `kaanal` commands from the directory where you ran `up`: they find the running instance
through `.kaanal/run.json` there.

To check the whole flow automatically, start `up` as above, then in another terminal:

    bash examples/nginx-server/check.sh      # or: pwsh examples/nginx-server/check.ps1

Local development only: Kaanal binds to 127.0.0.1 and protects its control API with a per-run token.

License: Apache-2.0
