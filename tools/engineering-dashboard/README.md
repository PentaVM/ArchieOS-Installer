# Private engineering dashboard

Separate Next.js development tooling; not an Archie daemon or ISO component.
All fourteen views and `/api/status` require a durable, opaque session. The ingest
credential cannot log in, and a viewer session cannot ingest events or run commands.

## Local operation

Requires Node 24 and npm. Run `npm ci`, `npm run check`, `npm test`, then
`npm run build`. Set the server-only variables documented in `.env.example` in
the local process environment (or an ignored `.env.local`). Never use a
`NEXT_PUBLIC_` prefix, commit credentials, or put credentials in command arguments.
Use independent random session and ingest secrets of at least 32 characters.
Set the configured origin exactly, without a trailing slash. `npm start` listens
only on `127.0.0.1:3077`. PostgreSQL is mandatory in every environment.

`node tests/browser.mjs` starts a separate production server with generated test
credentials and an explicit `ARCHIE_DASHBOARD_BROWSER_DATABASE_URL`, checks all views with isolated Chrome
contexts, and retains local desktop/mobile screenshots. It requires Chrome and
the `playwright` package; `ARCHIE_PLAYWRIGHT_MODULE` can point to an existing
installation. Test fixtures are not published as project evidence.

## Outbound publisher

From this directory, `npm run publish-status` inventories tracked, hidden,
untracked and ignored entries using Git. Ignored trees are aggregated; file
contents are never read or transmitted. Supply a status JSON filename to publish
a specific event. Set `ARCHIE_DASHBOARD_INGEST_URL` to the protected HTTPS
`/api/ingest` endpoint and the independent ingest secret in the local environment.
Without an endpoint, sanitized immutable events remain in the local outbox.
`npm run publish-status -- --retry` retries pending payloads unchanged. A matching
server event ID is idempotent; a conflicting payload is rejected. Receipts hold
payload hashes. Events older than seven days require explicit review, not silent
retimestamping. Keep one publisher per outbox.

`node scripts/run-check.ts TEST module executable arguments...` records start,
30-second progress and completion, duration and exit status. It streams sanitized
local logs, hashes them and publishes bounded status summaries. It never invokes
a shell. Set `ARCHIE_DASHBOARD_OUTBOX` consistently for inventory and wrappers
when invoking them from different working directories. Remote outages retain
events and do not change the checked command's exit code.

## Deployment and limits

The deployed adapter uses `ARCHIE_DASHBOARD_DATABASE_URL` (PostgreSQL) for events,
sessions and rate limits. On Vercel, missing database configuration fails closed;
there is no embedded database fallback. Schema creation requires a database
role with DDL permission. PostgreSQL 18.4 integration and production Chrome checks
have passed in an isolated local cluster. Remote Vercel database configuration and
deployment remain pending. No deployment URL is claimed.
Use HTTPS, Node 24 and the exact deployment origin. Native Vercel password
protection can be added if the account supports it; application auth remains.

The current release presents status timelines and repository metadata. Specialized
service/resource/IPC topology, repository-content
excerpts, retention management and automatic refresh remain pending. An empty
section is explicitly unverified; it is never evidence of a passing subsystem.
`npm run publish-status -- --release` publishes `docs/RELEASE_STATUS.json` as the
module/milestone matrix, preserving its recorded Git baseline. PASS cells require
a command and can be expanded to inspect that evidence. Maturity applies only to
the stated scope, not to the entire subsystem's production readiness.
The global login budget is ten attempts per ten minutes, deliberately conservative
for a private single-viewer tool; saturation temporarily blocks legitimate login.

Security checks cover expiry, revocation after restart, CSRF origin rejection,
anonymous API isolation, payload bounds, escaping, split-chunk secret redaction,
outbox retry and integrity. Database-backed authentication and reopening tests
now use PostgreSQL exclusively; without an explicit test connection they skip.
The optional PostgreSQL integration test requires an explicit
`ARCHIE_DASHBOARD_TEST_DATABASE_URL` with permission to create an isolated test
schema. It tests real concurrent idempotency, rate limits, reopened storage,
session expiry/revocation and corruption detection, then removes only that randomly
named schema. Without the URL the test is explicitly skipped, never a PASS.

The Core, state and types tests that need a database are `#[ignore]`, so a clone with
no PostgreSQL reports them as ignored rather than failing 67 tests that are not defects.
`--core` therefore runs with `--include-ignored`: this is the run that has a cluster, so
it is the run that must not skip them. `#[ignore]` and not an early return, because an
early return reports a PASS and a test that did not run must never look like one that did.

`node tests/with-postgres.mjs` creates and stops a separate local cluster using
`ARCHIE_POSTGRES_BIN`, runs the data/security tests serially, then runs production
Chrome checks. Set `ARCHIE_PLAYWRIGHT_MODULE` when needed. Generated credentials
stay in the child environment; fixture databases and logs are retained locally.
Optional extension fixtures use `ARCHIE_POSTGRES_EXTENSION_ROOT`; see
`distro/database/README.md`. Thirteen tests passed with the reviewed extensions;
the subsequent browser startup failed under host memory pressure. The browser
passed separately against plain PostgreSQL. Run heavy checks sequentially on this
host; a concurrent TypeScript check exhausted memory, then passed with a bounded
heap when run alone.
