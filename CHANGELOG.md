# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/ferrobox/ferrobox/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ferrobox/ferrobox/releases/tag/v0.1.0
