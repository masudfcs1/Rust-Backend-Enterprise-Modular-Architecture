# Operations

## PostgreSQL and Neon

The runtime connection uses a bounded pool with a 60-second idle timeout and 30-minute maximum connection lifetime. Prepared statement caching is disabled for compatibility with transaction poolers. Application logs do not contain SQL text, bind values, connection strings, passwords, or tokens.

Use the pooled Neon endpoint for `DATABASE_URL` and its direct endpoint for `DATABASE_URL_UNPOOLED`. Run migrations through the direct endpoint because SQLx migration advisory locks require a stable session. The migration command refuses known Neon pooler hosts. Other poolers also require a direct migration URL.

The supplied Neon string contained `channel_binding=require`. SQLx 0.8 does not implement SCRAM channel binding and otherwise ignores unknown connection parameters. This implementation rejects that option. The local `.env` is configured with `sslmode=verify-full` and without `channel_binding`: TLS encryption, certificate trust, and hostname verification are enforced. **This does not provide channel binding.** If channel binding is mandatory, use a PostgreSQL driver that supports it before deployment. References: [SQLx connection option parser](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-postgres/src/options/parse.rs), [SQLx SCRAM implementation](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-postgres/src/connection/sasl.rs).

Remote databases require `sslmode=verify-full` in every environment. Development allows unencrypted local PostgreSQL only. Production requires verified TLS even for local hostnames. Unknown and duplicate URL options are rejected to prevent silently ignored configuration.

## Deployment sequence

1. Build and test the image using the committed lockfile.
2. Inject environment values from the deployment platform's secret store. Use a dedicated runtime database role with only the necessary table permissions; use a separate migration role for DDL.
3. Run the image once with `migrate` and the direct migration connection.
4. Start replicas with `serve`, `APP_ENV=production`, `HTTP_HOST=0.0.0.0`, and exact HTTPS browser origins.
5. Terminate HTTPS at a trusted reverse proxy. Route readiness probes to `/health/ready` and liveness probes to `/health/live`.

The server reads `.env` when present without overriding existing process environment. Production images contain neither `.env` nor source credentials. Never bake secrets into image layers. Keep migration files immutable once applied; add a new forward migration for corrections. Database backups and backward-compatible schema changes are operational responsibilities.

The readiness endpoint checks connectivity, required tables, and applied migration checksums within two seconds. Liveness does not call PostgreSQL. The process handles Ctrl+C and SIGTERM, drains HTTP requests, and closes its database pool.

CI checks formatting, strict Clippy, unit/HTTP tests, actual PostgreSQL integration tests, and the Docker build. It does not publish or deploy to an unconfigured destination. Dependabot tracks Cargo, Docker, and GitHub Actions updates.

## Configuration

| Variable | Default | Constraint / purpose |
| --- | --- | --- |
| `APP_ENV` | `development` | `development`, `test`, `production` |
| `HTTP_HOST` | `127.0.0.1` | Bind IP; use `0.0.0.0` in containers |
| `HTTP_PORT` | `3000` | 1–65535 |
| `DATABASE_URL` | Required | Runtime connection; never logged |
| `DATABASE_URL_UNPOOLED` | Runtime URL | Direct migration connection |
| `DATABASE_MAX_CONNECTIONS` | `10` | 1–100 per process |
| `DATABASE_ACQUIRE_TIMEOUT_SECONDS` | `10` | 1–60 seconds |
| `JWT_SECRET` | Required | At least 32 bytes of random material; no built-in secret |
| `JWT_ISSUER` | `enterprise-rust-backend` | Nonempty expected issuer |
| `JWT_AUDIENCE` | `enterprise-api` | Nonempty expected audience |
| `ACCESS_TOKEN_TTL_SECONDS` | `900` | 60–3600 seconds |
| `REFRESH_TOKEN_TTL_SECONDS` | `604800` | 3600–2592000; longer than access lifetime |
| `PASSWORD_CONCURRENCY` | `4` | 1–32 hashing tasks per process |
| `CORS_ALLOWED_ORIGINS` | Empty | Comma-separated exact origins; HTTPS in production |
| `REQUEST_TIMEOUT_SECONDS` | `15` | 1–120 seconds |
| `REQUEST_BODY_LIMIT_BYTES` | `16384` | 1024–1048576 bytes |
| `AUTH_REQUESTS_PER_MINUTE` | `20` | 1–10000 requests per socket peer per process |
| `RUST_LOG` | `info,sqlx=warn` | Tracing filter; request logs omit headers, bodies, and query strings |

## Initial administrator

Register the intended account through the normal API, verify the person's identity through your operational process, then use a trusted database session:

```sql
BEGIN;
UPDATE users SET role = 'admin'
WHERE email = 'verified-admin@example.com' AND is_active = TRUE
RETURNING id, email, role;
-- Confirm that exactly the intended row was returned before committing.
COMMIT;
```

There is no automatic first-user promotion, seeded admin password, or role-editing HTTP endpoint. Disable accounts through the same controlled administrative process with `UPDATE users SET is_active = FALSE WHERE id = ...`; existing access tokens are rejected on subsequent requests.

## Session maintenance and client behavior

Clients must serialize refresh calls for a session and store the newly returned refresh token atomically. Replaying an old token, including a concurrent duplicate, revokes the family. A refresh response lost after commit requires login again. Token responses must be kept out of analytics and application logs. For a browser client, choose a secure token storage strategy in the frontend/BFF design; this backend uses explicit bearer tokens and does not set cookies.

Schedule periodic cleanup through a trusted database job, preferably in small batches for a large installation:

```sql
DELETE FROM auth_sessions WHERE expires_at < NOW();
```

Foreign keys cascade cleanup to refresh-token history. Retain consumed refresh hashes until their session expires so replay detection continues to work. Rotating `JWT_SECRET` invalidates all current access tokens; existing refresh sessions can obtain tokens signed with the new secret. Revoke sessions too if responding to a compromise.

## Deliberate scope and limits

- The in-process auth limiter is bounded to 10,000 client entries and is per replica. It uses only the socket peer and ignores forwarded headers. Behind a proxy, requests share the proxy's limit. Configure ingress rate limiting and trusted client-IP handling for distributed deployments.
- There is no email verification, password reset, MFA, account recovery, organization tenancy, audit-event store, or granular permission editor yet. These require explicit business workflows.
- Duplicate registration returns 409 and therefore reveals that an address is registered. Login uses the same 401 for nonexistent users, wrong passwords, and disabled users.
- The API has no email-change or password-change route. Profile edits change only display names.
- The API serves HTTP; deployment infrastructure must provide HTTPS, request/header limits, ingress protections, log retention, monitoring, backups, and deployment rollback.
- This is a tested backend foundation, not a claim of compliance certification or a substitute for deployment-specific security review.
