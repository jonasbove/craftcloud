## What and why

<!-- What does this change, and why? Link the issue if there is one. The title is the commit
message: plain English, what changed for the person or developer. -->

## How it was tested

<!-- Tests added, and how you checked the behaviour: the desktop client, the web frontend, the
`craftcloud` command, the live tests against a throwaway server (CONTRIBUTING.md), screenshots. -->

**Not verified:** <!-- what you couldn't try (a platform, a real server, a Craft app) -->

## Checklist

- [ ] `cargo xtask ci` passes (fmt, clippy, tests, wasm), and `cargo deny check` if dependencies
      changed.
- [ ] A change to what the sync uploads, downloads, ignores or deletes has a test, and an entry in
      `docs/RELEASE-NOTES.md` when older clients would act differently on the same account.
- [ ] Nothing the OxiCloud server lacks was patched around in its image; the gap is in
      `docs/UPSTREAM.md`.
- [ ] Neutral data only: "John Doe" (`john`), `cloud.example.org`; no real names, hosts, tokens or
      private files (AGENTS.md).
- [ ] Our own work: no code, names or wording copied from other projects, and no ArtCraft name or
      mark in CraftCloud's branding.
