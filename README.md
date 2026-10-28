# Rust-Backend-Enterprise-Modular-Architecture

A runnable modular monolith built with Rust, Axum, Tokio, PostgreSQL, and SQLx. Auth owns credentials workflows and sessions; Users owns identities and profiles; Health exposes liveness and readiness. Each module keeps its routes, business rules, and SQL together.

## Included

- Registration and login with Argon2id password hashing on bounded blocking workers...
- Short-lived, signed JWT access tokens with issuer, audience, algorithm, and expiry validation.
- Random refresh tokens stored as SHA-256 hashes, transactional rotation, replay detection, and session logout.
- Atomic account/session creation. Revoked sessions and disabled accounts cannot use existing access tokens; roles are read from the database on each authenticated request.
- Self-service profile updates and admin-only user listing and lookup. Public requests cannot set roles.
- Strict configuration, validated JSON, consistent errors, bounded pagination, request IDs, structured logs, CORS allowlist, request timeouts, and peer-IP auth throttling.
- Embedded migrations, isolated database integration tests, a non-root Docker image, local Compose services, and CI checks.

## Start with the configured Neon database

The local `.env` contains the supplied database connection and a generated signing secret. It is excluded from Git and Docker build contexts. `.env.example` contains placeholders only. See [operations](docs/operations.md) for the SQLx TLS compatibility decision.

Install Rust, or use the isolated toolchain prepared for this workspace on Windows:

```powershell
. .\.tooling\use-rust.ps1
cargo run --locked -- check-db
cargo run --locked -- migrate
cargo run --locked -- serve
```

For a fresh clone, copy `.env.example` to `.env`, set your database URL, and replace `JWT_SECRET` with at least 32 random bytes, such as the output of `openssl rand -hex 32`. The example secret deliberately fails startup validation. The server defaults to `http://127.0.0.1:3000`.

Migrations use `DATABASE_URL_UNPOOLED` when present. On Neon, use its direct endpoint for migrations and its pooled endpoint for application traffic. Serving never applies migrations implicitly.

## Local PostgreSQL with Docker

```sh
docker compose up --build -d
curl http://localhost:3000/health/ready
docker compose down
```

Compose uses its own local PostgreSQL database and applies migrations in a one-shot service before starting the API. It reads only `JWT_SECRET` from the root `.env`; it does not use the Neon connection. Data survives `docker compose down` in a named volume.

## API

| Method | Path | Access |
| --- | --- | --- |
| GET | `/health/live` | Public; process health |
| GET | `/health/ready` | Public; database and migration readiness |
| POST | `/api/v1/auth/register` | Public, throttled |
| POST | `/api/v1/auth/login` | Public, throttled |
| POST | `/api/v1/auth/refresh` | Refresh token, throttled |
| POST | `/api/v1/auth/logout` | Bearer access token, throttled |
| GET | `/api/v1/users/me` | Signed-in user |
| PATCH | `/api/v1/users/me` | Signed-in user; `display_name` only |
| GET | `/api/v1/users/` | Admin; `page` and `per_page` |
| GET | `/api/v1/users/{id}` | Admin |

Requests with bodies use `Content-Type: application/json`. Authentication uses `Authorization: Bearer <access_token>`. See the [OpenAPI contract](docs/openapi.yaml) for full schemas and error responses.

```sh
curl -X POST http://localhost:3000/api/v1/auth/register \
  -H 'Content-Type: application/json' \
  -d '{"email":"member@example.com","display_name":"Member","password":"a unique long passphrase"}'
```

Successful JSON responses use `{"data": ...}`. Errors use `{"error":{"code":"unauthorized","message":"..."}}`. Every response carries `X-Request-ID` and `Cache-Control: no-store`; logout returns an empty 204 response. Validation returns 422, missing or invalid authentication 401, insufficient permissions 403, and duplicates 409.

All new accounts have the `user` role. Promote an initial administrator through a controlled database operation described in [operations](docs/operations.md); there is no default admin password or public role-setting endpoint.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

On Windows, `powershell -File scripts/check.ps1` runs the same checks and uses the workspace toolchain if present.

Database tests are explicit and separate. Set `TEST_DATABASE_URL` to a **direct** PostgreSQL endpoint that permits schema creation, then run:

```sh
cargo test --locked --test api_database -- --ignored
```

These tests never load `.env` or fall back to `DATABASE_URL`. They create a uniquely named test schema, apply the actual migrations, exercise the HTTP API, and remove their own schema. CI runs this suite against a disposable PostgreSQL service. A test process killed during execution may leave its uniquely named schema behind.

## Extend

Read [architecture](docs/architecture.md) for module boundaries and [operations](docs/operations.md) for deployment, configuration, and current limitations. Add products, organizations, or billing only when their requirements exist; this project contains no placeholder business modules.
