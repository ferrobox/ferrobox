# FerroBox

FerroBox is a self-hosted artifact registry for Cargo, npm, PyPI, OCI images,
Helm charts, Conan, Maven, NuGet, Go modules, and generic files, backed by
PostgreSQL and any S3-compatible object store.

## Quickstart: Docker Compose

Requires Docker with the Compose plugin.

```bash
git clone https://github.com/ferrobox/ferrobox.git
cd ferrobox/infra
./dev-up.sh
```

The first run creates `infra/.env` from `infra/.env.example` and fills in
PostgreSQL, Garage, and S3 secrets with freshly generated random values; an
existing `.env` is left untouched. Open http://localhost:3000 and sign in
with `admin` / `admin` (the default `ADMIN_USERNAME` / `ADMIN_PASSWORD`;
change them in `.env` before first boot if you want different ones).

To stop the stack: `docker compose -f infra/docker-compose.yml down` (add
`-v` to also delete the PostgreSQL and Garage volumes).

## Quickstart: Helm

Requires Kubernetes and Helm 3.8 or later (needed to pull a chart from an OCI
registry).

```bash
helm install ferrobox oci://ghcr.io/ferrobox/charts/ferrobox --version 0.2.1 \
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
docker pull ghcr.io/ferrobox/ferrobox:0.2.1
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
- **Assays**: automatic SBOM extraction (CycloneDX) plus vulnerability and
  license scanning for every published package version. Vulnerability
  results come from the instance feed when one is loaded, and from
  `api.osv.dev` otherwise.
- **Vulnerability feed**: one index for the instance, covering npm, PyPI,
  crates.io, Maven, NuGet, and Go. Import the file, or sync the signed
  artifact `ghcr.io/ferrobox/osv-db:YYYY-MM-DD` from the Vulnerability feed page.
- **Admission policies**: block downloads or promotions that fail CVE,
  license, or Cosign signature checks. A CVE check fails closed when the
  feed is missing or that version's assay is not ready.
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

## Vulnerability feed

Until an administrator loads an index, assays keep calling `api.osv.dev`.
The index is one `ferrobox-osv-index` file for the whole instance. It is not
a repository, and it is not a GitHub Release of this project.

On the **Vulnerability feed** page, an administrator syncs
`ghcr.io/ferrobox/osv-db:YYYY-MM-DD`. Paste [`osv-sync.pub.pem`](osv-sync.pub.pem)
as the Cosign public key. A rejected signature does not replace the index
already in use. A repository's **Packages** tab shows the dataset, import
date, and mode. An air-gapped install downloads the same bytes with
`oras pull` on that reference, checks `ferrobox-osv-index.json.gz.sha256`,
and imports the file with `POST /api/security/osv-feed` and an
`X-FerroBox-Sha256` header.

OCI images, Helm charts, Conan, and generic files are not part of that index.
The **Download** button in the console does not apply admission policies; a
client pull and a promote do.

## Upgrading to 0.2.1

Client protocols are unchanged. The feed form is on the **Vulnerability
feed** page. A repository's **Packages** tab shows the dataset, import
date, and mode.

## Upgrading to 0.2.0

Client protocols are unchanged. If a repository policy's known-vulnerability
rule is already on, pulls and promotes of packages in that index are denied
until a feed is loaded and that version's assay is ready. Leave the rule off
to keep the previous behavior.

## License

FerroBox is licensed under the [Functional Source License, Version 1.1,
Apache 2.0 Future License](LICENSE.md) (FSL-1.1-Apache-2.0). Each version
automatically becomes available under the Apache License 2.0 two years after
its release.
