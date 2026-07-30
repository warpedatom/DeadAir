# Security Policy

## Scope

DeadAir is an offline solver: it reads a scored-graph JSON on stdin/file and
writes ranked paths to stdout. It connects to nothing and attacks nothing. The
security surface is **parsing untrusted JSON input**.

## Reporting a vulnerability

Report privately rather than in a public issue:

- Open a GitHub **security advisory** (Security tab -> Report a vulnerability), or
- Email the maintainer (see the GitHub profile).

Include the version, a description, and a minimal reproducer. We aim to
acknowledge within a few days and fix confirmed issues promptly.

## Supported versions

The latest published crate version is supported.
