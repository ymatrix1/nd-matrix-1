# Security policy

## Reporting a vulnerability

Use GitHub's private reporting: [Report a vulnerability](https://github.com/ymatrix1/nd-matrix-1/security/advisories/new). Do not open a public issue for a security problem.

Include the version or commit, your OS, and steps to reproduce. If a crafted file triggers it (for example a malformed `.ns4p`), describe it or share a minimal synthetic sample. Never send firmware, `.ns4b` banks, samples or vendor material ([docs/LEGAL.md](docs/LEGAL.md)).

## Supported versions

Only the latest release and `main` get fixes.

## Scope

Memory safety or crashes when parsing `CBIN` files, plugin or standalone hangs on untrusted input, and supply-chain issues in the build. Rights-holder concerns go through the [legal contact](docs/LEGAL.md), not here.
