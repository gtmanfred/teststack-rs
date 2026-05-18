# ts (teststack)

Rust port of [teststack](https://github.com/gtmanfred/teststack), a CLI for
managing Docker containers that back a project's test suite. Reads a
`teststack.toml`, brings up service containers (databases, caches, brokers),
optionally builds a test image from `Dockerfile.j2`, injects environment
variables computed from the running containers, and executes an ordered DAG of
test steps inside a `tests` container.

This rewrite is binary-compatible with existing `teststack.toml` files. The
binary is named `ts`.

## Install

From source:

```bash
cargo install --path .
# or, after a release tag has been cut:
# cargo install ts
```

Prebuilt binaries are attached to each GitHub Release (`v*` tags):
`aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`. Download the tarball matching your platform and
drop the extracted `ts` binary on your `$PATH`.

## Quick start

```bash
cd path/to/your/project          # contains teststack.toml + Dockerfile.j2
ts build                          # render Dockerfile.j2 and build the image
ts start                          # bring up services + tests container
ts env --inside                   # show env vars for inside the tests container
ts run                            # execute every step in tests.steps
ts run --step flake8              # execute one step
ts exec -- bash                   # drop into the tests container
ts stop                           # tear down all containers and prune the network
```

## Commands

| Command   | Purpose                                                       |
| --------- | ------------------------------------------------------------- |
| `tag`     | Print the computed image tag (`<dir>:<git-tag-or-commit>`)    |
| `render`  | Render `Dockerfile.j2` -> `Dockerfile` via Jinja              |
| `build`   | Render if stale, then `docker build` with BuildKit + secrets  |
| `start`   | Bring up service containers + the tests container             |
| `stop`    | Stop and remove all containers, prune the project network    |
| `restart` | `stop` then `start`                                           |
| `status`  | Show status table for all project containers                  |
| `env`     | Print `export VAR=value` lines for the running services       |
| `exec`    | `docker exec -ti` into the tests container                    |
| `run`     | Execute the `tests.steps` DAG inside the tests container      |

Chaining is supported. Global flags must precede the first subcommand:

```bash
ts -p ../other-project stop start run --step tests -- -k test_users
```

## Config (`teststack.toml`)

See the [upstream docs](https://teststack.readthedocs.org) for the full schema.
Notable behavior preserved by this port:

- TOML order is preserved. `services` start in the order declared; `tests.steps`
  execute in declaration order (`process_steps` returns an `IndexMap`).
- `tests.steps.<name>` can be a string, an array of strings, or a table with
  `command`, `check`, `requires`, and `user`. The step DAG mirrors the Python
  implementation: `check` exit code `0` skips the step; `requires` cascades;
  `required_by` skips when no dependent needs the work.
- `export` values are interpolated against a service's `environment` table plus
  `HOST` and `PORT;<port>/<proto>` derived from the running container. With
  `--inside`, `HOST` becomes the container's IP on the project network and
  ports return the container-side number; without it, `HOST=localhost` and
  ports return the host-mapped value.
- `tests.mounts.<name>.secret = true` is wired as `--secret id=<name>,src=...`
  on `docker build`; runtime mounts use bind mounts with the declared
  `target`/`mode`.
- String `command` values are shell-split (not wrapped in `sh -c`), so postgres
  `command = "-c max_connections=25"` reaches the entrypoint as two args.

## Docker socket

Resolution order:

1. `DOCKER_HOST` env var
2. `DOCKER_CONTEXT` env var
3. `currentContext` in `~/.docker/config.json` (parsed from
   `~/.docker/contexts/meta/<sha256>/meta.json`)
4. `connect_with_local_defaults` (`/var/run/docker.sock`)

So setups like Docker Desktop on macOS or Lima/Colima that publish a non-default
context work without extra configuration.

## What's not ported (yet)

- `import` / `import-env` / `copy` commands (recursive CLI re-entry pattern).
- Podman backend; `client.name = "docker"` is the only supported value in v1.
- The entry-point plugin system for third-party commands/clients.

## Development

```bash
cargo test                                  # unit tests for config + step DAG
cargo build --release
cp target/release/ts ~/.local/bin/ts        # local install
```

Releases are cut by pushing a `v*` tag. See `.github/workflows/release.yml`.
