# Owner companion

Local WebAuthn/passkey service for the Android and iPhone flow in the native wizard.
Uses PostgreSQL and SimpleWebAuthn; no SQLite, hosted identity service or billing API.
This component is configured separately and is not yet provisioned by the ISO.

Use Node 22+ and `npm ci`. Create a private JSON file containing `connection`, a
PostgreSQL connection URL, then use an owner profile saved by the wizard and the
actual 64-character hexadecimal node identity:

```sh
node setup.mjs /private/database.json /private/wizard-profile.json NODE_ID /private/new-owner
ARCHIE_OWNER_CONFIG=/private/new-owner/owner.json npm start
ARCHIE_OWNER_ACTIVATION_FILE=/private/new-owner/activation archie-wizard
```

Do not put database credentials or the activation code in command arguments, URLs,
source control or logs. Setup is create-only. Restrict the output directory to its
operator on Windows; POSIX permissions alone do not configure Windows ACLs.
Open `http://localhost:7668` on this same computer. The browser displays its standard
cross-device QR flow when supported. Android/iPhone handle fingerprint, face or PIN
verification; the server requires user verification but cannot choose the modality.
The browser's hybrid transport may require Bluetooth and Internet, so cross-device
passkeys are not promised to work fully offline. No biometric data is sent to Archie.

The first credential requires the private activation code within 15 minutes of
service start. Another credential requires a prior owner-approved add-device
operation. Challenges expire after 120 seconds; exact-action approvals expire after
60 seconds and can be consumed once. Requests are node/session bound. The native
wizard authenticates requests and verifies responses with HMAC; request bodies,
paths, nonces and timestamps are covered. Sensor consent remains required locally.

An `activate-node` receipt is evidence only: this service does not join a fabric,
change Core permissions or authorize disk erasure. The implemented consumer is the
wizard's camera/microphone test. Credential revocation administration and physical
phone interoperability remain to be completed.

Tests run via `node tests/with-postgres.mjs --owner` from the engineering-dashboard
directory. Set `ARCHIE_POSTGRES_BIN` to PostgreSQL's bin directory and
`ARCHIE_PLAYWRIGHT_MODULE` to an installed Playwright module. The integration test
uses real PostgreSQL and browser-verified signatures from a virtual CTAP2
authenticator. Without the fixture, that test is explicitly skipped.
