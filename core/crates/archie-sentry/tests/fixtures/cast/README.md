Self-signed RSA-2048 certificates and PKCS#8 keys for the in-process CAST v2
receiver in `tests/servers/cast.rs`. Test-only material, generated with
`openssl req -x509 -newkey rsa:2048 -nodes -days 36500`; never used outside tests.
`other-*` is a second identity used to prove that a wrong pin is refused.
