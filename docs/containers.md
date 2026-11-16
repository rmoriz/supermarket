# Container evaluation stack

A single-command stack that proves the signed cookbook API end to end: Postgres,
Redis, an Infra Server (`microwave`), the port and a Workstation image. It is
for evaluation on one machine, not a production deployment.

## Start

```sh
docker compose up -d --build
docker compose ps                                   # wait for healthy
docker compose exec workstation /work/demo.sh
```

`demo.sh` signs in as the `workstation` node, uploads the bundled `demo`
cookbook, downloads it again, and asserts the `show` and `list` responses. A
green run ends with `demo: PASS`.

The published port is loopback-only on `127.0.0.1:3000`. Override the host side
with `SUPERMARKET_PORT=13000` or bind wider with `SUPERMARKET_BIND=0.0.0.0`.

## Verify

```sh
docker compose ps                    # db/redis/microwave/supermarket healthy, workstation running
curl -s http://127.0.0.1:3000/api/v1/health   # "status":"ok"
docker compose logs --tail 20 supermarket
docker compose down -v               # drops both databases, keys and uploads
```

The demo uploads version `0.1.0`; a second run against the same volumes fails
with "same version already exists". Use `docker compose down -v` for a clean
run.

## Where the pieces live

- `Containerfile` — the application image (default target) and the one-shot
  `bootstrap` stage.
- `containers/bootstrap.sh` — waits for both servers, generates the Workstation
  keypair, creates the server user/org/key and the Supermarket account.
- `containers/workstation/` — `knife.rb`, the demo cookbook and `demo.sh`.
  Bind-mounted read-only at `/work`; the keypair lives in the
  `workstation-keys` volume at `/keys`.
- `harness/schema.sql` and `containers/db/02-microwave.sql` — database
  initialization.
- Named volumes: `db-data`, `microwave-data`, `uploads`, `workstation-keys`.

## Infra Server image

The default is `ghcr.io/rmoriz/microwave-server:0.6.3`. A local build is
selectable without editing compose:

```sh
MW_IMAGE=microwave-server:local docker compose up -d --build
```

## Security caveats

Evaluation-grade only:

- Fixed database passwords, a fixed `SECRET_KEY_BASE` default and no TLS.
- The Workstation runs as root and its private key sits in a Docker volume.
- Port `3000` is bound to loopback; the Infra Server listens only on the
  compose network.
- The bootstrap Supermarket account is an admin and `OWNERS_CAN_REMOVE_ARTIFACTS`
  is unset, so only admins can delete cookbooks.
- Uploads and both databases are deleted by `down -v`; nothing is backed up.
