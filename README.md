# FerroBox

FerroBox is a self-hosted artifact registry for Cargo, npm, PyPI, OCI images,
Helm charts, Conan, Maven, NuGet, Go modules, and generic files, backed by
PostgreSQL and any S3-compatible object store.

## Quickstart: Docker Compose

Requires Docker with the Compose plugin.

```bash
git clone https://github.com/ferrobox/ferrobox.git
cd ferrobox/infra
cp .env.example .env
# Edit .env: set POSTGRES_PASSWORD, GARAGE_RPC_SECRET, GARAGE_ADMIN_TOKEN,
# GARAGE_METRICS_TOKEN, S3_SECRET_ACCESS_KEY, and ADMIN_PASSWORD.
docker compose up --build
```

Open http://localhost:3000 and sign in with the `ADMIN_USERNAME` /
`ADMIN_PASSWORD` you set in `.env`.

## Quickstart: Helm

Requires Kubernetes and Helm 3.8 or later (needed to pull a chart from an OCI
registry).

```bash
helm install ferrobox oci://ghcr.io/ferrobox/charts/ferrobox --version 0.1.0 \
  --namespace ferrobox --create-namespace
```

The chart bundles a single-node PostgreSQL and a single-node Garage
(S3-compatible storage) by default, and generates every secret (admin
password, database password, Garage tokens) on first install if you don't
set one yourself; point `postgresql.externalUrl` or `s3.external*` at your
own if you'd rather bring your own. See `infra/chart/values.yaml` for every
option, and run `helm status ferrobox -n ferrobox` after install for the
admin credentials and access URL.

## Container image

```bash
docker pull ghcr.io/ferrobox/ferrobox:0.1.0
```

The same image is used by both quickstarts above. It expects a `DATABASE_URL`
and a set of `S3_*` variables pointing at PostgreSQL and an S3-compatible
bucket -- see `infra/.env.example` for the full list.

## Features

- **Ten package ecosystems** behind one binary: Cargo, npm, PyPI, OCI images,
  Helm charts, Conan, Maven, NuGet, Go modules, and generic artifacts.
- **Three repository kinds**: `Forge` (hosted storage), `Mirror` (pull-through
  cache of an upstream registry), and `Alloy` (a virtual repository that
  aggregates several `Forge`/`Mirror` repositories under one URL).
- **RBAC**: users, groups, per-repository read/write access, and scoped API
  tokens, including robot accounts with optional expiry.
- **SSO**: OpenID Connect (Keycloak-compatible) with just-in-time provisioning
  and group mapping.
- **Assays**: automatic SBOM extraction (CycloneDX) plus OSV vulnerability and
  license scanning for every published package version.
- **Admission policies**: block downloads or promotions that fail CVE,
  license, or Cosign signature checks.
- **Retention and garbage collection**: per-repository or instance-wide
  policies, with a dry run before anything is deleted.
- **Quotas**: per-repository storage limits.
- **WORM lock**: make a repository's published artifacts immutable once you're
  ready to freeze it.
- **Replication**: scheduled push or pull sync of a `Forge` repository's
  catalog to or from another FerroBox instance.
- **Webhooks**: notify external systems on artifact and assay events, with
  delivery history and a manual ping.
- **Audit log** of administrative actions, and full-text package **search**
  across repositories.

## License

FerroBox is licensed under the [Functional Source License, Version 1.1,
Apache 2.0 Future License](LICENSE.md) (FSL-1.1-Apache-2.0). Each version
automatically becomes available under the Apache License 2.0 two years after
its release.
