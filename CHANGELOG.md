# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-10-03

### Changed

**Web console**

- The vulnerability feed is configured on its own admin page. A
  repository's Packages tab shows the dataset, import date, and mode.
  Deleting a repository still does not remove the index.
- Removing the vulnerability feed deletes that instance-wide index and
  the saved sync settings, then re-runs stored assays.

## [0.2.0] - 2026-10-03

### Added

**Security**

- Assays can consult a local OSV index (`OSV_FEED_PATH`, format
  `ferrobox-osv-index` v1, plain JSON or gzip) instead of calling
  `api.osv.dev`. When the variable is unset, the live query is unchanged.
- An administrator can import that index with `POST /api/security/osv-feed`
  and an `X-FerroBox-Sha256` header. The instance stores the dataset
  version and checksum; a bad checksum or a bad document does not replace
  the previous index, and the imported index is loaded again after a
  restart. `GET` on the same path reports the imported feed. An imported
  index takes precedence over `OSV_FEED_PATH`.
- The package list shows a vulnerability-feed card (dataset, import
  date, and mode) and a `[0 Crit | 2 High]` column on assayed versions.
  A version that has not been assayed does not show a zero. The column
  opens that assay.
- Importing a vulnerability index with a new checksum re-runs the
  stored assays in the background. Each one is marked running, then
  replaced with the result of the new index. Repeating the same
  checksum does not re-run them.
- A connected instance can pull a Cosign-signed OCI index
  (`OSV_SYNC_REF`, for example `ghcr.io/ferrobox/osv-db:2026-10-03`)
  on a schedule. The signed layer is the same `ferrobox-osv-index` file
  an air-gapped instance imports; there is no `docker save` path. A
  missing or invalid signature does not replace the active index.
  After a signed pull the feed card mode is `sync`; an upload
  stays `file`.
- The OSV database workflow keeps going when GitHub's package
  visibility API returns 404 for a package that a public repository
  already published as public.
- A workflow builds one gzipped `ferrobox-osv-index` from the OSV
  exports for npm, PyPI, crates.io, Maven, NuGet, and Go, pushes it to
  `ghcr.io/<owner>/osv-db:YYYY-MM-DD`, and signs that artifact with the
  Cosign key stored as `OSV_COSIGN_KEY`. The public key is
  `osv-sync.pub.pem` in this repository. The checksum file is a second
  layer of that artifact, so an air-gapped install can `oras pull` it
  and run `sha256sum -c`. The workflow does not create a GitHub Release,
  and a run deletes any `osv-db-*` release left on this repository so
  the product release stays Latest. An OSV range that records
  `last_affected` stays an inclusive upper bound.
- The Packages card can save the OCI reference and the Cosign public key
  and sync now. A rejected signature does not replace the active index.
  Settings saved from the card win over `OSV_SYNC_REF` after the first
  save. Those variables still seed the row on first boot when the table
  is empty.

### Changed

**Security**

- A repository policy that checks known vulnerabilities now fails closed.
  If that clause is on and the vulnerability feed is missing, or the
  assay for that version is not ready, the pull and the promote are
  denied. A missing feed is reported as `feed missing`. With the clause
  off, a missing assay still does not block.

## [0.1.1] - 2026-09-30

### Changed

**Web console**

- Replaced the placeholder app icon and favicon with the official FerroBox
  logo, and colored the "FerroBox" wordmark with the official brand colors,
  in the sidebar and on the login screen.

### Fixed

**Documentation**

- Synced the README's Docker Compose quickstart with `infra/dev-up.sh`,
  which generates `.env` and every secret automatically on first run; the
  previous instructions asked readers to edit secrets by hand.

## [0.1.0] - 2026-09-27

### Added

**Project**

- Initial public release of FerroBox, a self-hosted artifact registry for
  Cargo, npm, PyPI, OCI images, Helm charts, Conan, Maven, NuGet, Go modules,
  and generic files.
- Licensed under the Functional Source License, Version 1.1, Apache 2.0
  Future License (FSL-1.1-Apache-2.0).

**Repository management**

- `Forge` (hosted), `Mirror` (pull-through cache of an upstream registry),
  and `Alloy` (virtual repository aggregating several `Forge`/`Mirror`
  repositories under one URL) repository kinds.
- On-demand and scheduled prefetch of cached `Mirror` packages from their
  upstream.
- Promote a published package version between `Forge` repositories.
- Export and import a repository as a portable `.tar.gz` bundle, or push a
  bundle straight to another FerroBox instance.
- Scheduled push or pull replication of a `Forge` repository's catalog
  to or from another FerroBox instance.

**Registries**

- Cargo: sparse index, publish, yank, search, and `Mirror` pull-through
  cache.
- npm: publish/install (`Forge`) and `Mirror` pull-through cache honoring
  upstream dist-tags.
- PyPI: `twine upload` / `pip install` (`Forge`), `Mirror` pull-through
  cache, and a PEP 691 JSON simple index.
- OCI: `docker push`/`pull` (`Forge`), a Docker Hub pull-through cache
  (`Mirror`), Docker registry token auth, and Cosign signature detection.
- Helm: OCI chart registry with nested image names.
- Conan: `Forge` upload/install, `Mirror` pull-through cache, and prefetch,
  all on the v2 API with revisions.
- Maven: SNAPSHOT and release support.
- NuGet: V3 protocol for `Forge`, `Mirror`, and `Alloy`.
- Go: GOPROXY protocol for `Forge`, `Mirror`, and `Alloy`.
- Generic: unstructured binary artifacts.

**Security and compliance**

- Assays: automatic SBOM extraction (CycloneDX), plus OSV vulnerability and
  declared-license scanning, for every published package version.
- Admission policies that block downloads or promotions failing CVE,
  license, or Cosign signature checks, with warn/deny audit events.
- Cosign simple-signature verification against a PEM public key.
- Per-repository WORM lock to make published artifacts immutable.

**Access control**

- Users, groups, and roles, with per-repository read/write access grants.
- API tokens with read/write scopes, optional expiry, and robot accounts.
- OpenID Connect single sign-on (Keycloak-compatible) with just-in-time
  provisioning and group mapping.
- Audit log of administrative and repository actions.

**Operations**

- Per-repository or instance-wide retention policies (keep-last /
  keep-days) with a dry run before deleting, plus garbage collection.
- Per-repository storage quotas.
- Webhooks for artifact publish and assay completion, with delivery
  history and a manual ping.
- Instance-wide package search.

**Web console and deployment**

- React admin console (English/Spanish) for repositories, artifacts,
  assays, users, groups, and settings.
- Single container image (Rust backend and static frontend) and a
  `docker compose` setup with PostgreSQL and Garage (S3-compatible
  storage).
- A Helm chart for Kubernetes, with optional bundled PostgreSQL and
  Garage.

[Unreleased]: https://github.com/ferrobox/ferrobox/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/ferrobox/ferrobox/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/ferrobox/ferrobox/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/ferrobox/ferrobox/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/ferrobox/ferrobox/releases/tag/v0.1.0
